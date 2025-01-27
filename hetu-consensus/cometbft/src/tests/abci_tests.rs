use crate::proxyapp::{RemoteClient, AbciClient};
use cometbft_proto::abci::v1::{InfoRequest, InitChainRequest};
use cometbft_proto::google::protobuf::{Timestamp, Duration};
use cometbft_proto::types::v1::{BlockParams, ConsensusParams, EvidenceParams, ValidatorParams, VersionParams};
use std::time::{SystemTime, UNIX_EPOCH};
use std::str::FromStr;
use std::net::SocketAddr;
use std::fs;
use std::path::PathBuf;
use cometbft::public_key::Algorithm;

use crate::genesis::GenesisDoc;

#[tokio::test]
async fn test_abci() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new ABCI client
    let addr = SocketAddr::from_str("127.0.0.1:26658")?;
    let mut client = RemoteClient::new(addr.to_string());
    client.connect().await?;

    // Load genesis file
    let genesis_path = PathBuf::from("/Users/bufrr/github/hetu-hub-consensus/genesis.json");
    let genesis_doc = if let Ok(contents) = fs::read_to_string(&genesis_path) {
        let doc: GenesisDoc = serde_json::from_str(&contents)?;
        Some(doc)
    } else {
        return Err("Genesis file not found".into());
    };

    // First query ABCI Info
    let info_req = InfoRequest {
        version: crate::cometbft::COMETBFT_VERSION.to_string(),
        block_version: crate::cometbft::BLOCK_PROTOCOL_VERSION,
        p2p_version: crate::cometbft::P2P_PROTOCOL_VERSION,
        abci_version: crate::cometbft::ABCI_VERSION.to_string(),
    };

    let info_resp = client.info(&info_req).await?;
    println!("ABCI Info Response: {:?}", info_resp);

    // Use chain ID from genesis file
    let chain_id = genesis_doc.as_ref().unwrap().chain_id.clone();

    // Get genesis time from genesis file
    let genesis_time = {
        let unix_time = std::time::UNIX_EPOCH +
            std::time::Duration::from_secs(genesis_doc.as_ref().unwrap().genesis_time.unix_timestamp() as u64);
        unix_time.duration_since(UNIX_EPOCH).unwrap()
    };

    // Convert consensus params from genesis file
    let consensus_params = genesis_doc.as_ref().unwrap().consensus.params.clone();

    // Convert validators to ABCI format from genesis
    // let validators = genesis_doc.as_ref().unwrap().validators.iter().map(|v| ValidatorUpdate {
    //     pub_key: Some(ProtoPublicKey {
    //         sum: Some(match &v.pub_key {
    //             cometbft::public_key::PublicKey::Ed25519(key) => {
    //                 ProtoPublicKeySum::Ed25519(key.as_bytes().to_vec())
    //             }
    //             _ => panic!("Unsupported public key type"),
    //         }),
    //     }),
    //     power: v.power.value() as i64,
    // }).collect();

    // Get app state from genesis
    let app_state_bytes = serde_json::to_vec(&genesis_doc.as_ref().unwrap().app_state).unwrap_or_default();

    let init_chain_req = InitChainRequest {
        time: Some(Timestamp {
            seconds: genesis_time.as_secs() as i64,
            nanos: genesis_time.subsec_nanos() as i32,
        }),
        chain_id,
        consensus_params: Some(ConsensusParams {
            block: Some(BlockParams {
                max_bytes: consensus_params.block.max_bytes as i64,
                max_gas: consensus_params.block.max_gas as i64,
            }),
            evidence: Some(EvidenceParams {
                max_age_num_blocks: consensus_params.evidence.max_age_num_blocks as i64,
                max_age_duration: Some(Duration {
                    seconds: consensus_params.evidence.max_age_duration.0.as_secs() as i64,
                    nanos: consensus_params.evidence.max_age_duration.0.subsec_nanos() as i32,
                }),
                max_bytes: consensus_params.evidence.max_bytes as i64,
            }),
            validator: Some(ValidatorParams {
                pub_key_types: consensus_params.validator.pub_key_types.iter()
                    .map(|a| match a {
                        Algorithm::Ed25519 => "ed25519".to_string(),
                        _ => panic!("Unsupported pub key type"),
                    })
                    .collect(),
            }),
            version: Some(VersionParams {
                app: consensus_params.version.map(|v| v.app).unwrap_or(0),
            }),
            abci: None,
        }),
        validators: vec![],
        app_state_bytes: app_state_bytes.into(),
        initial_height: genesis_doc.as_ref().unwrap().initial_height,
    };

    // Try to init chain
    let init_chain_resp = match client.init_chain(&init_chain_req).await {
        Ok(resp) => resp,
        Err(e) => return Err(format!("Failed to init chain: {:?}", e).into()),
    };
    println!("InitChain Response: {:?}", init_chain_resp);

    // Query Info again to verify the changes
    let info_resp = client.info(&info_req).await?;
    println!("ABCI Info Response after InitChain: {:?}", info_resp);
    assert_eq!(info_resp.last_block_height, 0, "Height should still be 0 after InitChain");

    Ok(())
}
