use thiserror::Error;

pub type Result<T> = std::result::Result<T, ShrLinkError>;

#[derive(Error, Debug)]
pub enum ShrLinkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Compression error: {0}")]
    Compression(String),
    
    #[error("Network error: {0}")]
    Network(String),
    
    #[error("P2P error: {0}")]
    P2P(String),
    
    #[error("HTTP error: {0}")]
    Http(String),
    
    #[error("Configuration error: {0}")]
    Config(#[from] toml::de::Error),
    
    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    
    #[error("Timeout: {0}")]
    Timeout(String),
    
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}
