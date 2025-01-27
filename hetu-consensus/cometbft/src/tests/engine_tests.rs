use std::net::TcpStream;
use std::io::{self, Read, Write};
use crate::engine;
use cometbft_proto::{Protobuf};
use cometbft_proto::abci::v1::{Request, Response};
use cometbft::abci::{request, response};
use cometbft::block;

fn read_length_prefixed(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut length_buf = [0u8; 4];
    stream.read_exact(&mut length_buf)?;
    let length = u32::from_be_bytes(length_buf) as usize;
    
    let mut data = vec![0u8; length];
    stream.read_exact(&mut data)?;
    Ok(data)
}

fn write_length_prefixed(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    let length = (data.len() as u32).to_be_bytes();
    stream.write_all(&length)?;
    stream.write_all(data)?;
    Ok(())
}

// #[tokio::test]
// async fn test_info() -> io::Result<()> {
//     let mut engine = engine::Engine::new("127.0.0.1:26658".parse().unwrap(), "test".to_string());
//
//     let info_response = engine.info()?;
//
//     // You can access various fields from the info response
//     println!("App version: {}", info_response.version);
//     println!("App name: {}", info_response.app_version);
//     println!("Last block height: {}", info_response.last_block_height);
//
//     Ok(())
// }
