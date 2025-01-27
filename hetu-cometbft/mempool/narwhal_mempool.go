package mempool

import (
	"fmt"
	"sync"
	"time"

	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/libs/log"
	"github.com/cometbft/cometbft/libs/service"
	"github.com/cometbft/cometbft/types"
)

// NarwhalMempool is a mempool implementation that proxies transactions to Narwhal
type NarwhalMempool struct {
	service.BaseService

	mtx sync.RWMutex

	// Narwhal client connection
	client NarwhalClient

	// Original CometBFT mempool for block proposal
	proposalMempool *CListMempool

	// Queue of batches from Narwhal
	batchQueue   []*BatchTransactions
	maxQueueSize int

	// Local storage of transactions by hash
	txsByHash map[string]*mempoolTx

	// Reactor for broadcasting transactions
	reactor *NarwhalMempoolReactor

	txMapMtx sync.RWMutex // Separate lock for txsByHash operations

	logger log.Logger
}

// NarwhalClient defines the interface for interacting with Narwhal
type NarwhalClient interface {
	service.Service
	// SubmitTransaction submits a transaction to Narwhal
	SubmitTransaction(tx []byte) error
	// GetNextBatch gets the next batch of ordered transactions from Narwhal
	GetNextBatch() (*BatchTransactions, error)
}

// NewNarwhalMempool returns a new NarwhalMempool
func NewNarwhalMempool(client NarwhalClient, proposalMempool *CListMempool) *NarwhalMempool {
	mp := &NarwhalMempool{
		client:          client,
		proposalMempool: proposalMempool,
		txsByHash:       make(map[string]*mempoolTx),
		txMapMtx:        sync.RWMutex{},
		batchQueue:      make([]*BatchTransactions, 0),
		maxQueueSize:    100, // Configurable size
		logger:          log.NewNopLogger(),
	}
	mp.BaseService = *service.NewBaseService(mp.logger, "NarwhalMempool", mp)
	return mp
}

// SetReactor sets the reactor for broadcasting transactions
func (mem *NarwhalMempool) SetReactor(reactor *NarwhalMempoolReactor) {
	mem.mtx.Lock()
	defer mem.mtx.Unlock()
	mem.reactor = reactor
}

// ProposalMempool returns the underlying proposal mempool
func (mem *NarwhalMempool) ProposalMempool() *CListMempool {
	mem.mtx.RLock()
	defer mem.mtx.RUnlock()
	return mem.proposalMempool
}

// OnStart implements Service
func (mem *NarwhalMempool) OnStart() error {
	if err := mem.client.Start(); err != nil {
		return err
	}
	go mem.pollNarwhalTxs()
	return nil
}

// OnStop implements Service
func (mem *NarwhalMempool) OnStop() {
	mem.client.Stop()
}

// pollNarwhalTxs continuously polls transactions from Narwhal and forwards them to the proposal mempool
func (mem *NarwhalMempool) pollNarwhalTxs() {
	for {
		if !mem.IsRunning() {
			return
		}

		batch, err := mem.client.GetNextBatch()
		if err != nil {
			mem.logger.Error("failed to get batch from narwhal", "err", err)
			time.Sleep(100 * time.Millisecond)
			continue
		}

		if len(batch.Transactions) == 0 {
			continue
		}

		// Pre-check if we have all transactions
		haveAllTxs := true
		mem.txMapMtx.RLock()
		for _, txHash := range batch.Transactions {
			if _, exists := mem.txsByHash[string(txHash)]; !exists {
				haveAllTxs = false
				mem.logger.Error("Missing transaction", "tx_hash", string(txHash))
				break
			}
		}
		mem.txMapMtx.RUnlock()

		if !haveAllTxs {
			continue
		}

		// Add to batch queue
		mem.mtx.Lock()
		if len(mem.batchQueue) >= mem.maxQueueSize {
			mem.logger.Info("Batch queue full, waiting for space",
				"queue_size", len(mem.batchQueue),
				"max_size", mem.maxQueueSize)
			mem.mtx.Unlock()
			time.Sleep(100 * time.Millisecond)
			continue
		}
		mem.batchQueue = append(mem.batchQueue, batch)
		queueSize := len(mem.batchQueue)
		mem.mtx.Unlock()

		// Only notify if we successfully added the batch
		mem.proposalMempool.notifyTxsAvailable()
		mem.logger.Debug("Added batch to queue",
			"num_txs", len(batch.Transactions),
			"queue_size", queueSize,
			"round", batch.Round)
	}
}

