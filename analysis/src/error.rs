use thiserror::Error;

#[derive(Error, Debug)]
pub enum AnalysisError {
    #[error("Failed to read BWS file: {0}")]
    BwsReadError(#[from] bridge_parsers::BridgeError),

    #[error("Failed to parse PBN file: {0}")]
    PbnParseError(String),

    #[error("Player not found: {0}")]
    PlayerNotFound(String),

    #[error("No results found for board {0}")]
    NoResultsForBoard(u32),

    #[error("Invalid contract string: {0}")]
    InvalidContract(String),

    #[error("Missing required data: {0}")]
    MissingData(String),

    #[error("Invalid input data: {0}")]
    InvalidData(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AnalysisError>;
