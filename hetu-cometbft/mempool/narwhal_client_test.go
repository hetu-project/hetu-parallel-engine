package mempool

import (
	"github.com/cometbft/cometbft/libs/log"
	"github.com/stretchr/testify/require"
	"net"
	"testing"
)

func setupMockNarwhalServer(t *testing.T) (func(), net.Listener, net.Listener) {
	submitAddr := &net.TCPAddr{IP: net.ParseIP("127.0.0.1"), Port: defaultSubmitPort}
	receiveAddr := &net.TCPAddr{IP: net.ParseIP("127.0.0.1"), Port: defaultReceivePort}

	submitListener, err := net.ListenTCP("tcp", submitAddr)
	require.NoError(t, err)

	receiveListener, err := net.ListenTCP("tcp", receiveAddr)
	require.NoError(t, err)

	cleanup := func() {
		submitListener.Close()
		receiveListener.Close()
	}

	return cleanup, submitListener, receiveListener
}

func TestNarwhalClientBasic(t *testing.T) {
	// Create client
	client := NewDefaultNarwhalClient("127.0.0.1", log.NewNopLogger())
	err := client.Start()
	require.NoError(t, err)
	defer client.Stop()

	// Test SubmitTransaction and receiving it back
	testTx := []byte("test tx2222")
	err = client.SubmitTransaction(testTx)
	require.NoError(t, err)

	// Test receiving the batch
	batch, err := client.GetNextBatch()
	require.NoError(t, err)
	require.Len(t, batch.Transactions, 1)
	require.Equal(t, testTx, batch.Transactions[0])
}
