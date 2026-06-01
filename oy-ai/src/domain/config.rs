use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Reasoning effort level: "none", "low", "medium", "high", "xhigh"
    /// Defaults to "high" if None.
    pub reasoning_effort: Option<String>,
    /// Maximum context capacity in tokens (e.g. 200000). Used for UI display.
    pub context_capacity: Option<u64>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://opencode.ai/zen/go/v1/chat/completions".to_string(),
            api_key: String::new(),
            model: "deepseek-v4-flash".to_string(),
            reasoning_effort: Some("high".to_string()),
            context_capacity: Some(200_000),
        }
    }
}

impl AiConfig {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
            reasoning_effort: Some("high".to_string()),
            context_capacity: Some(200_000),
        }
    }

    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    pub fn with_context_capacity(mut self, capacity: Option<u64>) -> Self {
        self.context_capacity = capacity;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_config_defaults() {
        let config = AiConfig::default();
        assert_eq!(config.model, "deepseek-v4-flash");
        assert_eq!(
            config.base_url,
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
        assert_eq!(config.reasoning_effort, Some("high".to_string()));
        assert_eq!(config.context_capacity, Some(200_000));
    }

    #[test]
    fn test_ai_config_new() {
        let config = AiConfig::new(
            "https://example.com/api".into(),
            "sk-test-key".into(),
            "gpt-4".into(),
        );
        assert_eq!(config.base_url, "https://example.com/api");
        assert_eq!(config.api_key, "sk-test-key");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.reasoning_effort, Some("high".to_string()));
        assert_eq!(config.context_capacity, Some(200_000));
    }

    #[test]
    fn test_ai_config_with_reasoning_effort() {
        let config = AiConfig::new("url".into(), "key".into(), "m".into())
            .with_reasoning_effort(Some("low".into()));
        assert_eq!(config.reasoning_effort, Some("low".to_string()));
    }

    #[test]
    fn test_ai_config_with_reasoning_effort_none() {
        let config =
            AiConfig::new("url".into(), "key".into(), "m".into()).with_reasoning_effort(None);
        assert_eq!(config.reasoning_effort, None);
    }

    #[test]
    fn test_ai_config_with_context_capacity() {
        let config = AiConfig::new("url".into(), "key".into(), "m".into())
            .with_context_capacity(Some(128_000));
        assert_eq!(config.context_capacity, Some(128_000));
    }

    #[test]
    fn test_ai_config_debug() {
        let config = AiConfig::new("http://localhost".into(), "key123".into(), "m".into());
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("http://localhost"));
        assert!(debug_str.contains("key123"));
        assert!(debug_str.contains("m"));
    }
}
