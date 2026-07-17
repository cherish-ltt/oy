use std::fs;
use std::path::PathBuf;

use oy_agent::infrastructure::file_permissions::set_file_permissions_600;
use oy_agent::{
    infrastructure::tools::{
        ToolRegistry, bash::BashTool, edit::EditTool, grep::GrepTool, read::ReadTool,
        uuid_tool::UuidTool, write::WriteTool,
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
    /// Maximum context capacity in tokens (e.g. 200000). Defaults to 200k if None.
    pub context_capacity: Option<u64>,
    /// Whether to read skills from ~/.claude/skills/ (default: true if None)
    pub read_claude_skills: Option<bool>,
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
        if self.context_capacity.is_some() {
            existing.context_capacity = self.context_capacity;
        }
        if self.read_claude_skills.is_some() {
            existing.read_claude_skills = self.read_claude_skills;
        }
        let toml_string =
            toml::to_string(&existing).map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(&path, toml_string).map_err(|e| format!("Failed to write config: {}", e))?;
        set_file_permissions_600(&path).map_err(|e| format!("Failed to set permissions: {}", e))?;
        Ok(())
    }
}

pub fn build_provider_config(config: &GlobalTomlConfig) -> Result<AiConfig, String> {
    let api_key = config.api_key.clone().ok_or_else(|| {
        "API key is not set. Set it in ~/.oy-ai-agent/config.toml:\n\n\
         [api_key]\n\
         api_key = \"sk-or-...\""
            .to_string()
    })?;

    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());

    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "deepseek-v4-flash".to_string());

    let mut ai_config = AiConfig::new(base_url, api_key, model);

    // Pass reasoning_effort from config, defaulting to "high"
    let effort = config
        .reasoning_effort
        .clone()
        .or_else(|| Some("high".to_string()));
    ai_config = ai_config.with_reasoning_effort(effort);

    // Pass context_capacity from config, defaulting to 200k
    let ctx = config.context_capacity.or(Some(200_000));
    ai_config = ai_config.with_context_capacity(ctx);

    Ok(ai_config)
}

/// Register the default set of tools (Read, Write, Bash, Edit, Uuid).
pub fn register_default_tools(registry: &mut ToolRegistry) {
    registry.register(ReadTool);
    registry.register(WriteTool);
    registry.register(EditTool);
    registry.register(BashTool);
    registry.register(UuidTool);
    registry.register(GrepTool);
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
