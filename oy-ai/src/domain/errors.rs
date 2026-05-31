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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_display() {
        let err = AiError::ApiError("rate limited".into());
        assert_eq!(format!("{}", err), "API error: rate limited");
    }

    #[test]
    fn test_parse_error_display() {
        let inner = serde_json::from_str::<i32>("not-a-number").unwrap_err();
        let err = AiError::ParseError(inner);
        let display = format!("{}", err);
        assert!(display.starts_with("Parse error: "));
    }

    #[test]
    fn test_config_error_display() {
        let err = AiError::ConfigError("missing API key".into());
        assert_eq!(format!("{}", err), "Config error: missing API key");
    }

    #[test]
    fn test_max_retries_display() {
        let err = AiError::MaxRetriesExceeded;
        assert_eq!(format!("{}", err), "Max retries exceeded");
    }

    #[test]
    fn test_error_is_debug() {
        let err = AiError::ApiError("test".into());
        assert!(format!("{:?}", err).contains("test"));
    }
}
