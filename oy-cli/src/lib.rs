use std::env;

use clap::Parser;
use oy_agent::Orchestrator;
use oy_agent::infrastructure::agents::main_agent::MainAgent;
use oy_agent::infrastructure::tools::edit::EditTool;
use oy_agent::infrastructure::tools::read::ReadTool;
use oy_agent::infrastructure::tools::write::WriteTool;
use oy_agent::infrastructure::tools::{ToolRegistry, bash::BashTool};
use oy_ai::{AiConfig, OpenCodeGoProvider};
use serde::Deserialize;

/// CLI arguments for oy-agent
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct CliArgs {
    /// Prompt to send to the agent (if omitted, launches the TUI)
    #[arg(short = 'p', long)]
    pub prompt: Option<String>,

    #[arg(short = 'm', long)]
    pub model: Option<String>,
}

/// Configuration loaded from ~/.oy-ai-agent/config.toml
#[derive(Debug, Deserialize, Default)]
pub struct CliConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

impl CliConfig {
    /// Load config from ~/.oy-ai-agent/config.toml, returning defaults for missing fields.
    pub fn load() -> Self {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => return Self::default(),
        };
        let config_path = home.join(".oy-ai-agent").join("config.toml");
        if !config_path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

/// Build an `AiConfig` by merging CLI args, config file, env vars, and defaults.
///
/// Priority (highest first):
///   1. CLI argument (`--model`)
///   2. Config file (`~/.oy-ai-agent/config.toml`)
///   3. Environment variable (`OPENROUTER_*`)
///   4. Hardcoded default
///
/// `api_key` is required: if none of the sources provide it, the process exits.
pub fn build_provider_config(cli_config: &CliConfig, cli_args: &CliArgs) -> AiConfig {
    let api_key = cli_config
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

    let base_url = cli_config
        .base_url
        .clone()
        .or_else(|| env::var("OPENROUTER_BASE_URL").ok())
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());

    let model = cli_args
        .model
        .clone()
        .or_else(|| cli_config.model.clone())
        .or_else(|| env::var("OPENROUTER_MODEL").ok())
        .unwrap_or_else(|| "anthropic/claude-haiku-4.5".to_string());

    AiConfig::new(base_url, api_key, model)
}

/// Register the default set of tools (Read, Write, Bash).
pub fn register_default_tools(registry: &mut ToolRegistry) {
    registry.register(ReadTool);
    registry.register(WriteTool);
    registry.register(EditTool);
    registry.register(BashTool);
}

/// Run the agent with the given CLI arguments, or launch the TUI if no prompt is given.
pub async fn run(args: CliArgs) -> Result<(), anyhow::Error> {
    // No prompt → launch TUI
    if args.prompt.is_none() {
        oy_tui::run_tui()
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
        return Ok(());
    }

    let prompt = args.prompt.as_ref().unwrap();
    let cli_config = CliConfig::load();
    let ai_config = build_provider_config(&cli_config, &args);

    eprintln!("url:{}", ai_config.base_url);
    eprintln!(
        "key:{}...",
        &ai_config.api_key[..8.min(ai_config.api_key.len())]
    );

    let provider = OpenCodeGoProvider::new(ai_config);
    let mut registry = ToolRegistry::new();
    register_default_tools(&mut registry);

    let main_agent = MainAgent::new_with_max_iterations(None);
    let mut orchestrator = Orchestrator::new(main_agent, provider, registry);
    orchestrator.init();
    let result = orchestrator.execute(prompt).await?;
    println!("{}", result);
    Ok(())
}
