use cometbft::abci::{request, response, Request, Response};
use cometbft::block::Height;
use cometbft::hash::AppHash;
use cometbft_proto::abci::v1beta1::{Request as RawRequest, Response as RawResponse};
use cometbft_proto::Error as ProtoError;
use prost::Message;
use std::io;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

pub struct Engine {
    pub stream: TcpStream,
}

impl Engine {
    pub fn connect(addr: SocketAddr) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self { stream })
    }

    pub fn send(&mut self, req: Request) -> Result<Response, Error> {
        // Convert to protobuf
        let proto_req = match req {
            Request::Info(info) => RawRequest {
                value: Some(cometbft_proto::abci::v1beta1::request::Value::Info(
                    cometbft_proto::abci::v1beta1::RequestInfo {
                        version: info.version,
                        block_version: info.block_version,
                        p2p_version: info.p2p_version,
                    },
                )),
            },
            // Add other request types here
            _ => return Err(Error::UnsupportedRequest),
        };

        let mut proto_bytes = Vec::new();
        proto_req
            .encode(&mut proto_bytes)
            .map_err(|e| Error::Encode(e.to_string()))?;

        // Write length prefix and request bytes
        self.stream.write_all(&proto_bytes.len().to_be_bytes())?;
        self.stream.write_all(&proto_bytes)?;

        // Read response length
        let mut len_buf = [0u8; 8];
        self.stream.read_exact(&mut len_buf)?;
        let resp_len = u64::from_be_bytes(len_buf);

        // Read response bytes
        let mut resp_buf = vec![0u8; resp_len as usize];
        self.stream.read_exact(&mut resp_buf)?;

        // Parse response
        let proto_resp =
            RawResponse::decode(&resp_buf[..]).map_err(|e| Error::Decode(e.to_string()))?;

        // Convert response
        match proto_resp.value {
            Some(cometbft_proto::abci::v1beta1::response::Value::Info(info)) => {
                Ok(Response::Info(response::Info {
                    data: info.data,
                    version: info.version,
                    app_version: info.app_version,
                    last_block_height: Height::try_from(info.last_block_height)
                        .map_err(|e| Error::Conversion(e.to_string()))?,
                    last_block_app_hash: AppHash::try_from(info.last_block_app_hash)
                        .map_err(|e| Error::Conversion(e.to_string()))?,
                }))
            }
            // Add other response types here
            _ => Err(Error::UnexpectedResponse),
        }
    }

    pub fn info(&mut self) -> Result<response::Info, Error> {
        let info_req = request::Info {
            version: env!("CARGO_PKG_VERSION").to_string(),
            block_version: 11,                  // CometBFT block protocol version
            p2p_version: 8,                     // CometBFT p2p protocol version
            abci_version: "0.17.0".to_string(), // ABCI protocol version
        };

        let req = Request::Info(info_req);

        self.send(req).and_then(|resp| match resp {
            Response::Info(info) => Ok(info),
            _ => Err(Error::UnexpectedResponse),
        })
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Proto(ProtoError),
    Encode(String),
    Decode(String),
    Conversion(String),
    UnexpectedResponse,
    UnsupportedRequest,
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<ProtoError> for Error {
    fn from(err: ProtoError) -> Self {
        Error::Proto(err)
    }
}
