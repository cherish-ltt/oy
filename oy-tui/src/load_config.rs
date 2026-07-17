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
        // ── 原子写入 ──
        let tmp_path = path.with_extension("toml.tmp");

        // 步骤 1：写入临时文件
        fs::write(&tmp_path, &toml_string).map_err(|e| format!("Failed to write config: {}", e))?;

        // 步骤 2：原子替换原文件
        fs::rename(&tmp_path, &path).map_err(|e| {
            // 如果 rename 失败，尝试清理临时文件（忽略清理本身的错误）
            let _ = fs::remove_file(&tmp_path);
            format!("Failed to rename config file: {}", e)
        })?;

        // 步骤 3：设置文件权限（保持在 rename 之后）
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
    use std::sync::Mutex;

    /// Serializes tests that modify the HOME environment variable.
    static HOME_MUTEX: Mutex<()> = Mutex::new(());

    // ── 已有测试 ──
    #[test]
    fn test_config_default() {
        let config = GlobalTomlConfig::default();
        assert!(config.api_key.is_none());
        assert!(config.base_url.is_none());
        assert!(config.model.is_none());
        assert!(config.theme.is_none());
    }

    // ── 新增测试 ──

    /// Helper: temporarily override HOME for the duration of the test.
    /// Serialized via HOME_MUTEX to prevent race conditions between concurrent tests.
    fn with_temp_home(f: impl FnOnce()) {
        let _lock = HOME_MUTEX.lock().unwrap();
        let tmp = tempfile::TempDir::new().unwrap();
        let original_home = std::env::var("HOME").ok();
        // SAFETY: This is a test helper; HOME is restored after the test.
        unsafe {
            std::env::set_var("HOME", tmp.path());
        }
        // Ensure .oy-ai-agent dir exists
        std::fs::create_dir_all(tmp.path().join(".oy-ai-agent")).unwrap();
        f();
        // Restore HOME
        if let Some(home) = original_home {
            // SAFETY: Restoring original HOME after test completes.
            unsafe {
                std::env::set_var("HOME", home);
            }
        } else {
            // SAFETY: Removing HOME after test completes; it was not set before.
            unsafe {
                std::env::remove_var("HOME");
            }
        }
        // TempDir is dropped here, auto-cleanup
    }

    #[test]
    fn test_save_roundtrip() {
        with_temp_home(|| {
            let config = GlobalTomlConfig {
                api_key: Some("sk-test-123".to_string()),
                base_url: Some("https://custom.example.com".to_string()),
                model: Some("gpt-5".to_string()),
                theme: Some("dark".to_string()),
                reasoning_effort: Some("high".to_string()),
                context_capacity: Some(300000),
                read_claude_skills: Some(false),
            };

            assert!(config.save().is_ok());

            let loaded = GlobalTomlConfig::load().expect("Config should be loadable after save");
            assert_eq!(loaded.api_key, Some("sk-test-123".to_string()));
            assert_eq!(
                loaded.base_url,
                Some("https://custom.example.com".to_string())
            );
            assert_eq!(loaded.model, Some("gpt-5".to_string()));
            assert_eq!(loaded.theme, Some("dark".to_string()));
            assert_eq!(loaded.reasoning_effort, Some("high".to_string()));
            assert_eq!(loaded.context_capacity, Some(300000));
            assert_eq!(loaded.read_claude_skills, Some(false));
        });
    }

    #[test]
    fn test_save_partial_override() {
        with_temp_home(|| {
            // Save full config first
            let full = GlobalTomlConfig {
                api_key: Some("key1".to_string()),
                base_url: Some("url1".to_string()),
                ..Default::default()
            };
            full.save().unwrap();

            // Now save a partial override
            let partial = GlobalTomlConfig {
                api_key: Some("key2".to_string()),
                ..Default::default()
            };
            partial.save().unwrap();

            // Load and verify: api_key updated, base_url unchanged
            let loaded = GlobalTomlConfig::load().unwrap();
            assert_eq!(loaded.api_key, Some("key2".to_string()));
            assert_eq!(loaded.base_url, Some("url1".to_string()));
        });
    }

    #[test]
    fn test_atomic_save_no_temp_left() {
        with_temp_home(|| {
            let config = GlobalTomlConfig {
                api_key: Some("no-temp".to_string()),
                ..Default::default()
            };
            config.save().unwrap();

            // Verify no .tmp file remains in the config directory
            let home = std::env::var("HOME").unwrap();
            let config_dir = std::path::PathBuf::from(home).join(".oy-ai-agent");
            let entries: Vec<_> = std::fs::read_dir(&config_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect();
            assert!(
                !entries
                    .iter()
                    .any(|n| n.to_string_lossy().ends_with(".tmp"))
            );
        });
    }

    #[test]
    fn test_save_fails_when_no_home() {
        let _lock = HOME_MUTEX.lock().unwrap();
        // Temporarily remove HOME env variable to test error handling.
        let original_home = std::env::var("HOME").ok();

        // SAFETY: Test helper; HOME is restored after the check.
        unsafe {
            std::env::remove_var("HOME");
        }

        // If dirs::home_dir() still returns a value (e.g. from system user database
        // on Unix), this environment cannot trigger the "no home" path — skip.
        if dirs::home_dir().is_some() {
            // Restore HOME before skipping
            if let Some(home) = original_home {
                // SAFETY: Restoring original HOME after test completes.
                unsafe {
                    std::env::set_var("HOME", home);
                }
            }
            // Cannot test on this platform; skip silently.
            return;
        }

        let config = GlobalTomlConfig::default();
        let result = config.save();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot determine home directory")
        );

        // Restore HOME
        if let Some(home) = original_home {
            // SAFETY: Restoring original HOME after test completes.
            unsafe {
                std::env::set_var("HOME", home);
            }
        }
    }
}
