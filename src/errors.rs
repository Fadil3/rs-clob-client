use alloy::primitives::ChainId;
use alloy::primitives::ruint::ParseError;
use alloy::signers::local::LocalSignerError;
use derive_builder::UninitializedFieldError;
use hmac::digest::InvalidLength;
use reqwest::{Method, StatusCode, header};
use thiserror::Error;

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Base64Decode(#[from] base64::DecodeError),
    #[error("request error: {0}")]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    InvalidHeaderValue(#[from] header::InvalidHeaderValue),
    #[error(transparent)]
    InvalidLength(#[from] InvalidLength),
    #[error(transparent)]
    LocalSigner(#[from] LocalSignerError),
    #[error("missing contract config for chain id {0} with neg_risk = {1}")]
    MissingContractConfig(ChainId, bool),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Signer(#[from] alloy::signers::Error),
    #[error("error({status_code}) making {method} call to {path} with {message}")]
    Status {
        status_code: StatusCode,
        method: Method,
        path: String,
        message: String,
    },
    #[error("synchronization error: multiple threads are attempting to log in or log out")]
    Synchronization,
    #[error(transparent)]
    UninitializedField(#[from] UninitializedFieldError),
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error(transparent)]
    U256Parse(#[from] ParseError),
    #[error("Validation error: {0}")]
    Validation(String),
}
