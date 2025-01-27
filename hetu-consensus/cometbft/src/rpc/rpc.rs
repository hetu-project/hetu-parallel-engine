use std::sync::Arc;
use tokio::sync::RwLock;
use cometbft::block::{Height, Id as BlockId};
use cometbft_rpc::endpoint::{
    block::Response as BlockResponse,
    validators::Response as ValidatorsResponse,
    header::Response as HeaderResponse,
};
use crate::chain::state::ChainState;
use crate::chain::store::ChainStore;
use crate::chain::types::Error as ChainError;

#[derive(Debug)]
pub enum Error {
    InvalidParams(String),
    ChainError(ChainError),
}

impl From<ChainError> for Error {
    fn from(err: ChainError) -> Self {
        Error::ChainError(err)
    }
}

pub struct LocalRpcClient {
    chain_state: Arc<RwLock<ChainState>>,
    store: Arc<RwLock<ChainStore>>,
}

impl LocalRpcClient {
    pub fn new(chain_state: Arc<RwLock<ChainState>>, store: Arc<RwLock<ChainStore>>) -> Self {
        Self { chain_state, store }
    }

    pub async fn get_block(&self, height: Height) -> Result<BlockResponse, Error> {
        let store = self.store.read().await;
        let block = store.load_block(height)
            .map_err(Error::from)
            .and_then(|maybe_block| maybe_block
                .ok_or_else(|| Error::InvalidParams(format!("block not found at height {}", height))))?;
        
        let block_id = block.header().id();

        Ok(BlockResponse {
            block_id,
            block,
        })
    }

    pub async fn get_latest_block(&self) -> Result<BlockResponse, Error> {
        let state = self.chain_state.read().await;
        let height = state.header.height;
        drop(state);
        self.get_block(height).await
    }

    pub async fn get_validators(&self, height: Height) -> Result<ValidatorsResponse, Error> {
        let store = self.store.read().await;
        let validators = store.load_validators(height)
            .map_err(Error::from)
            .and_then(|maybe_validators| maybe_validators
                .ok_or_else(|| Error::InvalidParams(format!("validators not found at height {}", height))))?;

        Ok(ValidatorsResponse::new(
            height,
            validators.validators().to_vec(),
            validators.validators().len() as i32,
        ))
    }

    pub async fn get_latest_validators(&self) -> Result<ValidatorsResponse, Error> {
        let state = self.chain_state.read().await;
        let height = state.header.height;
        let validators = state.validators.clone()
            .ok_or_else(|| Error::InvalidParams("latest validators not found".to_string()))?;

        Ok(ValidatorsResponse::new(
            height,
            validators.validators().to_vec(),
            validators.validators().len() as i32,
        ))
    }

    pub async fn get_header(&self, height: Height) -> Result<HeaderResponse, Error> {
        let store = self.store.read().await;
        let header = store.load_block_meta(height)
            .map_err(Error::from)
            .and_then(|maybe_header| maybe_header
                .ok_or_else(|| Error::InvalidParams(format!("header not found at height {}", height))))?;

        Ok(HeaderResponse {
            header,
        })
    }

    pub async fn get_latest_header(&self) -> Result<HeaderResponse, Error> {
        let state = self.chain_state.read().await;
        Ok(HeaderResponse {
            header: state.header.clone(),
        })
    }

    pub async fn get_commit(&self, height: Height) -> Result<BlockResponse, Error> {
        // For now, return the block since commit info is part of the block
        self.get_block(height).await
    }

    pub async fn get_latest_commit(&self) -> Result<BlockResponse, Error> {
        self.get_latest_block().await
    }
}
