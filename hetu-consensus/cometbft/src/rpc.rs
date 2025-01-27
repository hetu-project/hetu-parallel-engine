use std::sync::Arc;
use cometbft::block::{Block, Height};
use cometbft::rpc::{Client, HttpClient, HttpClientUrl, Error};
use cometbft::validator::Set as ValidatorSet;
use std::str::FromStr;

pub struct RpcClient {
    inner: Arc<HttpClient>,
}

impl RpcClient {
    pub fn new(url: &str) -> Result<Self, Error> {
        let url = HttpClientUrl::from_str(url).map_err(|e| Error::invalid_params(e.to_string()))?;
        let client = HttpClient::new(url)?;
        Ok(Self {
            inner: Arc::new(client),
        })
    }

    pub async fn get_block(&self, height: Height) -> Result<Block, Error> {
        let response = self.inner.block(height).await?;
        Ok(response.block)
    }

    pub async fn get_latest_block(&self) -> Result<Block, Error> {
        let response = self.inner.latest_block().await?;
        Ok(response.block)
    }

    pub async fn get_validators(&self, height: Height) -> Result<ValidatorSet, Error> {
        let response = self.inner.validators(height).await?;
        Ok(response.validator_set)
    }

    pub async fn get_latest_validators(&self) -> Result<ValidatorSet, Error> {
        let response = self.inner.latest_validators().await?;
        Ok(response.validator_set)
    }

    pub async fn get_commit(&self, height: Height) -> Result<Block, Error> {
        let response = self.inner.commit(height).await?;
        Ok(response.signed_header.commit)
    }

    pub async fn get_latest_commit(&self) -> Result<Block, Error> {
        let response = self.inner.latest_commit().await?;
        Ok(response.signed_header.commit)
    }

    pub async fn get_block_results(&self, height: Height) -> Result<Block, Error> {
        let response = self.inner.block_results(height).await?;
        Ok(response.results)
    }

    pub async fn get_latest_block_results(&self) -> Result<Block, Error> {
        let response = self.inner.latest_block_results().await?;
        Ok(response.results)
    }
}
