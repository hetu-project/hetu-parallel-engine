use cometbft::Error as CometError;
use serde_json;
use std::error::Error as StdError;
use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    InvalidKey(String),
    InvalidBlock(String),
    Database(String),
    NoValidatorsFound,
    NoState,
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidKey(msg) => write!(f, "Invalid key: {}", msg),
            Error::InvalidBlock(msg) => write!(f, "Invalid block: {}", msg),
            Error::Database(msg) => write!(f, "Database error: {}", msg),
            Error::NoValidatorsFound => write!(f, "No validators found"),
            Error::NoState => write!(f, "No state found"),
            Error::Other(err) => write!(f, "Other error: {}", err),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

impl Error {
    pub fn invalid_key<S: Into<String>>(msg: S) -> Self {
        Error::InvalidKey(msg.into())
    }

    pub fn database<S: Into<String>>(msg: S) -> Self {
        Error::Database(msg.into())
    }

    pub fn invalid_block<S: Into<String>>(msg: S) -> Self {
        Error::InvalidBlock(msg.into())
    }
}

impl From<Error> for CometError {
    fn from(err: Error) -> Self {
        match err {
            Error::InvalidKey(msg) => CometError::invalid_key(msg),
            Error::InvalidBlock(msg) => CometError::invalid_block(msg),
            Error::Database(msg) => CometError::invalid_block(msg),
            Error::NoValidatorsFound => {
                CometError::invalid_block("No validators found".to_string())
            }
            Error::NoState => CometError::invalid_block("No state found".to_string()),
            Error::Other(err) => CometError::invalid_block(err),
        }
    }
}

impl From<CometError> for Error {
    fn from(err: CometError) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Other(err.to_string())
    }
}
