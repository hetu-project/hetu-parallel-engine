use crate::chain::state::Store;
use crate::chain::ChainStore;
use crate::cometbft::BLOCK_PROTOCOL_VERSION;
use crate::config::Config;
use crate::consensus::types::{RoundState, RoundStep, TimeoutInfo};
use crate::consensus::{TIMEOUT_COMMIT, TIMEOUT_NEW_HEIGHT, TIMEOUT_PROPOSE};
use crate::proxyapp::AbciClient;
use crate::validator_key::PrivValidatorKey;
use cometbft::account::Id;
use cometbft::block::{
    header::{Header, Version},
    parts::Header as PartSetHeader,
    Block, Commit, Height, Id as BlockId, Round,
};
use cometbft::chain::Id as ChainId;
use cometbft::consensus::State;
use cometbft::evidence::List as EvidenceList;
use cometbft::hash::Hash;
use cometbft::validator::Set;
use cometbft::{AppHash, Error, Time};
use cometbft_proto::abci::v1::{CommitRequest, InitChainResponse, ProcessProposalStatus};
use cometbft_proto::abci::v1::{
    FinalizeBlockRequest, InitChainRequest, PrepareProposalRequest, PrepareProposalResponse,
    ProcessProposalRequest, ProcessProposalResponse, ValidatorUpdate,
};
use cometbft_proto::google::protobuf::{Duration as ProtoDuration, Timestamp};
use cometbft_proto::types::v1::{
    BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams,
};
use prost::bytes::Bytes;
use std::fmt;
use std::time::Duration as StdDuration;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

#[derive(Debug)]
pub enum ConsensusError {
    InvalidHeight(Height),
    InvalidProposal(String),
    Other(Error),
}

impl fmt::Display for ConsensusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConsensusError::InvalidHeight(h) => write!(f, "invalid height: {}", h),
            ConsensusError::InvalidProposal(s) => write!(f, "invalid proposal: {}", s),
            ConsensusError::Other(e) => write!(f, "consensus error: {}", e),
        }
    }
}

impl From<Error> for ConsensusError {
    fn from(err: Error) -> Self {
        ConsensusError::Other(err)
    }
}

impl From<crate::chain::types::Error> for ConsensusError {
    fn from(err: crate::chain::types::Error) -> Self {
        ConsensusError::Other(Error::protocol(err.to_string()))
    }
}

pub struct ConsensusState {
    pub state: State,
    pub block_store: ChainStore,
    pub state_store: Store,
    pub round_state: RoundState,
    pub proxy_app: Box<dyn AbciClient>,
    pub quit_rx: mpsc::Receiver<()>,
    pub config: Config,
    pub validator_key: Option<PrivValidatorKey>,
    pub timeout_tx: mpsc::Sender<TimeoutInfo>,
    pub timeout_rx: mpsc::Receiver<TimeoutInfo>,
}

impl ConsensusState {
    pub fn new(
        state: State,
        block_store: ChainStore,
        state_store: Store,
        proxy_app: Box<dyn AbciClient>,
        quit_rx: mpsc::Receiver<()>,
        config: Config,
        validator_key: Option<PrivValidatorKey>,
    ) -> Self {
        let (timeout_tx, timeout_rx) = mpsc::channel(100);
        let height = state.height;
        let round = state.round;
        let validators = state_store.validators().unwrap();
        Self {
            state,
            block_store,
            state_store,
            round_state: RoundState::new(
                height,
                round,
                RoundStep::NewHeight,
                Time::now(),
                validators,
            ),
            proxy_app,
            quit_rx,
            config,
            validator_key,
            timeout_tx,
            timeout_rx,
        }
    }

