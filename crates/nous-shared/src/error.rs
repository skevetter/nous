#[derive(Debug, thiserror::Error)]
pub enum NousError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("encryption error: {0}")]
    Encryption(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, NousError>;
