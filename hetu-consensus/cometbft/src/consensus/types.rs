use cometbft::block::{Block, Height, Id as BlockId, Round};
use cometbft::validator::Set as ValidatorSet;
use cometbft::Time;
use std::time::Duration;

/// Consensus round steps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundStep {
    NewHeight,
    NewRound,
    Propose,
    Commit,
}

/// Round state for consensus
#[derive(Debug, Clone)]
pub struct RoundState {
    pub height: Height,
    pub round: Round,
    pub step: RoundStep,
    pub start_time: Time,
    pub commit_time: Time,
    pub validators: ValidatorSet,
    pub proposal: Option<Block>,
    pub proposal_block: Option<Block>,
    pub proposal_block_parts: Option<BlockId>,
    pub last_commit: Option<BlockId>,
}

/// Timeout information
#[derive(Debug, Clone)]
pub struct TimeoutInfo {
    pub duration: Duration,
    pub height: Height,
    pub round: Round,
    pub step: RoundStep,
}

/// Block proposal
#[derive(Debug, Clone)]
pub struct Proposal {
    pub height: Height,
    pub round: Round,
    pub block_id: BlockId,
    pub timestamp: Time,
}

impl RoundState {
    pub fn new(
        height: Height,
        round: Round,
        step: RoundStep,
        start_time: Time,
        validators: ValidatorSet,
    ) -> Self {
        Self {
            height,
            round,
            step,
            start_time,
            commit_time: start_time,
            validators,
            proposal: None,
            proposal_block: None,
            proposal_block_parts: None,
            last_commit: None,
        }
    }
}
