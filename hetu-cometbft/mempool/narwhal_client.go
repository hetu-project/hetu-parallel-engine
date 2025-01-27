package mempool

import (
	"bufio"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/cometbft/cometbft/libs/log"
	"github.com/cometbft/cometbft/libs/service"
	narwhalpb "github.com/cometbft/cometbft/proto/narwhal"
	"google.golang.org/protobuf/proto"
)

const (
	defaultSubmitPort  = 4003
	defaultReceivePort = 20001
	maxFrameLength     = 16777215 // 16MB max frame length
)

// DefaultNarwhalClient implements the NarwhalClient interface using raw TCP
type DefaultNarwhalClient struct {
	service.BaseService

	addr string

	// TCP connection for submitting transactions
	submitConn     net.Conn
	submitReader   *bufio.Reader
	submitWriter   *bufio.Writer
	submitConnLock sync.Mutex

	// TCP connection for receiving transactions
	receiveConn     net.Conn
	receiveReader   *bufio.Reader
	receiveWriter   *bufio.Writer
	receiveConnLock sync.Mutex

	// Channel for received transactions
	txChan chan *BatchTransactions

	logger log.Logger
}

// BatchTransactions represents a batch of transactions with round number
type BatchTransactions struct {
	Transactions [][]byte
	Round        uint64
}

func NewDefaultNarwhalClient(addr string, logger log.Logger) *DefaultNarwhalClient {
	client := &DefaultNarwhalClient{
		addr:   addr,
		logger: logger,
		txChan: make(chan *BatchTransactions, 1000), // Buffer for incoming transaction batches
	}
	client.BaseService = *service.NewBaseService(logger, "NarwhalClient", client)
	return client
}

func (c *DefaultNarwhalClient) OnStart() error {
	var err error

	// Setup submit connection
	submitAddr := fmt.Sprintf("%s:%d", c.addr, defaultSubmitPort)
	c.submitConn, err = net.Dial("tcp", submitAddr)
	if err != nil {
		return fmt.Errorf("failed to connect to submit port: %w", err)
	}
	c.submitReader = bufio.NewReader(c.submitConn)
	c.submitWriter = bufio.NewWriter(c.submitConn)

	// Setup receive connection
	receiveAddr := fmt.Sprintf("%s:%d", c.addr, defaultReceivePort)
	c.receiveConn, err = net.Dial("tcp", receiveAddr)
	if err != nil {
		c.submitConn.Close()
		return fmt.Errorf("failed to connect to receive port: %w", err)
	}
	c.receiveReader = bufio.NewReader(c.receiveConn)
	c.receiveWriter = bufio.NewWriter(c.receiveConn)

	// Start reading transactions in background
	go c.readTransactions()

	return nil
}

func (c *DefaultNarwhalClient) OnStop() {
	if c.submitConn != nil {
		c.submitConn.Close()
	}
	if c.receiveConn != nil {
		c.receiveConn.Close()
	}
}

// readTransactions continuously reads transactions from the receive connection
func (c *DefaultNarwhalClient) readTransactions() {
	for {
		if !c.IsRunning() {
			return
		}

		data, err := c.readFrame(c.receiveReader)
		if err != nil {
			if err != io.EOF && c.IsRunning() {
				c.logger.Error("Error reading transaction", "err", err)
				// Try to reconnect
				if err := c.reconnectReceive(); err != nil {
					c.logger.Error("Failed to reconnect receive", "err", err)
					time.Sleep(time.Second)
					continue
				}
			}
			continue
		}

		// Unmarshal protobuf message
		var batchProto narwhalpb.BatchTransactions
		if err := proto.Unmarshal(data, &batchProto); err != nil {
			c.logger.Error("Failed to unmarshal batch transactions", "err", err)
			continue
		}

		batch := &BatchTransactions{
			Transactions: batchProto.Transactions,
			Round:        batchProto.Round,
		}

		select {
		case c.txChan <- batch:
			c.logger.Debug("Received batch", "txs", len(batch.Transactions), "round", batch.Round)
		default:
			c.logger.Error("Transaction channel full, dropping batch", "txs", len(batch.Transactions))
		}
	}
}

