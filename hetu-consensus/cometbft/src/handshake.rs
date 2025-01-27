use crate::chain::state::ChainState;
use crate::genesis::GenesisDoc as Genesis;
use crate::proxyapp::{AbciClient, RemoteClient as Client};
use cometbft::block::Height;
use cometbft::chain::Id as ChainId;
use cometbft::validator::{Info as ValidatorInfo, Set as ValidatorSet};
use cometbft::{AppHash, Error};
use cometbft_proto::{
    abci::v1::{InfoRequest, InitChainRequest, ValidatorUpdate},
    types::v1::{ConsensusParams as ProtoConsensusParams, Validator as ProtoValidator},
};
use log::info;
use std::convert::TryFrom;

use crate::cometbft::{
    ABCI_VERSION, BLOCK_PROTOCOL_VERSION, COMETBFT_VERSION, P2P_PROTOCOL_VERSION,
};

pub struct Handshaker {
    app_client: Client,
    initial_state: ChainState,
    genesis: Genesis,
}

impl Handshaker {
    pub fn new(app_client: Client, initial_state: ChainState, genesis: Genesis) -> Self {
        Self {
            app_client,
            initial_state,
            genesis,
        }
    }

    /// Perform handshake with the app
    pub async fn handshake(&mut self) -> Result<(), Error> {
        // Send info request to get app info
        let info_req = InfoRequest {
            version: COMETBFT_VERSION.to_string(),
            block_version: BLOCK_PROTOCOL_VERSION,
            p2p_version: P2P_PROTOCOL_VERSION,
            abci_version: ABCI_VERSION.to_string(),
        };
        let res = self.app_client.info(&info_req).await?;

        let app_hash = res.last_block_app_hash;
        let app_height = res.last_block_height;
        let app_version = res.app_version;

        // Validate block height
        if app_height < 0 {
            return Err(Error::protocol(format!(
                "got negative last block height: {}, expected >= 0",
                app_height
            )));
        }

        info!(
            "ABCI Handshake App Info: height={}, hash={:X}, version={}, protocol={}",
            app_height, app_hash, res.version, app_version
        );

        // Only set app version if no existing state
        if self.initial_state.header.height == Height::from(0u32) {
            self.initial_state.header.version.app = app_version;
        }

        // For new chains, initialize chain
        // if app_height == 0 {
        //     // Convert validators to ValidatorSet
        //     let validators = ValidatorSet::new(
        //         self.genesis.validators.clone(),
        //         None, // proposer will be set in ValidatorSet::new
        //     );
        //
        //     // Create init chain request
        //     let req = self.genesis.create_init_chain_request();
        //
        //     let res = self.app_client.init_chain(&req).await?;
        //
        //     let app_validators = res.validators;
        //     let app_hash = res.app_hash;
        //     let consensus_params = res.consensus_params.unwrap_or_default();
        //
        //     // Validate app hash from init chain
        //     if !app_hash.is_empty() && app_hash.len() != 32 {
        //         return Err(Error::protocol(format!(
        //             "invalid app hash length from init chain: {}, expected: 32",
        //             app_hash.len()
        //         )));
        //     }
        //
        //     // Create new state with initial values
        //     let mut state = ChainState::new(
        //         ChainId::try_from(self.genesis.chain_id.as_str())?,
        //         Height::try_from(self.genesis.initial_height)?,
        //         validators,
        //         consensus_params.try_into()?,
        //     );
        //
        //     // Update validators if app returned them
        //     if !app_validators.is_empty() {
        //         let new_validators: Vec<ValidatorInfo> = app_validators.into_iter()
        //             .map(|v| {
        //                 if v.power < 0 {
        //                     return Err(Error::protocol(format!(
        //                         "invalid validator power from app: {}, cannot be negative",
        //                         v.power
        //                     )));
        //                 }
        //                 // Convert ValidatorUpdate to Validator proto
        //                 let proto_val = ProtoValidator {
        //                     address: vec![],  // Not used in ValidatorInfo
        //                     pub_key: v.pub_key,
        //                     proposer_priority: 0,  // Not used in ValidatorInfo
        //                     voting_power: v.power as i64,
        //                 };
        //                 ValidatorInfo::try_from(proto_val)
        //             })
        //             .collect::<Result<_, _>>()?;
        //         state.validators = Some(ValidatorSet::new(new_validators, None));
        //     }
        // }

        info!(
            "Completed ABCI Handshake - CometBFT and App are synced: height={}, hash={:X}",
            app_height, app_hash
        );

        Ok(())
    }
}
