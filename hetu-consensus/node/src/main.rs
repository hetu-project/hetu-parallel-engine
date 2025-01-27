// Copyright(C) Facebook, Inc. and its affiliates.

use alloy_primitives::private::serde;
use anyhow::{Context, Result};
use clap::{crate_name, crate_version, App, AppSettings, ArgMatches, SubCommand};
use config::{Committee, Export, Import, KeyPair, Parameters, WorkerId};
use consensus::Consensus;
use crypto::Digest;
use env_logger::Env;
use eyre::{bail, eyre};
use futures::{SinkExt, StreamExt};
use log::{error, info};
use primary::{Certificate, Primary};
use prost::Message;
use rocksdb::{Options, DB};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use store::Store;
use tokio::{
    net::TcpListener,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    time::sleep,
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use worker::Worker;

/// The default channel capacity.
pub const CHANNEL_CAPACITY: usize = 1_000;

pub mod proto {
    pub mod batch {
        include!(concat!(env!("OUT_DIR"), "/batch.rs"));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .about("A research implementation of Narwhal and Tusk.")
        .args_from_usage("-v... 'Sets the level of verbosity'")
        .subcommand(
            SubCommand::with_name("generate_keys")
                .about("Print a fresh key pair to file")
                .args_from_usage("--filename=<FILE> 'The file where to print the new key pair'"),
        )
        .subcommand(
            SubCommand::with_name("run")
                .about("Run a node")
                .args_from_usage("--keys=<FILE> 'The file containing the node keys'")
                .args_from_usage("--committee=<FILE> 'The file containing committee information'")
                .args_from_usage("--parameters=[FILE] 'The file containing the node parameters'")
                .args_from_usage("--store=<PATH> 'The path where to create the data store'")
                .subcommand(SubCommand::with_name("primary").about("Run a single primary"))
                .subcommand(
                    SubCommand::with_name("worker")
                        .about("Run a single worker")
                        .args_from_usage("--id=<INT> 'The worker id'"),
                )
                .setting(AppSettings::SubcommandRequiredElseHelp),
        )
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .get_matches();

    let log_level = match matches.occurrences_of("v") {
        0 => "error",
        1 => "warn",
        2 => "info",
        3 => "debug",
        _ => "trace",
    };
    let mut logger = env_logger::Builder::from_env(Env::default().default_filter_or(log_level));
    #[cfg(feature = "benchmark")]
    logger.format_timestamp_millis();
    logger.init();

    match matches.subcommand() {
        ("generate_keys", Some(sub_matches)) => KeyPair::new()
            .export(sub_matches.value_of("filename").unwrap())
            .context("Failed to generate key pair")?,
        ("run", Some(sub_matches)) => run(sub_matches).await?,
        _ => unreachable!(),
    }
    Ok(())
}

// Runs either a worker or a primary.
async fn run(matches: &ArgMatches<'_>) -> Result<()> {
    let key_file = matches.value_of("keys").unwrap();
    let committee_file = matches.value_of("committee").unwrap();
    let parameters_file = matches.value_of("parameters");
    let store_path = matches.value_of("store").unwrap();

    // Read the committee and node's keypair from file.
    let keypair = KeyPair::import(key_file).context("Failed to load the node's keypair")?;
    let committee =
        Committee::import(committee_file).context("Failed to load the committee information")?;

    // Load default parameters if none are specified.
    let parameters = match parameters_file {
        Some(filename) => {
            Parameters::import(filename).context("Failed to load the node's parameters")?
        }
        None => Parameters::default(),
    };

    // Make the data store.
    let store = Store::new(store_path).context("Failed to create a store")?;

    // Channels the sequence of certificates.
    let (tx_output, rx_output) = channel(CHANNEL_CAPACITY);

    // Check whether to run a primary, a worker, or an entire authority.
    match matches.subcommand() {
        // Spawn the primary and consensus core.
        ("primary", _) => {
            let (tx_new_certificates, rx_new_certificates) = channel(CHANNEL_CAPACITY);
            let (tx_feedback, rx_feedback) = channel(CHANNEL_CAPACITY);
            Primary::spawn(
                keypair,
                committee.clone(),
                parameters.clone(),
                store,
                /* tx_consensus */ tx_new_certificates,
                /* rx_consensus */ rx_feedback,
            );
            Consensus::spawn(
                committee,
                parameters.gc_depth,
                /* rx_primary */ rx_new_certificates,
                /* tx_primary */ tx_feedback,
                tx_output,
            );
        }

        // Spawn a single worker.
        ("worker", Some(sub_matches)) => {
            let id = sub_matches
                .value_of("id")
                .unwrap()
                .parse::<WorkerId>()
                .context("The worker id must be a positive integer")?;
            Worker::spawn(keypair.name, id, committee, parameters, store);
        }
        _ => unreachable!(),
    }

    // Analyze the consensus' output.
    analyze(rx_output, store_path.parse()?).await;

    // If this expression is reached, the program ends and all other tasks stop.
    unreachable!();
}

/// Receives an ordered list of certificates and apply any application-specific logic.
async fn analyze(mut rx_output: Receiver<Certificate>, store_path: String) {
    // Channel for passing batch transactions from certificate processor to TCP handler
    let (tx_transactions, rx_transactions) = channel::<(Vec<Vec<u8>>, u64)>(CHANNEL_CAPACITY);

    // Channel for managing client connections
    let (tx_broadcast, mut rx_broadcast) = channel::<(Vec<u8>, usize)>(CHANNEL_CAPACITY);
    let active_connections = Arc::new(tokio::sync::Mutex::new(HashMap::<
        SocketAddr,
        Sender<Vec<u8>>,
    >::new()));

    // Spawn certificate processor
    let store_path_clone = store_path.clone();
    tokio::spawn(async move {
        while let Some(certificate) = rx_output.recv().await {
            if let Err(e) =
                process_certificate(certificate, store_path_clone.clone(), &tx_transactions).await
            {
                error!("Failed to process certificate: {}", e);
            }
        }
        info!("Certificate processor ended");
    });

    // Spawn broadcaster task
    let active_connections_clone = active_connections.clone();
    tokio::spawn(async move {
        let mut rx_transactions = rx_transactions;
        while let Some((txs, round)) = rx_transactions.recv().await {
            // Convert to protobuf message
            let txs_len = txs.len();
            let proto_batch = proto::batch::BatchTransactions {
                transactions: txs,
                round,
            };

            // Serialize the message
            let bytes = proto_batch.encode_to_vec();

            // Get the current number of connections
            let conn_count = active_connections_clone.lock().await.len();
            if conn_count > 0 {
                if let Err(e) = tx_broadcast.send((bytes, conn_count)).await {
                    error!("Failed to send to broadcast channel: {}", e);
                }
                info!(
                    "Broadcasting batch with {} transactions to {} clients",
                    txs_len, conn_count
                );
            }
        }
    });

    // Spawn broadcast handler
    let active_connections_clone = active_connections.clone();
    tokio::spawn(async move {
        while let Some((bytes, expected_count)) = rx_broadcast.recv().await {
            let mut connections = active_connections_clone.lock().await;
            if connections.len() != expected_count {
                // Skip this broadcast as the number of connections has changed
                continue;
            }

            for (addr, tx) in connections.iter() {
                info!("Sending batch to client {}", addr);
                if let Err(e) = tx.send(bytes.clone()).await {
                    error!("Failed to send to client {}: {}", addr, e);
                }
            }
        }
    });

    // TCP connection handler
    let port = find_available_port(20001)
        .await
        .expect("Failed to find available port");
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
        .await
        .expect("Failed to bind to address");
    info!(
        "Listening for Narwhal clients on {}",
        listener.local_addr().unwrap()
    );

    // Accept and handle new connections
    loop {
        info!("Waiting for Narwhal client connection...");
        let (stream, addr) = listener
            .accept()
            .await
            .expect("Failed to accept connection");
        info!("Accepted connection from {}", addr);

        let (write, mut read) = Framed::new(stream, LengthDelimitedCodec::new()).split();
        let (tx_client, mut rx_client) = channel::<Vec<u8>>(CHANNEL_CAPACITY);

        // Store the client's sender
        active_connections.lock().await.insert(addr, tx_client);

        // Spawn client writer task
        let active_connections_clone = active_connections.clone();
        tokio::spawn(async move {
            let mut write = write;
            while let Some(bytes) = rx_client.recv().await {
                if let Err(e) = write.send(bytes.into()).await {
                    error!("Failed to send to client {}: {}", addr, e);
                    break;
                }
            }
            // Remove connection when writer task ends
            active_connections_clone.lock().await.remove(&addr);
            info!("Client {} writer task ended", addr);
        });

        // Spawn client reader task
        let active_connections_clone = active_connections.clone();
        tokio::spawn(async move {
            while read.next().await.is_some() {
                // Keep reading to detect disconnection
            }
            // Remove connection when reader detects disconnection
            active_connections_clone.lock().await.remove(&addr);
            info!("Client {} disconnected", addr);
        });
    }
}

async fn find_available_port(start_port: u16) -> std::io::Result<u16> {
    let mut port = start_port;
    loop {
        match TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            Ok(_) => return Ok(port),
            Err(_) => {
                port += 1;
                if port == 0 {
                    // Handle port number overflow
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        "No available ports found",
                    ));
                }
            }
        }
    }
}