// reconnectSubmit attempts to reconnect the submit connection
func (c *DefaultNarwhalClient) reconnectSubmit() error {
	c.submitConnLock.Lock()
	defer c.submitConnLock.Unlock()

	if c.submitConn != nil {
		c.submitConn.Close()
	}

	submitAddr := fmt.Sprintf("%s:%d", c.addr, defaultSubmitPort)
	conn, err := net.Dial("tcp", submitAddr)
	if err != nil {
		return fmt.Errorf("failed to reconnect submit: %w", err)
	}

	c.submitConn = conn
	c.submitReader = bufio.NewReader(conn)
	c.submitWriter = bufio.NewWriter(conn)
	return nil
}

// reconnectReceive attempts to reconnect the receive connection
func (c *DefaultNarwhalClient) reconnectReceive() error {
	c.receiveConnLock.Lock()
	defer c.receiveConnLock.Unlock()

	if c.receiveConn != nil {
		c.receiveConn.Close()
	}

	receiveAddr := fmt.Sprintf("%s:%d", c.addr, defaultReceivePort)
	conn, err := net.Dial("tcp", receiveAddr)
	if err != nil {
		return fmt.Errorf("failed to reconnect receive: %w", err)
	}

	c.receiveConn = conn
	c.receiveReader = bufio.NewReader(conn)
	c.receiveWriter = bufio.NewWriter(conn)
	return nil
}

// writeFrame writes a length-prefixed frame to the TCP connection
func (c *DefaultNarwhalClient) writeFrame(writer *bufio.Writer, data []byte) error {
	length := len(data)
	if length > maxFrameLength {
		return fmt.Errorf("frame too large: %d > %d", length, maxFrameLength)
	}

	// Write length prefix (4 bytes, big-endian)
	lenBuf := make([]byte, 4)
	binary.BigEndian.PutUint32(lenBuf, uint32(len(data)))
	if _, err := writer.Write(lenBuf); err != nil {
		return fmt.Errorf("failed to write length: %w", err)
	}

	// Write payload
	if _, err := writer.Write(data); err != nil {
		return fmt.Errorf("failed to write payload: %w", err)
	}

	return writer.Flush()
}

// readFrame reads a length-prefixed frame from the TCP connection
func (c *DefaultNarwhalClient) readFrame(reader *bufio.Reader) ([]byte, error) {
	// Read length prefix (4 bytes, big-endian)
	lenBuf := make([]byte, 4)
	if _, err := io.ReadFull(reader, lenBuf); err != nil {
		return nil, fmt.Errorf("failed to read length: %w", err)
	}

	length := binary.BigEndian.Uint32(lenBuf)
	if length > maxFrameLength {
		return nil, fmt.Errorf("frame too large: %d bytes (max %d)", length, maxFrameLength)
	}

	// Read payload
	data := make([]byte, length)
	_, err := io.ReadFull(reader, data)
	if err != nil {
		return nil, err
	}

	return data, nil
}

// SubmitTransaction submits a transaction to Narwhal via TCP
func (c *DefaultNarwhalClient) SubmitTransaction(tx []byte) error {
	c.submitConnLock.Lock()
	defer c.submitConnLock.Unlock()

	err := c.writeFrame(c.submitWriter, tx)
	if err != nil {
		// Try to reconnect and retry once
		if err := c.reconnectSubmit(); err != nil {
			return fmt.Errorf("failed to reconnect submit: %w", err)
		}

		// Retry sending
		err = c.writeFrame(c.submitWriter, tx)
		if err != nil {
			return fmt.Errorf("failed to submit transaction after reconnect: %w", err)
		}
	}

	return nil
}

// GetNextBatch returns the next batch of transactions from the channel
func (c *DefaultNarwhalClient) GetNextBatch() (*BatchTransactions, error) {
	if !c.IsRunning() {
		return nil, fmt.Errorf("client not running")
	}

	select {
	case batch := <-c.txChan:
		return batch, nil

		//case <-time.After(time.Second):
		//	return nil, fmt.Errorf("timeout waiting for batch")
	}
}
