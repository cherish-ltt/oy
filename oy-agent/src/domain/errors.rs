use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("AI error: {0}")]
    AiError(#[from] oy_ai::AiError),

    #[error("Max iterations reached")]
    MaxIterationsReached,

    #[error("Tool execution error: {0}")]
    ToolExecutionError(String),

    #[error("Chat Message error: {0}")]
    ChatMessageError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Session persistence error: {0}")]
    SessionPersistenceError(String),

    #[error("File Err - PathIsNotFile: {0}")]
    PathIsNotFile(String),

    #[error("Uuid error: {0}")]
    UuidError(#[from] uuid::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_error_display() {
        let err = AgentError::AiError(oy_ai::AiError::ApiError("timeout".into()));
        assert!(format!("{}", err).contains("timeout"));
    }

    #[test]
    fn test_max_iterations_display() {
        let err = AgentError::MaxIterationsReached;
        assert_eq!(format!("{}", err), "Max iterations reached");
    }

    #[test]
    fn test_tool_execution_error_display() {
        let err = AgentError::ToolExecutionError("tool not found".into());
        assert_eq!(format!("{}", err), "Tool execution error: tool not found");
    }

    #[test]
    fn test_serialization_error() {
        let inner = serde_json::from_str::<i32>("x").unwrap_err();
        let err = AgentError::SerializationError(inner);
        assert!(format!("{}", err).starts_with("Serialization error: "));
    }

    #[test]
    fn test_session_persistence_error() {
        let err = AgentError::SessionPersistenceError("disk full".into());
        assert_eq!(format!("{}", err), "Session persistence error: disk full");
    }

    #[test]
    fn test_path_is_not_file() {
        let err = AgentError::PathIsNotFile("/dev/null".into());
        assert_eq!(format!("{}", err), "File Err - PathIsNotFile: /dev/null");
    }

    #[test]
    fn test_uuid_error() {
        let inner = uuid::Uuid::parse_str("not-a-uuid").unwrap_err();
        let err = AgentError::UuidError(inner);
        assert!(format!("{}", err).starts_with("Uuid error: "));
    }
}
