use thiserror::Error;

#[derive(Debug, Error)]
pub enum PclError {
    #[error("collector '{0}' not found in registry")]
    CollectorNotFound(String),
    #[error("collector '{name}' failed: {source}")]
    CollectorFailed {
        name: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("directory not initialized: {0}")]
    NotInitialized(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}
