use async_trait::async_trait;
use cometbft::error::Error;
use cometbft_proto::abci::v1::{
    CommitRequest, CommitResponse, EchoRequest, FinalizeBlockRequest, FinalizeBlockResponse,
    FlushRequest, InfoRequest, InfoResponse, InitChainRequest, InitChainResponse,
    PrepareProposalRequest, PrepareProposalResponse, ProcessProposalRequest,
    ProcessProposalResponse, Request, Response,
};
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;

#[async_trait]
pub trait AbciClient: Send {
    async fn connect(&mut self) -> Result<(), Error>;
    async fn info(&mut self, req: &InfoRequest) -> Result<InfoResponse, Error>;
    async fn init_chain(&mut self, req: &InitChainRequest) -> Result<InitChainResponse, Error>;
    async fn echo(&mut self, msg: &str) -> Result<String, Error>;
    async fn flush(&mut self) -> Result<(), Error>;
    async fn prepare_proposal(
        &mut self,
        req: &PrepareProposalRequest,
    ) -> Result<PrepareProposalResponse, Error>;
    async fn process_proposal(
        &mut self,
        req: &ProcessProposalRequest,
    ) -> Result<ProcessProposalResponse, Error>;
    async fn finalize_block(
        &mut self,
        req: &FinalizeBlockRequest,
    ) -> Result<FinalizeBlockResponse, Error>;
    async fn commit(&mut self, req: &CommitRequest) -> Result<CommitResponse, Error>;
}

// CometBFT version constants
const COMETBFT_VERSION: &str = "0.38.15";
const BLOCK_PROTOCOL_VERSION: u64 = 11;
const P2P_PROTOCOL_VERSION: u64 = 8;
const ABCI_VERSION: &str = "2.0.0";

/// Remote ABCI client that connects to an external application process
pub struct RemoteClient {
    addr: String,
    stream: Option<TcpStream>,
}

impl Clone for RemoteClient {
    fn clone(&self) -> Self {
        Self {
            addr: self.addr.clone(),
            stream: None, // Stream cannot be cloned, new connection will be established when needed
        }
    }
}

impl RemoteClient {
    pub fn new(addr: String) -> Self {
        Self { addr, stream: None }
    }

    async fn send_request(&mut self, request: &Request) -> Result<Response, Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::invalid_key("Not connected to ABCI app".to_string()))?;
        let mut writer = BufWriter::new(stream);

        // Write the request
        let request_bytes = request.encode_length_delimited_to_vec();
        writer
            .write_all(&request_bytes)
            .await
            .map_err(|e| Error::invalid_key(format!("Failed to write request: {}", e)))?;

        // Write a flush request
        let flush_request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::Flush(
                Default::default(),
            )),
        };
        let flush_bytes = flush_request.encode_length_delimited_to_vec();
        writer
            .write_all(&flush_bytes)
            .await
            .map_err(|e| Error::invalid_key(format!("Failed to write flush: {}", e)))?;
        writer
            .flush()
            .await
            .map_err(|e| Error::invalid_key(format!("Failed to flush writer: {}", e)))?;

        // Read response to original request
        let response = self.read_response().await?;

        // Read response to flush request
        let _flush_response = self.read_response().await?;

        Ok(response)
    }

    async fn read_response(&mut self) -> Result<Response, Error> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::invalid_key("Not connected to ABCI app".to_string()))?;

        // Read length-delimited response using protoio.NewDelimitedReader
        let mut length_buf = [0u8; 10]; // Max varint size is 10 bytes
        let mut length_pos = 0;

        // Read varint length byte by byte
        loop {
            let mut buf = [0u8; 1];
            stream
                .read_exact(&mut buf)
                .await
                .map_err(|e| Error::invalid_key(format!("Failed to read length byte: {}", e)))?;
            length_buf[length_pos] = buf[0];
            length_pos += 1;

            // Try to parse varint
            if let Ok(l) = binary_varint_decode(&length_buf[..length_pos]) {
                // Check length bounds
                if l > 1024 * 1024 {
                    // 1MB limit like Go
                    return Err(Error::invalid_key("Response too large".to_string()));
                }

                // Read exact message bytes
                let mut msg_buf = vec![0u8; l as usize];
                stream
                    .read_exact(&mut msg_buf)
                    .await
                    .map_err(|e| Error::invalid_key(format!("Failed to read message: {}", e)))?;

                // Unmarshal using prost
                return Response::decode(msg_buf.as_slice())
                    .map_err(|e| Error::invalid_key(format!("Failed to decode response: {}", e)));
            }

            // Varint too long
            if length_pos >= 10 {
                return Err(Error::invalid_key("Invalid varint length".to_string()));
            }
        }
    }
}

#[async_trait]
impl AbciClient for RemoteClient {
    async fn connect(&mut self) -> Result<(), Error> {
        let stream = TcpStream::connect(&self.addr)
            .await
            .map_err(|e| Error::invalid_key(format!("Failed to connect to ABCI app: {}", e)))?;
        self.stream = Some(stream);
        Ok(())
    }

    async fn info(&mut self, req: &InfoRequest) -> Result<InfoResponse, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::Info(req.clone())),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::Info(info)) => Ok(info),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn init_chain(&mut self, req: &InitChainRequest) -> Result<InitChainResponse, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::InitChain(
                req.clone(),
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::InitChain(init)) => Ok(init),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn echo(&mut self, msg: &str) -> Result<String, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::Echo(
                EchoRequest {
                    message: msg.to_string(),
                },
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::Echo(echo)) => Ok(echo.message),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn flush(&mut self) -> Result<(), Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::Flush(
                FlushRequest::default(),
            )),
        };
        let _response = self.send_request(&request).await?;
        Ok(())
    }

    async fn prepare_proposal(
        &mut self,
        req: &PrepareProposalRequest,
    ) -> Result<PrepareProposalResponse, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::PrepareProposal(
                req.clone(),
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::PrepareProposal(res)) => Ok(res),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn process_proposal(
        &mut self,
        req: &ProcessProposalRequest,
    ) -> Result<ProcessProposalResponse, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::ProcessProposal(
                req.clone(),
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::ProcessProposal(resp)) => Ok(resp),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn finalize_block(
        &mut self,
        req: &FinalizeBlockRequest,
    ) -> Result<FinalizeBlockResponse, Error> {
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::FinalizeBlock(
                req.clone(),
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::FinalizeBlock(resp)) => Ok(resp),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }

    async fn commit(&mut self, req: &CommitRequest) -> Result<CommitResponse, Error> {
        println!("Sending Commit request");
        let request = Request {
            value: Some(cometbft_proto::abci::v1::request::Value::Commit(
                req.clone(),
            )),
        };
        let response = self.send_request(&request).await?;
        match response.value {
            Some(cometbft_proto::abci::v1::response::Value::Commit(commit)) => Ok(commit),
            _ => Err(Error::invalid_key("Unexpected response type".to_string())),
        }
    }
}

fn binary_varint_decode(buf: &[u8]) -> Result<u64, Error> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;

    for &byte in buf {
        if shift >= 64 {
            return Err(Error::invalid_key("Varint is too long".to_string()));
        }

        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
    }

    Err(Error::invalid_key("Unexpected EOF".to_string()))
}
