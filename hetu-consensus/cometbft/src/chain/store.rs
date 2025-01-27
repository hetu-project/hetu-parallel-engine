use crate::chain::types::{Error, Result};
use crate::db::Database;
use cometbft::{
    block::parts::Header as PartSetHeader,
    block::Id as BlockId,
    block::{Block, Height},
    Hash,
};
use serde::{Deserialize, Serialize};

const BLOCK_STORE_KEY: &[u8] = b"block_store";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockStoreState {
    pub base: Height,
    pub height: Height,
}

pub struct ChainStore {
    pub db: Box<dyn Database + Send + Sync>,
    pub state: BlockStoreState,
}

impl ChainStore {
    pub fn new(db: Box<dyn Database + Send + Sync>) -> Result<Self> {
        let state = BlockStoreState {
            base: Height::from(0u32),
            height: Height::from(0u32),
        };
        Ok(Self { db, state })
    }

    pub fn save_block(&self, block: &Block) -> Result<()> {
        let block_key = self.calc_block_key(block.header.height);
        let block_data = serde_json::to_vec(block)
            .map_err(|e| Error::invalid_key(format!("failed to serialize block: {}", e)))?;
        self.db.set(&block_key, &block_data)?;

        let parts_key = self.calc_block_parts_key(block.header.height);
        let parts_data = serde_json::to_vec(&block.header.height)
            .map_err(|e| Error::invalid_key(format!("failed to serialize block parts: {}", e)))?;
        self.db.set(&parts_key, &parts_data)?;

        let commit_key = self.calc_block_commit_key(block.header.height);
        let commit_data = serde_json::to_vec(&block.last_commit)
            .map_err(|e| Error::invalid_key(format!("failed to serialize block commit: {}", e)))?;
        self.db.set(&commit_key, &commit_data)?;

        let state_data = serde_json::to_vec(&self.state).map_err(|e| {
            Error::invalid_key(format!("failed to serialize block store state: {}", e))
        })?;
        self.db.set(BLOCK_STORE_KEY, &state_data)?;

        Ok(())
    }

    pub fn load_block(&self, height: Height) -> Result<Option<Block>> {
        match self.db.get(&self.calc_block_key(height))? {
            Some(bytes) => {
                let bytes = bytes.to_vec();
                let block = serde_json::from_slice(&bytes).map_err(|e| {
                    Error::invalid_key(format!("failed to deserialize block: {}", e))
                })?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    pub fn load_commit(&self, height: Height) -> Result<Option<cometbft::block::Commit>> {
        let key = self.calc_block_commit_key(height);
        match self.db.get(&key)? {
            Some(bytes) => {
                let commit = serde_json::from_slice(&bytes)?;
                Ok(Some(commit))
            }
            None => Ok(None),
        }
    }

    pub fn last_block_hash(&self) -> Result<Hash> {
        if self.state.height == Height::from(0u32) {
            Ok(Hash::default())
        } else {
            self.load_block(self.state.height)?
                .ok_or_else(|| Error::InvalidBlock("No last block found".to_string()))
                .map(|block| block.header.hash())
        }
    }

    pub fn consensus_hash(&self) -> Result<Hash> {
        self.load_block(self.state.height)?
            .ok_or_else(|| Error::InvalidBlock("No last block found".to_string()))
            .map(|block| block.header.consensus_hash)
    }

    pub fn last_block_id(&self) -> Result<Option<BlockId>> {
        // Get last block hash
        let last_hash = self.last_block_hash()?;

        // Convert Hash to Option<BlockId>
        if last_hash == Hash::default() {
            Ok(None)
        } else {
            Ok(Some(BlockId {
                hash: last_hash,
                part_set_header: PartSetHeader::default(),
            }))
        }
    }

    fn calc_block_key(&self, height: Height) -> Vec<u8> {
        format!("block:{}", height).into_bytes()
    }

    fn calc_block_parts_key(&self, height: Height) -> Vec<u8> {
        format!("block_parts:{}", height).into_bytes()
    }

    fn calc_block_commit_key(&self, height: Height) -> Vec<u8> {
        format!("block_commit:{}", height).into_bytes()
    }
}

impl Clone for ChainStore {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone_box(),
            state: self.state.clone(),
        }
    }
}
