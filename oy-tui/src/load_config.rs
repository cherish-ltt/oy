use std::env;
use std::fs;
use std::path::PathBuf;

use oy_agent::{
    infrastructure::tools::{
        ToolRegistry, bash::BashTool, edit::EditTool, read::ReadTool, write::WriteTool,
    },
    oy_ai::AiConfig,
};
use serde::{Deserialize, Serialize};

/// Configuration loaded from ~/.oy-ai-agent/config.toml
#[derive(Debug, Deserialize, Serialize, Default)]
pub struct GlobalTomlConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub theme: Option<String>,
    /// Reasoning effort level: "none", "low", "medium", "high", "xhigh"
    pub reasoning_effort: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".oy-ai-agent").join("config.toml"))
}

impl GlobalTomlConfig {
    /// Load config from ~/.oy-ai-agent/config.toml, returning defaults for missing fields.
    pub fn load() -> Option<Self> {
        let config_path = config_path()?;
        if !config_path.exists() {
            return None;
        }
        match fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or(None),
            Err(_) => None,
        }
    }

    /// Save config to ~/.oy-ai-agent/config.toml, merging with existing values.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("Cannot determine home directory")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        // Read existing config and merge
        let mut existing = Self::load().unwrap_or_default();
        if self.api_key.is_some() {
            existing.api_key = self.api_key.clone();
        }
        if self.base_url.is_some() {
            existing.base_url = self.base_url.clone();
        }
        if self.model.is_some() {
            existing.model = self.model.clone();
        }
        if self.theme.is_some() {
            existing.theme = self.theme.clone();
        }
        if self.reasoning_effort.is_some() {
            existing.reasoning_effort = self.reasoning_effort.clone();
        }
        let toml_string =
            toml::to_string(&existing).map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(&path, toml_string).map_err(|e| format!("Failed to write config: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = GlobalTomlConfig::default();
        assert!(config.api_key.is_none());
        assert!(config.base_url.is_none());
        assert!(config.model.is_none());
        assert!(config.theme.is_none());
    }
}

pub fn build_provider_config(config: &GlobalTomlConfig) -> AiConfig {
    let api_key = config
        .api_key
        .clone()
        .or_else(|| env::var("OPENROUTER_API_KEY").ok())
        .unwrap_or_else(|| {
            eprintln!(
                "OPENROUTER_API_KEY is not set. Set it in ~/.oy-ai-agent/config.toml \
                 or the OPENROUTER_API_KEY environment variable."
            );
            std::process::exit(1);
        });

    let base_url = config
        .base_url
        .clone()
        .or_else(|| env::var("OPENROUTER_BASE_URL").ok())
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());

    let model = config
        .model
        .clone()
        .or_else(|| env::var("OPENCODE_MODEL").ok())
        .unwrap_or_else(|| "deepseek-v4-flash".to_string());

    let mut ai_config = AiConfig::new(base_url, api_key, model);

    // Pass reasoning_effort from config, defaulting to "high"
    let effort = config
        .reasoning_effort
        .clone()
        .or_else(|| Some("high".to_string()));
    ai_config = ai_config.with_reasoning_effort(effort);

    ai_config
}

/// Register the default set of tools (Read, Write, Bash).
pub fn register_default_tools(registry: &mut ToolRegistry) {
    registry.register(ReadTool);
    registry.register(WriteTool);
    registry.register(EditTool);
    registry.register(BashTool);
}
