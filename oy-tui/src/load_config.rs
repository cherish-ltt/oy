use std::env;

use oy_agent::{
    infrastructure::tools::{
        ToolRegistry, bash::BashTool, edit::EditTool, read::ReadTool, write::WriteTool,
    },
    oy_ai::AiConfig,
};
use serde::Deserialize;

/// Configuration loaded from ~/.oy-ai-agent/config.toml
#[derive(Debug, Deserialize, Default)]
pub struct GlobalTomlConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

impl GlobalTomlConfig {
    /// Load config from ~/.oy-ai-agent/config.toml, returning defaults for missing fields.
    pub fn load() -> Option<Self> {
        let home = dirs::home_dir()?;
        let config_path = home.join(".oy-ai-agent").join("config.toml");
        if !config_path.exists() {
            return None;
        }
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or(None),
            Err(_) => None,
        }
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

    AiConfig::new(base_url, api_key, model)
}

/// Register the default set of tools (Read, Write, Bash).
pub fn register_default_tools(registry: &mut ToolRegistry) {
    registry.register(ReadTool);
    registry.register(WriteTool);
    registry.register(EditTool);
    registry.register(BashTool);
}