    pub async fn start(&mut self) -> Result<(), ConsensusError> {
        // Start from NewHeight
        self.enter_new_height().await?;

        // Schedule first timeout
        self.schedule_timeout(TIMEOUT_NEW_HEIGHT, RoundStep::NewHeight)
            .await;

        // Main consensus loop - never exit
        loop {
            // Handle timeouts and quit signals
            match self.receive_routine().await {
                Ok(()) => {
                    println!("Consensus loop exiting due to quit signal");
                    break;
                }
                Err(e) => {
                    println!("Error in consensus loop: {}", e);
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn enter_new_height(&mut self) -> Result<(), ConsensusError> {
        // At genesis, LastBlockHeight is 0, so increment will give us height 1
        let new_height = self.round_state.height.increment();

        // Update state for new height
        self.round_state.height = new_height;
        self.round_state.round = Round::default();
        self.round_state.step = RoundStep::NewHeight;
        self.round_state.proposal = None;
        self.round_state.proposal_block = None;
        self.round_state.proposal_block_parts = None;
        self.round_state.start_time = Time::now();

        // Schedule timeout for new height
        self.schedule_timeout(self.config.consensus.timeout_commit, RoundStep::NewHeight)
            .await;

        Ok(())
    }

    async fn receive_routine(&mut self) -> Result<(), ConsensusError> {
        loop {
            println!("Waiting for timeout or quit signal...");
            tokio::select! {
                Some(timeout) = self.timeout_rx.recv() => {
                    println!("Received timeout for step {:?}", timeout.step);
                    self.handle_timeout(timeout).await?;
                }
                // _ = self.quit_rx.recv() => {
                //     println!("Received quit signal, exiting receive routine");
                //     return Ok(());
                // }
            }
        }
    }

    async fn handle_timeout(&mut self, timeout: TimeoutInfo) -> Result<(), ConsensusError> {
        // Ignore timeouts from previous heights/rounds
        if timeout.height != self.round_state.height || timeout.round != self.round_state.round {
            println!(
                "Ignoring timeout for height {} round {}, current height {} round {}",
                timeout.height, timeout.round, self.round_state.height, self.round_state.round
            );
            return Ok(());
        }

        println!("Handling timeout for step {:?}", timeout.step);
        match timeout.step {
            RoundStep::NewHeight => {
                println!("Moving to propose step");
                // Move to propose
                self.enter_new_round(timeout.height, timeout.round).await?;
                self.enter_propose(timeout.height, timeout.round).await?;
                self.schedule_timeout(TIMEOUT_PROPOSE, RoundStep::Propose)
                    .await;
            }
            RoundStep::Propose => {
                // Get the proposal block (should exist since we create it in enter_propose)
                let block = self.round_state.proposal_block.clone().ok_or_else(|| {
                    ConsensusError::InvalidProposal("No proposal block".to_string())
                })?;

                println!("Sending PrepareProposal request");
                // Send PrepareProposal request to ABCI
                let prepare_proposal_req = self.create_prepare_proposal_request(&block).await?;
                let prepare_proposal_resp = self
                    .proxy_app
                    .prepare_proposal(&prepare_proposal_req)
                    .await
                    .map_err(|e| ConsensusError::Other(Error::protocol(e.to_string())))?;

                println!("Updating block with PrepareProposal response");
                // Update block with ABCI response
                let block = self
                    .update_block_with_prepare_proposal(block, prepare_proposal_resp)
                    .await?;

                println!("Sending ProcessProposal request");
                // Send ProcessProposal request to ABCI
                let process_proposal_req = self.create_process_proposal_request(&block).await?;
                let process_proposal_resp = self
                    .proxy_app
                    .process_proposal(&process_proposal_req)
                    .await
                    .map_err(|e| ConsensusError::Other(Error::protocol(e.to_string())))?;

                println!(
                    "ProcessProposal response status: {:?}",
                    process_proposal_resp.status
                );
                if process_proposal_resp.status == ProcessProposalStatus::Accept as i32 {
                    println!("Proposal accepted, storing block");
                    // Store the accepted block
                    self.round_state.proposal_block = Some(block);

                    println!("Entering commit");
                    // Enter commit
                    self.enter_commit(timeout.height).await?;
                    self.schedule_timeout(TIMEOUT_COMMIT, RoundStep::Commit)
                        .await;
                }
            }
            RoundStep::Commit => {
                // Get the proposal block
                let block = self.round_state.proposal_block.clone().ok_or_else(|| {
                    ConsensusError::InvalidProposal("No proposal block in commit".to_string())
                })?;

                // Commit block (includes FinalizeBlock, Commit ABCI request, and saving locally)
                self.commit_block(&block).await?;

                // Move to next height
                self.enter_new_height().await?;
                self.schedule_timeout(TIMEOUT_NEW_HEIGHT, RoundStep::NewHeight)
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn schedule_timeout(&mut self, duration: Duration, step: RoundStep) {
        println!(
            "Scheduling timeout for step {:?} with duration {:?}",
            step, duration
        );
        let timeout = TimeoutInfo {
            duration,
            height: self.round_state.height,
            round: self.round_state.round,
            step,
        };

        // Schedule the timeout
        let timeout_tx = self.timeout_tx.clone();
        let timeout_info = timeout.clone();
        let step = timeout_info.step;

        // Spawn the timeout task and let it run independently
        tokio::spawn(async move {
            println!("Starting timeout for step {:?}", step);
            tokio::time::sleep(duration).await;
            println!("Timeout expired for step {:?}, sending", step);
            // Send the timeout, if it fails the channel is closed
            if let Err(_) = timeout_tx.send(timeout_info).await {
                println!("Failed to send timeout - channel closed");
            } else {
                println!("Successfully sent timeout for step {:?}", step);
            }
        });
    }

    fn create_block_header(&self, height: Height) -> Result<Header, ConsensusError> {
        // Get proposer info
        let proposer = self
            .round_state
            .validators
            .proposer()
            .as_ref()
            .ok_or_else(|| {
                ConsensusError::Other(Error::protocol("no proposer found".parse().unwrap()))
            })?
            .clone();

        // Get last block ID - None for genesis block
        let last_block_id = if height == self.state.height {
            None
        } else {
            self.block_store.last_block_id()?
        };

        Ok(Header {
            version: Version {
                block: BLOCK_PROTOCOL_VERSION,
                app: 1,
            },
            chain_id: ChainId::try_from(self.config.base.chain_id.clone())?,
            height,
            time: self.round_state.start_time,
            last_block_id,
            last_commit_hash: None,
            data_hash: None,
            validators_hash: self.round_state.validators.hash(),
            next_validators_hash: self.round_state.validators.hash(),
            consensus_hash: Hash::default(),
            app_hash: AppHash::default(),
            last_results_hash: None,
            evidence_hash: None,
            proposer_address: proposer.address,
        })
    }

    async fn create_block(&self, height: Height) -> Result<Block, ConsensusError> {
        let header = self.create_block_header(height)?;

        // For genesis block (height == initial height), last_commit should be None
        // For other blocks, we need a non-None last_commit
        let last_commit = if u64::from(height) <= 1 {
            // For genesis block (height 1), there is no last commit since height 0 block doesn't exist
            None
        } else {
            // Create an empty commit for now
            Some(Commit {
                height: Height::from((u64::from(height) - 1) as u32),
                round: Round::default(),
                block_id: self.block_store.last_block_id()?.unwrap_or_default(),
                signatures: vec![],
            })
        };

        Ok(Block::new(
            header,
            vec![],
            EvidenceList::default(),
            last_commit,
        )?)
    }

    pub async fn prepare_proposal(&mut self, height: Height) -> Result<Block, ConsensusError> {
        // Get proposer info
        let proposer = self
            .round_state
            .validators
            .proposer()
            .as_ref()
            .ok_or_else(|| {
                ConsensusError::Other(Error::protocol("no proposer found".parse().unwrap()))
            })?
            .clone();

        // Create empty block
        let block = self.create_block(height).await?;

        // Sign block if we're the proposer
        if let Some(validator_key) = &mut self.validator_key {
            let mut signed_block = block.clone();
            validator_key.sign_block(&mut signed_block).map_err(|e| {
                ConsensusError::Other(Error::protocol(format!("failed to sign block: {}", e)))
            })?;
            Ok(signed_block)
        } else {
            Ok(block)
        }
    }

    pub async fn create_prepare_proposal_request(
        &self,
        block: &Block,
    ) -> Result<PrepareProposalRequest, ConsensusError> {
        let txs = vec![]; // Empty txs for now
        Ok(PrepareProposalRequest {
            max_tx_bytes: self.config.max_tx_bytes as i64,
            txs,
            local_last_commit: None,
            misbehavior: vec![],
            height: block.header.height.value() as i64,
            time: Some(Timestamp {
                seconds: block.header.time.unix_timestamp(),
                nanos: 0,
            }),
            next_validators_hash: block.header.next_validators_hash.as_bytes().to_vec().into(),
            proposer_address: block.header.proposer_address.as_bytes().to_vec().into(),
        })
    }

    pub async fn update_block_with_prepare_proposal(
        &self,
        mut block: Block,
        resp: PrepareProposalResponse,
    ) -> Result<Block, ConsensusError> {
        // Update block with response from ABCI
        block.data = resp.txs.into_iter().map(|b| b.to_vec()).collect();
        Ok(block)
    }

    pub async fn create_process_proposal_request(
        &self,
        block: &Block,
    ) -> Result<ProcessProposalRequest, ConsensusError> {
        Ok(ProcessProposalRequest {
            txs: block.data.iter().map(|b| b.to_vec().into()).collect(),
            proposed_last_commit: None,
            misbehavior: vec![],
            hash: block.header.hash().as_bytes().to_vec().into(),
            height: block.header.height.value() as i64,
            time: Some(Timestamp {
                seconds: block.header.time.unix_timestamp(),
                nanos: 0,
            }),
            next_validators_hash: block.header.next_validators_hash.as_bytes().to_vec().into(),
            proposer_address: block.header.proposer_address.as_bytes().to_vec().into(),
        })
    }

    pub async fn create_finalize_block_request(
        &self,
        block: &Block,
    ) -> Result<FinalizeBlockRequest, ConsensusError> {
        Ok(FinalizeBlockRequest {
            hash: block.header.hash().as_bytes().to_vec().into(),
            txs: block.data.iter().map(|b| b.to_vec().into()).collect(),
            decided_last_commit: None,
            misbehavior: vec![],
            height: block.header.height.value() as i64,
            time: Some(Timestamp {
                seconds: block.header.time.unix_timestamp(),
                nanos: 0,
            }),
            next_validators_hash: block.header.next_validators_hash.as_bytes().to_vec().into(),
            proposer_address: block.header.proposer_address.as_bytes().to_vec().into(),
        })
    }

    pub async fn process_proposal(&mut self, block: &Block) -> Result<bool, ConsensusError> {
        println!(
            "Processing proposal for block at height: {}",
            block.header.height
        );

        // Log block details
        println!(
            "Block details: time={:?}, num_txs={}, last_block_id={:?}",
            block.header.time,
            block.data.len(),
            block.header.last_block_id
        );

        // Create process proposal request
        let process_req = self.create_process_proposal_request(block).await?;
        println!(
            "Created ProcessProposal request: txs={}, height={}",
            process_req.txs.len(),
            process_req.height
        );

        // Send process proposal request
        println!("Sending ProcessProposal request to app");
        let process_resp = self.proxy_app.process_proposal(&process_req).await?;
        println!(
            "Received ProcessProposal response: status={:?}",
            process_resp.status
        );

        let process_proposal: ProcessProposalResponse = process_resp.into();

        let accepted = process_proposal.status == ProcessProposalStatus::Accept as i32;
        println!(
            "Proposal {} at height {}",
            if accepted { "ACCEPTED" } else { "REJECTED" },
            block.header.height
        );

        Ok(accepted)
    }

    async fn save_block(&mut self, block: &Block) -> Result<(), ConsensusError> {
        // TODO: Save block to store
        println!("Saving block to store: height={}", block.header.height);
        Ok(())
    }

    pub async fn commit_block(&mut self, block: &Block) -> Result<(), ConsensusError> {
        // First finalize the block
        println!("Sending FinalizeBlock request");
        let finalize_req = self.create_finalize_block_request(block).await?;
        let finalize_resp = self
            .proxy_app
            .finalize_block(&finalize_req)
            .await
            .map_err(|e| ConsensusError::Other(Error::protocol(e.to_string())))?;

        println!(
            "FinalizeBlock response received, events={}, validator_updates={}",
            finalize_resp.events.len(),
            finalize_resp.validator_updates.len()
        );

        // Then commit via ABCI
        println!("Sending Commit request");
        let commit_req = CommitRequest {};
        let commit_resp = self
            .proxy_app
            .commit(&commit_req)
            .await
            .map_err(|e| ConsensusError::Other(Error::protocol(e.to_string())))?;

        println!(
            "Commit response received, retain_height={}",
            commit_resp.retain_height
        );

        // Finally save block locally
        println!("Saving block locally");
        self.save_block(block).await?;

        // Update state
        self.round_state.height = Height::try_from(block.header.height.value())
            .map_err(|_| ConsensusError::Other(Error::protocol("Invalid height".to_string())))?;
        self.round_state.step = RoundStep::NewHeight;
        self.round_state.round = Round::try_from(0)
            .map_err(|_| ConsensusError::Other(Error::protocol("Invalid round".to_string())))?;
        self.round_state.proposal_block = None;
        self.round_state.proposal_block_parts = None;

        Ok(())
    }

    pub async fn create_and_sign_block(&mut self) -> Result<Block, ConsensusError> {
        let mut block = self.create_proposal_block().await?;

        if let Some(validator_key) = &mut self.validator_key {
            validator_key.sign_block(&mut block).map_err(|e| {
                ConsensusError::Other(Error::protocol(format!("failed to sign block: {}", e)))
            })?;
        }

        Ok(block)
    }

    pub async fn create_proposal_block(&mut self) -> Result<Block, ConsensusError> {
        self.prepare_proposal(self.round_state.height).await
    }

    pub fn is_proposal_valid(&self, block: &Block) -> bool {
        // For now just check height matches
        block.header.height == self.round_state.height
    }

    pub fn is_proposer(&self) -> bool {
        // In local node mode, we are always the proposer
        true
    }

    pub async fn enter_new_round(
        &mut self,
        height: Height,
        round: Round,
    ) -> Result<(), ConsensusError> {
        // Update state
        self.round_state.height = height;
        self.round_state.round = round;
        self.round_state.step = RoundStep::NewRound;

        // Reset proposal state
        self.round_state.proposal = None;
        self.round_state.proposal_block = None;
        self.round_state.proposal_block_parts = None;

        // Update times
        self.round_state.start_time = Time::now();
        self.round_state.commit_time = Time::now();

        Ok(())
    }

    pub async fn enter_propose(
        &mut self,
        height: Height,
        round: Round,
    ) -> Result<(), ConsensusError> {
        // Check if we're at the right height and round
        if height != self.round_state.height || round != self.round_state.round {
            println!(
                "Invalid height/round in enter_propose: got {}/{}, expected {}/{}",
                height, round, self.round_state.height, self.round_state.round
            );
            return Err(ConsensusError::InvalidHeight(height));
        }

        println!("Entering propose step");
        // Update step
        self.round_state.step = RoundStep::Propose;

        // Create proposal block (we're always the proposer in local mode)
        println!("Creating proposal block");
        let block = self.create_proposal_block().await?;
        self.round_state.proposal_block = Some(block);

        Ok(())
    }

    pub async fn enter_commit(&mut self, height: Height) -> Result<(), ConsensusError> {
        // Check if we're at the right height
        if height != self.round_state.height {
            return Err(ConsensusError::InvalidHeight(height));
        }

        // Update step
        self.round_state.step = RoundStep::Commit;

        // Get proposal block
        let block = match &self.round_state.proposal_block {
            Some(b) => b.clone(),
            None => {
                return Err(ConsensusError::InvalidProposal(
                    "No proposal block".to_string(),
                ))
            }
        };

        // Create empty commit
        let commit = Commit {
            height,
            round: self.round_state.round,
            block_id: BlockId {
                hash: block.header.hash(),
                part_set_header: PartSetHeader::default(),
            },
            signatures: vec![], // No signatures needed in local mode
        };

        // Save block to store
        self.save_block(&block).await?;

        // Update commit time
        self.round_state.commit_time = Time::now();

        Ok(())
    }

    /// Create InitChain request for ABCI app
    pub async fn create_init_chain_request(&self) -> Result<InitChainRequest, ConsensusError> {
        // Get genesis time
        let genesis_time = self.round_state.start_time;

        // Get consensus params from genesis
        let genesis_params = self.state_store.consensus_params()?;
        let consensus_params = Some(ConsensusParams {
            block: Some(BlockParams {
                max_bytes: genesis_params.block.max_bytes as i64,
                max_gas: genesis_params.block.max_gas,
            }),
            evidence: Some(EvidenceParams {
                max_age_num_blocks: genesis_params.evidence.max_age_num_blocks as i64,
                max_age_duration: Some(ProtoDuration {
                    seconds: genesis_params.evidence.max_age_duration.0.as_secs() as i64,
                    nanos: 0,
                }),
                max_bytes: genesis_params.evidence.max_bytes,
            }),
            validator: Some(ValidatorParams {
                pub_key_types: genesis_params
                    .validator
                    .pub_key_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect(),
            }),
            version: Some(VersionParams {
                app: genesis_params.version.as_ref().map(|v| v.app).unwrap_or(0),
            }),
            abci: None,
        });

        // Convert validators to updates
        let validator_updates: Vec<ValidatorUpdate> = self
            .round_state
            .validators
            .validators()
            .iter()
            .map(|v| ValidatorUpdate {
                pub_key: Some(v.pub_key.clone().into()),
                power: v.power.into(),
            })
            .collect();

        // Create request
        Ok(InitChainRequest {
            time: Some(genesis_time.into()),
            chain_id: self.config.base.chain_id.clone(),
            consensus_params,
            validators: vec![],
            app_state_bytes: Bytes::new(),
            initial_height: self.state.height.into(),
        })
    }

    /// Send InitChain request to ABCI app
    pub async fn init_chain(
        &mut self,
        app_state_bytes: Vec<u8>,
    ) -> Result<InitChainResponse, ConsensusError> {
        // Create request
        let mut init_chain_req = self.create_init_chain_request().await?;

        // Set app state
        init_chain_req.app_state_bytes = app_state_bytes.into();

        // Send request
        self.proxy_app
            .init_chain(&init_chain_req)
            .await
            .map_err(|e| {
                ConsensusError::Other(Error::protocol(format!("failed to init chain: {}", e)))
            })
    }
}
