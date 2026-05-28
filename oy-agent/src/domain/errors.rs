use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("AI error: {0}")]
    AiError(#[from] oy_ai::AiError),

    #[error("Max iterations reached")]
    MaxIterationsReached,

    #[error("Tool execution error: {0}")]
    ToolExecutionError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Session persistence error: {0}")]
    SessionPersistenceError(String),

    #[error("File Err - PathIsNotFile: {0}")]
    PathIsNotFile(String),

    #[error("Uuid error: {0}")]
    UuidError(#[from] uuid::Error),
}