// CheckTx implements the Mempool interface
func (mem *NarwhalMempool) CheckTx(tx types.Tx, callback func(*abci.ResponseCheckTx), txInfo TxInfo) error {
	// Create a wrapper callback to store the mempoolTx
	var memTx *mempoolTx
	txHash := fmt.Sprintf("%X", tx.Hash())
	resCbFirstTime := func(res *abci.ResponseCheckTx) {
		if res.Code == abci.CodeTypeOK {
			memTx = &mempoolTx{
				height:    mem.proposalMempool.height.Load(),
				gasWanted: res.GasWanted,
				tx:        tx,
			}

			// Store in local map
			mem.txMapMtx.Lock()
			mem.txsByHash[txHash] = memTx
			mem.txMapMtx.Unlock()
		}
		if callback != nil {
			callback(res)
		}
	}

	// First validate with original mempool
	err := mem.proposalMempool.CheckTx(tx, resCbFirstTime, txInfo)
	if err != nil {
		return err
	}

	// Then forward hash to Narwhal
	err = mem.client.SubmitTransaction([]byte(txHash))
	if err != nil {
		return err
	}

	return nil
}

// RemoveTxByKey implements the Mempool interface
func (mem *NarwhalMempool) RemoveTxByKey(txKey types.TxKey) error {
	return mem.proposalMempool.RemoveTxByKey(txKey)
}

// ReapMaxBytesMaxGas implements the Mempool interface
func (mem *NarwhalMempool) ReapMaxBytesMaxGas(maxBytes, maxGas int64) types.Txs {
	mem.mtx.Lock()
	defer mem.mtx.Unlock()

	// If we have batches in the queue, use the first one
	if len(mem.batchQueue) > 0 {
		batch := mem.batchQueue[0]

		// Build transaction list
		mem.txMapMtx.RLock()
		txs := make(types.Txs, len(batch.Transactions))
		for i, txHash := range batch.Transactions {
			if memTx, exists := mem.txsByHash[string(txHash)]; exists {
				txs[i] = memTx.tx
			} else {
				// If any transaction is missing, skip this batch
				mem.logger.Error("Missing transaction while reaping", "tx_hash", string(txHash))
				mem.txMapMtx.RUnlock()
				return nil
			}
		}
		mem.txMapMtx.RUnlock()

		// Remove the batch from queue after successful processing
		mem.batchQueue = mem.batchQueue[1:]

		return txs
	}

	// Fallback to proposal mempool if no batch is available
	return mem.proposalMempool.ReapMaxBytesMaxGas(maxBytes, maxGas)
}

// ReapMaxTxs implements the Mempool interface
func (mem *NarwhalMempool) ReapMaxTxs(max int) types.Txs {
	return mem.proposalMempool.ReapMaxTxs(max)
}

// Lock implements the Mempool interface
func (mem *NarwhalMempool) Lock() {
	mem.mtx.Lock()
}

// Unlock implements the Mempool interface
func (mem *NarwhalMempool) Unlock() {
	mem.mtx.Unlock()
}

// Update implements the Mempool interface
func (mem *NarwhalMempool) Update(
	height int64,
	txs types.Txs,
	deliverTxResponses []*abci.ExecTxResult,
	preCheck PreCheckFunc,
	postCheck PostCheckFunc,
) error {
	// Remove committed transactions from local storage using separate lock
	mem.txMapMtx.Lock()
	for _, tx := range txs {
		txHash := fmt.Sprintf("%X", tx.Hash())
		delete(mem.txsByHash, txHash)
	}
	mem.txMapMtx.Unlock()

	return mem.proposalMempool.Update(height, txs, deliverTxResponses, preCheck, postCheck)
}

// Flush implements the Mempool interface
func (mem *NarwhalMempool) Flush() {
	mem.txMapMtx.Lock()
	mem.txsByHash = make(map[string]*mempoolTx)
	mem.txMapMtx.Unlock()

	mem.mtx.Lock()
	mem.batchQueue = make([]*BatchTransactions, 0)
	mem.mtx.Unlock()

	mem.proposalMempool.Flush()
}

// FlushAppConn implements the Mempool interface
func (mem *NarwhalMempool) FlushAppConn() error {
	return mem.proposalMempool.FlushAppConn()
}

// Size implements the Mempool interface
func (mem *NarwhalMempool) Size() int {
	return mem.proposalMempool.Size()
}

// SizeBytes implements the Mempool interface
func (mem *NarwhalMempool) SizeBytes() int64 {
	return mem.proposalMempool.SizeBytes()
}

// TxsAvailable implements the Mempool interface
func (mem *NarwhalMempool) TxsAvailable() <-chan struct{} {
	return mem.proposalMempool.TxsAvailable()
}

// EnableTxsAvailable implements the Mempool interface
func (mem *NarwhalMempool) EnableTxsAvailable() {
	mem.proposalMempool.EnableTxsAvailable()
}
