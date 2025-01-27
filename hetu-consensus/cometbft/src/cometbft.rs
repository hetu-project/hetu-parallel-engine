use cometbft::{
    block::{Height, Round, Size},
    chain::Id as ChainId,
    consensus::params::{AbciParams, ValidatorParams, VersionParams},
    consensus::{Params, State},
    evidence::{self, Duration},
    public_key, validator,
    vote::Power,
    Error as CometError,
};
use cometbft_proto::abci::v1::InfoRequest;
use std::{fs, io, net::SocketAddr, path::Path, str::FromStr};

use crate::{
    chain::state::Store,
    chain::ChainStore,
    config::{BaseConfig, Config},
    consensus::state::ConsensusState,
    db::RocksDB,
    genesis::GenesisDoc,
    node_key::NodeKey,
    proxyapp::{AbciClient, RemoteClient},
    validator_key::PrivValidatorKey,
};

pub const COMETBFT_VERSION: &str = "0.38.15";
pub const BLOCK_PROTOCOL_VERSION: u64 = 11;
pub const P2P_PROTOCOL_VERSION: u64 = 8;
pub const ABCI_VERSION: &str = "2.0.0";

pub struct CometNode {
    config: Config,
    consensus: Option<ConsensusState>,
    abci_client: Option<Box<dyn AbciClient>>,
    block_store: Option<ChainStore>,
    state_store: Option<Store>,
    validator_key: Option<PrivValidatorKey>,
}

impl CometNode {
    pub fn new(config: Config) -> Self {
        CometNode {
            config,
            consensus: None,
            abci_client: None,
            block_store: None,
            state_store: None,
            validator_key: None,
        }
    }

    pub fn default() -> Self {
        let config = Config {
            base: BaseConfig::default(),
            rpc: Default::default(),
            consensus: Default::default(),
            max_tx_bytes: 1048576, // 1MB default
        };
        Self::new(config)
    }

    fn io_err(e: io::Error) -> CometError {
        CometError::invalid_key(e.to_string())
    }

