use cometbft::Error;
use rocksdb::{Options, WriteBatch, DB};
use std::path::Path;
use std::sync::Arc;

pub trait Database: Send + Sync {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error>;
    fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Error>;
    fn delete(&self, key: &[u8]) -> Result<(), Error>;
    fn clone_box(&self) -> Box<dyn Database + Send + Sync>;
}

pub struct Transaction {
    batch: WriteBatch,
}

impl Transaction {
    pub fn new() -> Self {
        Self {
            batch: WriteBatch::default(),
        }
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Error> {
        self.batch.put(key, value);
        Ok(())
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<(), Error> {
        self.batch.delete(key);
        Ok(())
    }
}

pub struct RocksDB {
    db: Arc<DB>,
}

impl RocksDB {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)
            .map_err(|e| Error::protocol(format!("failed to open database: {}", e)))?;
        Ok(Self { db: Arc::new(db) })
    }

    pub fn close(&self) {
        // RocksDB will be closed when dropped
    }
}

impl Clone for RocksDB {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}

impl Database for RocksDB {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        self.db
            .get(key)
            .map_err(|e| Error::protocol(format!("failed to get value: {}", e)))
    }

    fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Error> {
        self.db
            .put(key, value)
            .map_err(|e| Error::protocol(format!("failed to set value: {}", e)))
    }

    fn delete(&self, key: &[u8]) -> Result<(), Error> {
        self.db
            .delete(key)
            .map_err(|e| Error::protocol(format!("failed to delete value: {}", e)))
    }

    fn clone_box(&self) -> Box<dyn Database + Send + Sync> {
        Box::new(self.clone())
    }
}
