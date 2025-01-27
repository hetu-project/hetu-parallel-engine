use crate::chain::types::{Error, Result};
use crate::db::Database;
use cometbft::AppHash;
use cometbft::{
    block::{header::Header, Height},
    chain::Id as ChainId,
    consensus::Params,
    crypto::default::Sha256,
    validator::Set as ValidatorSet,
    Hash, Time,
};
use cometbft_proto::types::v1::ConsensusParams as RawConsensusParams;
use digest::Digest;
use serde::{Deserialize, Serialize};

const STATE_KEY: &[u8] = b"state";

#[derive(Debug, Serialize, Deserialize)]
pub struct ChainState {
    pub(crate) header: Header,
    pub(crate) validators: Option<ValidatorSet>,
    pub(crate) last_validators: Option<ValidatorSet>,
    pub(crate) last_height_validators_changed: Height,
    pub(crate) consensus_params: Params,
    pub(crate) last_height_consensus_params_changed: Height,
}

impl ChainState {
    pub fn new(
        chain_id: ChainId,
        initial_height: Height,
        validators: ValidatorSet,
        consensus_params: Params,
    ) -> Self {
        let proposer = validators.proposer().as_ref().unwrap();
        let params_bytes = serde_json::to_vec(&consensus_params).unwrap();
        let consensus_hash =
            Hash::Sha256(Sha256::digest(&params_bytes).to_vec().try_into().unwrap());

        let header = Header {
            version: cometbft::block::header::Version { block: 11, app: 1 },
            chain_id,
            height: initial_height,
            time: Time::now(),
            last_block_id: None,
            last_commit_hash: None,
            data_hash: None,
            validators_hash: validators.hash(),
            next_validators_hash: validators.hash(),
            consensus_hash,
            app_hash: AppHash::default(),
            last_results_hash: None,
            evidence_hash: None,
            proposer_address: proposer.address.clone(),
        };

        Self {
            header,
            validators: Some(validators.clone()),
            last_validators: None,
            last_height_validators_changed: initial_height,
            consensus_params,
            last_height_consensus_params_changed: initial_height,
        }
    }

    pub fn copy(&self) -> Self {
        Self {
            header: self.header.clone(),
            validators: self.validators.clone(),
            last_validators: self.last_validators.clone(),
            last_height_validators_changed: self.last_height_validators_changed,
            consensus_params: self.consensus_params.clone(),
            last_height_consensus_params_changed: self.last_height_consensus_params_changed,
        }
    }
}

/// Store manages state persistence.
pub struct Store {
    pub db: Box<dyn Database + Send + Sync>,
}

impl Store {
    pub fn new(db: Box<dyn Database + Send + Sync>) -> Self {
        Self { db }
    }

    pub fn chain_id(&self) -> Result<ChainId> {
        let state = self.load_state()?.ok_or_else(|| Error::NoState)?;
        ChainId::try_from(state.header.chain_id).map_err(|e| Error::Other(e.to_string()))
    }

    pub fn validators(&self) -> Result<ValidatorSet> {
        let state = self.load_state()?.ok_or_else(|| Error::NoState)?;
        self.load_validators(state.header.height)?
            .ok_or_else(|| Error::NoValidatorsFound)
    }

    pub fn next_validators(&self) -> Result<ValidatorSet> {
        let state = self.load_state()?.ok_or_else(|| Error::NoState)?;
        let next_height = state.header.height.increment();
        self.load_validators(next_height)?
            .ok_or_else(|| Error::NoValidatorsFound)
    }

    pub fn app_hash(&self) -> Result<AppHash> {
        Ok(self
            .load_state()?
            .ok_or_else(|| Error::NoState)?
            .header
            .app_hash)
    }

    pub fn last_results_hash(&self) -> Result<Option<Hash>> {
        Ok(self
            .load_state()?
            .ok_or_else(|| Error::NoState)?
            .header
            .last_results_hash)
    }

    pub fn version(&self) -> Result<Header> {
        Ok(self.load_state()?.ok_or_else(|| Error::NoState)?.header)
    }

    pub fn consensus_params(&self) -> Result<Params> {
        let state = self.load_state()?.ok_or_else(|| Error::NoState)?;

        // First try to load height-specific params
        if let Ok(Some(params)) = self.load_consensus_params(state.header.height.value() as i64) {
            return Ok(params);
        }

        // If no height-specific params exist, use state params
        Ok(state.consensus_params)
    }

    pub fn consensus_hash(&self) -> Result<Hash> {
        let params = self.consensus_params()?;
        let params_bytes = serde_json::to_vec(&params)
            .map_err(|e| Error::Other(format!("failed to serialize consensus params: {}", e)))?;
        Ok(Hash::Sha256(
            Sha256::digest(&params_bytes).to_vec().try_into().unwrap(),
        ))
    }

    pub fn validator_addresses(&self) -> Result<Option<Vec<cometbft::account::Id>>> {
        let validators = self.validators()?;
        Ok(Some(
            validators
                .validators()
                .iter()
                .map(|v| v.address.clone())
                .collect(),
        ))
    }

