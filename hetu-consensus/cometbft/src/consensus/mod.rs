pub mod state;
pub mod types;

pub use state::ConsensusState;
pub use types::{RoundStep, RoundState, TimeoutInfo, Proposal};

use std::time::Duration;
use cometbft::{
    block::{Height, Size},
    consensus::{params::{AbciParams, ValidatorParams, VersionParams}, Params},
    evidence,
    public_key::Algorithm,
};

// Default consensus parameters for local node
pub fn default_consensus_params() -> Params {
    Params {
        block: Size {
            max_bytes: 22020096,
            max_gas: -1,
            time_iota_ms: 1000, // 1 second block time
        },
        evidence: evidence::Params {
            max_age_num_blocks: 100000,
            max_age_duration: evidence::Duration(Duration::from_secs(48 * 3600)), // 48 hours
            max_bytes: 1048576,
        },
        validator: ValidatorParams {
            pub_key_types: vec![Algorithm::Ed25519],
        },
        version: Some(VersionParams {
            app: 1,
        }),
        abci: AbciParams {
            vote_extensions_enable_height: Some(Height::from(0u32)),
        },
    }
}

// Default timeout durations
pub const TIMEOUT_PROPOSE: Duration = Duration::from_secs(3);
pub const TIMEOUT_COMMIT: Duration = Duration::from_secs(1);
pub const TIMEOUT_NEW_HEIGHT: Duration = Duration::from_millis(10);
