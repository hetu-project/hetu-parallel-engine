use dirs;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(rename = "base")]
    pub base: BaseConfig,
    pub rpc: RpcConfig,
    pub consensus: ConsensusConfig,
    pub max_tx_bytes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base: BaseConfig::default(),
            rpc: RpcConfig::default(),
            consensus: ConsensusConfig::default(),
            max_tx_bytes: 1048576, // 1MB default
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BaseConfig {
    pub root_dir: PathBuf,
    pub proxy_app: String,
    pub moniker: String,
    pub genesis_file: String,
    pub priv_validator_key_file: String,
    pub node_key_file: String,
    pub chain_id: String,
    pub db_backend: String,
}

impl Default for BaseConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let root_dir = home.join(".cometbft");
        Self {
            root_dir: root_dir.clone(),
            proxy_app: "tcp://127.0.0.1:26658".to_string(),
            moniker: "evmosd".to_string(),
            genesis_file: root_dir
                .join("/tmp/genesis.json")
                .to_str()
                .unwrap()
                .to_string(),
            priv_validator_key_file: root_dir
                .join("/tmp/priv_validator_key.json")
                .to_str()
                .unwrap()
                .to_string(),
            node_key_file: root_dir
                .join("/tmp/node_key.json")
                .to_str()
                .unwrap()
                .to_string(),
            chain_id: "evmos_9002-1".to_string(),
            db_backend: "rocksdb".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RpcConfig {
    pub laddr: SocketAddr,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            laddr: "127.0.0.1:26657".parse().unwrap(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConsensusConfig {
    pub timeout_propose: Duration,
    pub timeout_propose_delta: Duration,
    pub timeout_commit: Duration,
    pub create_empty_blocks: bool,
    pub create_empty_blocks_interval: Duration,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            timeout_propose: Duration::from_secs(3),
            timeout_propose_delta: Duration::from_millis(500),
            timeout_commit: Duration::from_secs(1),
            create_empty_blocks: true,
            create_empty_blocks_interval: Duration::from_secs(0),
        }
    }
}
