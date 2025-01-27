package mempool

import (
	abci "github.com/cometbft/cometbft/abci/types"
	"github.com/cometbft/cometbft/p2p"
	protomem "github.com/cometbft/cometbft/proto/tendermint/mempool"
	"github.com/cometbft/cometbft/types"
)

// NarwhalMempoolReactor wraps the original reactor to broadcast transactions when they come from Narwhal
type NarwhalMempoolReactor struct {
	*Reactor
	narwhalMempool *NarwhalMempool
}

// NewNarwhalMempoolReactor creates a new NarwhalMempoolReactor that wraps the original reactor
func NewNarwhalMempoolReactor(narwhalMempool *NarwhalMempool, originalReactor *Reactor) *NarwhalMempoolReactor {
	return &NarwhalMempoolReactor{
		Reactor:        originalReactor,
		narwhalMempool: narwhalMempool,
	}
}

// CheckTx implements Mempool interface for RPC transactions
func (reactor *NarwhalMempoolReactor) CheckTx(tx types.Tx, callback func(*abci.ResponseCheckTx), txInfo TxInfo) error {
	// For RPC transactions, send to Narwhal
	return reactor.narwhalMempool.CheckTx(tx, callback, txInfo)
}

// BroadcastTx broadcasts a transaction to all peers
func (reactor *NarwhalMempoolReactor) BroadcastTx(tx types.Tx) {
	msg := &protomem.Message{
		Sum: &protomem.Message_Txs{
			Txs: &protomem.Txs{Txs: [][]byte{tx}},
		},
	}

	peers := reactor.Switch.Peers().List()
	for _, peer := range peers {
		success := peer.Send(p2p.Envelope{
			ChannelID: MempoolChannel,
			Message:   msg,
		})
		if !success {
			reactor.Logger.Debug("Failed to broadcast tx to peer", "peer", peer.ID(), "tx", tx)
		}
	}
}
