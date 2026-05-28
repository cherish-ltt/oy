use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("Parse error: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("Max retries exceeded")]
    MaxRetriesExceeded,
}