async fn process_certificate(
    certificate: Certificate,
    store_path: String,
    tx_transactions: &Sender<(Vec<Vec<u8>>, u64)>,
) -> eyre::Result<()> {
    // Process each batch in the certificate
    for (digest, worker_id) in certificate.header.payload {
        match reconstruct_batch(digest, worker_id, store_path.clone()).await {
            Ok(batch) => match bincode::deserialize(&batch) {
                Ok(WorkerMessage::Batch(txs)) => {
                    if !txs.is_empty() {
                        if let Err(e) = tx_transactions.send((txs, certificate.header.round)).await
                        {
                            error!("Failed to send batch transactions: {}", e);
                            return Err(eyre::eyre!("Channel closed"));
                        }
                    }
                }
                _ => eyre::bail!("Unrecognized message format"),
            },
            Err(e) => {
                error!("Failed to reconstruct batch: {}", e);
                continue;
            }
        }
    }

    Ok(())
}

async fn reconstruct_batch(
    digest: Digest,
    worker_id: u32,
    store_path: String,
) -> eyre::Result<Vec<u8>> {
    let max_attempts = 3;
    let backoff_ms = 500;
    let db_path = format!("{}-{}", store_path, worker_id);
    // Open the database to each worker
    let db = rocksdb::DB::open_for_read_only(&rocksdb::Options::default(), db_path, true)?;

    for attempt in 0..max_attempts {
        // Query the db
        let key = digest.to_vec();
        match db.get(&key)? {
            Some(res) => return Ok(res),
            None if attempt < max_attempts - 1 => {
                println!(
                    "digest {} not found, retrying in {}ms",
                    digest,
                    backoff_ms * (attempt + 1)
                );
                sleep(Duration::from_millis(backoff_ms * (attempt + 1))).await;
                continue;
            }
            None => eyre::bail!(
                "digest {} not found after {} attempts",
                digest,
                max_attempts
            ),
        }
    }
    unreachable!()
}

pub type Transaction = Vec<u8>;
pub type Batch = Vec<Transaction>;
#[derive(serde::Deserialize)]
pub enum WorkerMessage {
    Batch(Batch),
}
