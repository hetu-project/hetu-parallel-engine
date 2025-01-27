use std::io;
use std::net::TcpStream;
use std::path::PathBuf;
use cometbft::abci::{request, response, Request, Response};
use tempfile::TempDir;

use crate::cometbft::CometNode;
use crate::db::RocksDB;
use crate::chain::ChainStore;

fn setup_test_db() -> (ChainStore, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("ChainStore.db");
    let db = RocksDB::open(&db_path).unwrap();
    let store = ChainStore::new(Box::new(db)).unwrap();
    (store, temp_dir)
}