    /// Initialize the node
    pub async fn init(&mut self) -> Result<(), CometError> {
        // Create directories if they don't exist
        let db_dir = self.config.base.root_dir.join("data");
        if !db_dir.exists() {
            fs::create_dir_all(&db_dir).map_err(Self::io_err)?;
        }

        // Load node key
        let _node_key = NodeKey::load_or_generate(Path::new(&self.config.base.node_key_file))
            .map_err(Self::io_err)?;

        // Load genesis doc
        let genesis = GenesisDoc::load(&self.config.base.genesis_file)?;

        // Create chain database
        let chain_db_path = db_dir.join("chain");
        let db = RocksDB::open(&chain_db_path)
            .map_err(|e| CometError::protocol(format!("failed to create chain database: {}", e)))?;

        // Create state database
        let state_db_path = db_dir.join("state");
        let state_db = RocksDB::open(&state_db_path)
            .map_err(|e| CometError::protocol(format!("failed to create state database: {}", e)))?;

        // Create chain store
        let chain_store = ChainStore::new(Box::new(db.clone()))?;

        // Create state store
        let mut state_store = Store::new(Box::new(state_db.clone()));

        // Load validator key
        let validator_key = PrivValidatorKey::load_or_generate(
            Path::new(&self.config.base.priv_validator_key_file),
            &self.config.base.chain_id,
        )
        .map_err(|e| CometError::protocol(format!("failed to load validator key: {}", e)))?;

        // Parse ABCI address
        let proxy_app = &self.config.base.proxy_app;
        let addr = if proxy_app.starts_with("tcp://") {
            SocketAddr::from_str(&proxy_app[6..])
                .map_err(|e| CometError::invalid_key(format!("Invalid proxy app address: {}", e)))?
        } else {
            return Err(CometError::invalid_key(
                "Proxy app must start with tcp://".to_string(),
            ));
        };

        // Create and connect ABCI client
        let mut client = RemoteClient::new(addr.to_string());
        client.connect().await?;

        // Get app info
        let info_req = InfoRequest {
            version: ABCI_VERSION.to_string(),
            block_version: BLOCK_PROTOCOL_VERSION,
            p2p_version: P2P_PROTOCOL_VERSION,
            abci_version: ABCI_VERSION.to_string(),
        };

        let info_resp = client.info(&info_req).await?;
        println!("Info Response: {:?}", info_resp);

        // Create initial state - start at height 0 for genesis
        let initial_height = Height::from(genesis.initial_height as u32);
        let chain_id = ChainId::try_from(genesis.chain_id.clone())
            .map_err(|e| CometError::protocol(format!("Invalid chain ID: {}", e)))?;

        // Create initial validator set with our validator key
        let validator_info = validator::Info::new(*validator_key.pub_key(), Power::try_from(1u32)?);

        // Initialize state store with genesis state
        state_store.save_initial_state(
            chain_id.clone(),
            vec![validator_info], // Use our validator key as the only validator
            genesis.consensus.params.clone(),
            initial_height,
        )?;

        let state = State {
            height: Height::from(0u32), // At genesis, LastBlockHeight=0 (block H=0 does not exist)
            round: Round::default(),
            step: 0,
            block_id: None,
        };

        // Get app state from genesis
        let app_state_bytes = serde_json::to_vec(&genesis.app_state).unwrap_or_default();

        // Create consensus state
        let mut consensus_state = ConsensusState::new(
            state,
            chain_store.clone(),
            state_store.clone(),
            Box::new(client), // Move client into consensus state
            tokio::sync::mpsc::channel(100).1,
            self.config.clone(),
            Some(validator_key.clone()),
        );

        // Initialize chain with genesis
        let init_chain_resp = consensus_state
            .init_chain(app_state_bytes)
            .await
            .map_err(|e| CometError::protocol(format!("failed to init chain: {}", e)))?;
        println!("InitChain Response: {:?}", init_chain_resp);

        // Save validators and consensus params from init chain response
        let validators = init_chain_resp
            .validators
            .iter()
            .map(|v| {
                let pub_key = match &v.pub_key {
                    Some(pk) => match &pk.sum {
                        Some(cometbft_proto::crypto::v1::public_key::Sum::Ed25519(key)) => {
                            cometbft::PublicKey::from_raw_ed25519(&key).unwrap()
                        }
                        _ => panic!("Unsupported public key type"),
                    },
                    None => panic!("Validator has no public key"),
                };
                cometbft::validator::Info::new(
                    pub_key,
                    cometbft::vote::Power::try_from(v.power).unwrap(),
                )
            })
            .collect();

        let consensus_params = Params {
            block: Size {
                max_bytes: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.block.as_ref())
                    .map(|b| b.max_bytes as i64)
                    .unwrap_or(22020096) as u64,
                max_gas: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.block.as_ref())
                    .map(|b| b.max_gas as i64)
                    .unwrap_or(-1),
                time_iota_ms: 0,
            },
            evidence: evidence::Params {
                max_age_num_blocks: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.evidence.as_ref())
                    .map(|e| e.max_age_num_blocks as u64)
                    .unwrap_or(100000),
                max_age_duration: Duration(std::time::Duration::from_secs(
                    init_chain_resp
                        .consensus_params
                        .as_ref()
                        .and_then(|p| p.evidence.as_ref())
                        .and_then(|e| e.max_age_duration.as_ref())
                        .map(|d| d.seconds as u64)
                        .unwrap_or(48 * 3600),
                )),
                max_bytes: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.evidence.as_ref())
                    .map(|e| e.max_bytes as i64)
                    .unwrap_or(1048576),
            },
            validator: ValidatorParams {
                pub_key_types: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.validator.as_ref())
                    .map(|v| {
                        v.pub_key_types
                            .iter()
                            .filter_map(|t| public_key::Algorithm::from_str(t).ok())
                            .collect()
                    })
                    .unwrap_or_else(|| vec![public_key::Algorithm::Ed25519]),
            },
            version: Some(VersionParams {
                app: init_chain_resp
                    .consensus_params
                    .as_ref()
                    .and_then(|p| p.version.as_ref())
                    .map(|v| v.app)
                    .unwrap_or(0),
            }),
            abci: AbciParams {
                vote_extensions_enable_height: None,
            },
        };

        // Store initial state
        state_store.save_initial_state(
            chain_id,
            validators,
            consensus_params,
            Height::from(0u32),
        )?;

        self.consensus = Some(consensus_state);
        self.abci_client = None; // Client is moved to consensus state
        self.block_store = Some(chain_store);
        self.state_store = Some(state_store);
        self.validator_key = Some(validator_key);

        Ok(())
    }

    /// Perform handshake with ABCI app
    pub async fn handshake(&mut self) -> Result<(), CometError> {
        // Get ABCI client
        let client = self
            .abci_client
            .as_mut()
            .ok_or_else(|| CometError::invalid_key("ABCI client not initialized".to_string()))?;

        // Get app info
        let info_req = InfoRequest {
            version: ABCI_VERSION.to_string(),
            block_version: BLOCK_PROTOCOL_VERSION,
            p2p_version: P2P_PROTOCOL_VERSION,
            abci_version: ABCI_VERSION.to_string(),
        };

        let info_resp = client.info(&info_req).await?;
        println!("Handshake Info Response: {:?}", info_resp);

        Ok(())
    }

    /// Run consensus engine
    pub async fn run_consensus(&mut self) -> Result<(), CometError> {
        let consensus = self
            .consensus
            .as_mut()
            .ok_or_else(|| CometError::protocol("consensus not initialized".to_string()))?;

        // Start consensus state
        consensus
            .start()
            .await
            .map_err(|e| CometError::protocol(e.to_string()))?;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), CometError> {
        // TODO: Implement graceful shutdown
        Ok(())
    }
}
