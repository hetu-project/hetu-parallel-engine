use std::path::PathBuf;
use cometbft_node::cometbft::CometNode;
use cometbft_node::chain::ChainStore;
use cometbft_node::db::RocksDB;
use cometbft_node::config::Config;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create block store
    // let db = RocksDB::open(&PathBuf::from("/tmp/cometbft_blocks"))?;
    // let _store = ChainStore::new(Box::new(db))?;

    // Create and initialize node
    let mut node = CometNode::default();
    // Initialize the node
    // let (quit_tx, quit_rx) = mpsc::channel(1);
    node.init().await?;

    // Perform handshake with ABCI app
    // node.handshake().await?;

    // Start consensus engine
    let _ = node.run_consensus().await;

    // Wait for consensus to stop
    // consensus_handle.abort();

    Ok(())
}