    pub(crate) fn load_state(&self) -> Result<Option<ChainState>> {
        match self.db.get(STATE_KEY)? {
            Some(bytes) => {
                let bytes = bytes.to_vec();
                let state =
                    serde_json::from_slice(&bytes).map_err(|e| Error::Other(e.to_string()))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    pub(crate) fn save_state(&self, state: &ChainState) -> Result<()> {
        let data = serde_json::to_vec(state).map_err(|e| Error::Other(e.to_string()))?;
        self.db.set(STATE_KEY, &data)?;
        Ok(())
    }

    pub fn save_validators(&self, validators: Vec<cometbft::validator::Info>) -> Result<()> {
        let proposer = validators.first().cloned();
        let validator_set = ValidatorSet::new(validators, proposer);
        let state = ChainState::new(
            self.chain_id()?,
            Height::from(0u32), // At genesis, LastBlockHeight=0 (block H=0 does not exist)
            validator_set.clone(),
            self.consensus_params()?,
        );
        self.save_state(&state)?;
        self.save_validators_height(Height::from(0u32), &validator_set)
    }

    pub fn save_initial_state(
        &mut self,
        chain_id: ChainId,
        validators: Vec<cometbft::validator::Info>,
        consensus_params: Params,
        initial_height: Height,
    ) -> Result<()> {
        let proposer = validators.first().cloned();
        let validator_set = ValidatorSet::new(validators, proposer);
        let state = ChainState::new(
            chain_id,
            initial_height,
            validator_set.clone(),
            consensus_params.clone(),
        );

        // Save state and validators
        self.save_state(&state)?;
        self.save_validators_height(initial_height, &validator_set)?;

        // Explicitly save consensus params at initial height
        self.save_consensus_params(initial_height.value() as i64, consensus_params)?;

        Ok(())
    }

    fn save_validators_height(&self, height: Height, validators: &ValidatorSet) -> Result<()> {
        let key = format!("validators/{}", height.value());
        let value = serde_json::to_vec(validators)?;
        self.db.set(key.as_bytes(), &value)?;
        Ok(())
    }

    fn load_validators(&self, height: Height) -> Result<Option<ValidatorSet>> {
        let key = format!("validators/{}", height.value());
        let validators_bytes = self
            .db
            .get(key.as_bytes())
            .map_err(|e| Error::Database(e.to_string()))?;

        match validators_bytes {
            Some(bytes) => {
                let bytes = bytes.to_vec();
                serde_json::from_slice(&bytes)
                    .map_err(|e| Error::Other(e.to_string()))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    pub fn update_state(&mut self, state: ChainState) -> Result<()> {
        println!(
            "Updating state with height={}, consensus_params.block.max_bytes={}",
            state.header.height, state.consensus_params.block.max_bytes
        );

        // Ensure consensus params are valid
        if state.consensus_params.block.max_bytes == 0 {
            return Err(Error::Other(
                "Invalid consensus params: max_bytes cannot be 0".to_string(),
            ));
        }

        let state_bytes = serde_json::to_vec(&state)
            .map_err(|e| Error::Other(format!("Failed to serialize state: {}", e)))?;

        self.db
            .set(STATE_KEY, &state_bytes)
            .map_err(|e| Error::Database(e.to_string()))?;

        Ok(())
    }

    pub fn save_consensus_params(&mut self, height: i64, params: Params) -> Result<()> {
        let key = format!("consensusParamsKey:{}", height);
        let info = ConsensusParamsInfo {
            consensus_params: params,
            last_height_changed: height, // For now, assume params changed at this height
        };
        let bytes = serde_json::to_vec(&info).map_err(|e| {
            Error::Other(format!("Failed to serialize consensus params info: {}", e))
        })?;
        self.db.set(key.as_bytes(), &bytes)?;
        Ok(())
    }

    pub fn load_consensus_params(&self, height: i64) -> Result<Option<Params>> {
        let key = format!("consensusParamsKey:{}", height);

        if let Ok(Some(bytes)) = self.db.get(key.as_bytes()) {
            let params_info: ConsensusParamsInfo = serde_json::from_slice(&bytes).map_err(|e| {
                Error::Other(format!(
                    "Failed to deserialize consensus params info: {}",
                    e
                ))
            })?;

            // If params are empty at this height, load from last_height_changed
            let empty_raw_params = RawConsensusParams::default();
            let params_proto: RawConsensusParams = params_info.consensus_params.clone().into();
            if params_proto == empty_raw_params {
                let last_changed_key =
                    format!("consensusParamsKey:{}", params_info.last_height_changed);
                if let Ok(Some(bytes)) = self.db.get(last_changed_key.as_bytes()) {
                    let last_changed_info: ConsensusParamsInfo = serde_json::from_slice(&bytes)
                        .map_err(|e| {
                            Error::Other(format!(
                                "Failed to deserialize consensus params info: {}",
                                e
                            ))
                        })?;
                    return Ok(Some(last_changed_info.consensus_params));
                }
            }

            return Ok(Some(params_info.consensus_params));
        }

        Ok(None)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConsensusParamsInfo {
    consensus_params: Params,
    last_height_changed: i64,
}

impl Clone for Store {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone_box(),
        }
    }
}
