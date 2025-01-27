package mempool

import (
	"github.com/cometbft/cometbft/libs/log"
	"github.com/stretchr/testify/mock"
)

// MockNarwhalClient is a mock implementation of NarwhalClient
type MockNarwhalClient struct {
	mock.Mock
}

func (m *MockNarwhalClient) Start() error {
	args := m.Called()
	return args.Error(0)
}

func (m *MockNarwhalClient) Stop() error {
	args := m.Called()
	return args.Error(0)
}

func (m *MockNarwhalClient) Reset() error {
	args := m.Called()
	return args.Error(0)
}

func (m *MockNarwhalClient) OnReset() error {
	args := m.Called()
	return args.Error(0)
}

func (m *MockNarwhalClient) IsRunning() bool {
	args := m.Called()
	return args.Bool(0)
}

func (m *MockNarwhalClient) Quit() <-chan struct{} {
	args := m.Called()
	return args.Get(0).(<-chan struct{})
}

func (m *MockNarwhalClient) SetLogger(logger log.Logger) {
	m.Called(logger)
}

func (m *MockNarwhalClient) GetNextBatch() (*BatchTransactions, error) {
	args := m.Called()
	return args.Get(0).(*BatchTransactions), args.Error(1)
}

func (m *MockNarwhalClient) OnStart() error {
	args := m.Called()
	return args.Error(0)
}

func (m *MockNarwhalClient) OnStop() {
	m.Called()
}

// MockAppConnMempool implements proxy.AppConnMempool for testing
type MockAppConnMempool struct {
	mock.Mock
}

func (app *MockAppConnMempool) Error() error {
	args := app.Called()
	return args.Error(0)
}
