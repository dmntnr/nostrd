use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum RelayError {
    #[error("invalid event: {0}")]
    InvalidEvent(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("event expired")]
    EventExpired,

    #[error("event is protected")]
    EventProtected,

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("secp256k1 error: {0}")]
    Secp256k1(#[from] secp256k1::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, RelayError>;
