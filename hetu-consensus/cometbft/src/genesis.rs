use cometbft::Error;
use cometbft::{consensus, validator, PublicKey, Time};
use cometbft_proto::abci::v1::{InitChainRequest, ValidatorUpdate};
use cometbft_proto::crypto::v1::{
    public_key::Sum as ProtoPublicKeySum, PublicKey as ProtoPublicKey,
};
use cometbft_proto::google::protobuf::{Duration, Timestamp};
use serde::Deserialize;
use serde_json;
use std::fs;

/// Load genesis doc from config
#[derive(Debug, Deserialize)]
pub struct GenesisDoc {
    pub genesis_time: Time,
    pub chain_id: String,
    pub initial_height: i64,
    pub app_state: serde_json::Value,
    pub consensus: ConsensusWrapper,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsensusWrapper {
    pub params: consensus::Params,
}

impl GenesisDoc {
    pub fn load(path: &str) -> Result<Self, Error> {
        let contents = fs::read_to_string(path)
            .map_err(|e| Error::invalid_key(format!("Failed to read genesis file: {}", e)))?;

        if let Ok(doc) = serde_json::from_str(&contents) {
            Ok(doc)
        } else {
            Err(Error::invalid_key(
                "Failed to parse genesis file".to_string(),
            ))
        }
    }

    // pub fn create_init_chain_request(&self) -> InitChainRequest {
    //     // Get app state
    //     let app_state_bytes = serde_json::to_vec(&self.app_state).unwrap_or_default();
    //
    //     InitChainRequest {
    //         time: Some(Timestamp {
    //             seconds: self.genesis_time.unix_timestamp(),
    //             nanos: 0,
    //         }),
    //         chain_id: self.chain_id.clone(),
    //         consensus_params: Some(self.consensus.params.clone().into()),
    //         validators: vec![],  // Empty validator set - validators will be returned in InitChainResponse
    //         app_state_bytes: app_state_bytes.into(),
    //         initial_height: self.initial_height,
    //     }
    // }
}
