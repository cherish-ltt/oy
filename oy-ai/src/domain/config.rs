use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://opencode.ai/zen/go/v1/chat/completions".to_string(),
            api_key: String::new(),
            model: "deepseek-v4-flash".to_string(),
        }
    }
}

impl AiConfig {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_config_default_model() {
        let config = AiConfig::default();
        assert_eq!(config.model, "deepseek-v4-flash");
        assert_eq!(
            config.base_url,
            "https://opencode.ai/zen/go/v1/chat/completions"
        );
    }
}
