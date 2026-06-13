use clap::Parser;
use oy_agent::infrastructure::persistence::{
    find_latest_session, get_session_preview, list_all_sessions, list_sub_agent_sessions,
};
use oy_agent::infrastructure::tools::edit::EditTool;
use oy_agent::infrastructure::tools::grep::GrepTool;
use oy_agent::infrastructure::tools::read::ReadTool;
use oy_agent::infrastructure::tools::write::WriteTool;
use oy_agent::infrastructure::tools::{ToolRegistry, bash::BashTool};
use oy_ai::AiConfig;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// CLI arguments for oy-agent
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Prompt to send to the agent (if omitted, launches the TUI)
    #[arg(short = 'p', long)]
    pub prompt: Option<String>,

    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Continue latest session (load most recent session, or start new if none)
    #[arg(short = 'c', long)]
    pub r#continue: bool,

    /// Restore a session interactively (session selector)
    #[arg(short = 'r', long)]
    pub restore: bool,

    /// Load a specific session file by path
    #[arg(short = 's', long = "session")]
    pub session: Option<PathBuf>,
}

#[derive(Parser, Debug)]
pub enum Commands {
    /// Update oy CLI tool to the latest version via npm
    Update,
    /// List and restore sub-agent sessions
    #[command(name = "sub-sessions")]
    SubSessions,
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

/// Build an `AiConfig` by merging CLI args, config file, and defaults.
///
/// Priority (highest first):
///   1. CLI argument (`--model`)
///   2. Config file (`~/.oy-ai-agent/config.toml`)
///   3. Hardcoded default
///
/// `api_key` is required: if none of the sources provide it, the process exits.
pub fn build_provider_config(cli_config: &CliConfig, cli_args: &CliArgs) -> AiConfig {
    let api_key = cli_config.api_key.clone().unwrap_or_else(|| {
        eprintln!(
            "API key is not set. Set it in ~/.oy-ai-agent/config.toml:\n\n\
             [api_key]\n\
             api_key = \"sk-or-...\""
        );
        std::process::exit(1);
    });

    let base_url = cli_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());

    let model = cli_args
        .model
        .clone()
        .or_else(|| cli_config.model.clone())
        .unwrap_or_else(|| "anthropic/claude-haiku-4.5".to_string());

    AiConfig::new(base_url, api_key, model)
}

/// Register the default set of tools (Read, Write, Bash).
pub fn register_default_tools(registry: &mut ToolRegistry) {
    registry.register(ReadTool);
    registry.register(WriteTool);
    registry.register(EditTool);
    registry.register(BashTool);
    registry.register(GrepTool);
}

/// Run the agent with the given CLI arguments, or launch the TUI if no prompt is given.
pub async fn run(args: CliArgs) -> Result<(), anyhow::Error> {
    // 1. Update subcommand
    if matches!(args.command, Some(Commands::Update)) {
        return run_update().await;
    }

    // 2. Sub-sessions command
    if matches!(args.command, Some(Commands::SubSessions)) {
        return run_sub_sessions().await;
    }

    // 3. Continue latest session
    if args.r#continue {
        return run_continue_session().await;
    }

    // 4. Restore session from interactive selector
    if args.restore {
        return run_restore_session().await;
    }

    // 5. Load a specific session file by path
    if let Some(path) = &args.session {
        return run_session_path(path).await;
    }

    // 6. Existing logic: launch TUI (fresh) or handle direct prompt
    if args.prompt.is_some() {
        // TODO: implement direct prompt mode
        return Ok(());
    }

    // Launch fresh TUI
    oy_tui::run_tui(None)
        .await
        .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
    Ok(())
}

// ── Update subcommand ──────────────────────────────────────────

async fn run_update() -> Result<(), anyhow::Error> {
    let timeout = Duration::from_secs(300);

    // First attempt: default registry
    println!(
        "⏳ Running: npm install -g @ghyper9023/oy (timeout: {}s)...",
        timeout.as_secs()
    );
    match run_npm(&["install", "-g", "@ghyper9023/oy"], timeout).await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                println!("{}", stderr);
            }
            println!("✅ Update successful:\n{}", stdout);
            return Ok(());
        },
        Err(e) => {
            println!("⚠️  First attempt failed: {}", e);
            println!("⏳ Retrying with npm official registry...");
        },
    }

    // Second attempt: official npm registry
    match run_npm(
        &[
            "install",
            "-g",
            "@ghyper9023/oy",
            "--registry",
            "https://registry.npmjs.org/",
        ],
        timeout,
    )
    .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                println!("{}", stderr);
            }
            println!("✅ Update successful:\n{}", stdout);
            Ok(())
        },
        Err(e) => {
            eprintln!("❌ Update failed: {}", e);
            std::process::exit(1);
        },
    }
}

async fn run_npm(args: &[&str], timeout: Duration) -> Result<std::process::Output, anyhow::Error> {
    let child = Command::new("npm").args(args).kill_on_drop(true).output();

    tokio::time::timeout(timeout, child)
        .await
        .map_err(|_| anyhow::anyhow!("Command timed out after {}s", timeout.as_secs()))?
        .map_err(|e| anyhow::anyhow!("Failed to execute npm: {}", e))
        .and_then(|output| {
            if output.status.success() {
                Ok(output)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!(
                    "npm exited with code {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                ))
            }
        })
}

// ── Session commands ───────────────────────────────────────────

async fn run_continue_session() -> Result<(), anyhow::Error> {
    match find_latest_session() {
        Ok(Some(entry)) => {
            eprintln!(
                "📂 Resuming session: {} (project: {})",
                entry.uuid, entry.project_name
            );
            oy_tui::run_tui(Some(entry.path))
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
        },
        Ok(None) => {
            eprintln!("ℹ️  No previous session found. Starting fresh.");
            oy_tui::run_tui(None)
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
        },
        Err(e) => {
            eprintln!("⚠️  Error finding sessions: {}", e);
            oy_tui::run_tui(None)
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
        },
    }
    Ok(())
}

async fn run_restore_session() -> Result<(), anyhow::Error> {
    let sessions = list_all_sessions()?;

    if sessions.is_empty() {
        eprintln!("ℹ️  No sessions found. Starting fresh.");
        oy_tui::run_tui(None)
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
        return Ok(());
    }

    // ── Interactive session selector ──
    eprintln!("\n📋 Select a session to restore:\n");
    for (i, entry) in sessions.iter().enumerate() {
        let preview = get_session_preview(&entry.path)
            .ok()
            .flatten()
            .unwrap_or_else(|| "(no user message)".to_string());
        let uuid_str = entry.uuid.to_string();
        let uuid_short: String = uuid_str.chars().take(12).collect();
        eprintln!(
            "  [{:2}] {}... | {} | {}",
            i + 1,
            uuid_short,
            entry.project_name,
            preview
        );
    }
    eprintln!("\n  [0] Cancel");
    eprint!("\nEnter selection (0-{}): ", sessions.len());
    std::io::Write::flush(&mut std::io::stderr())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if let Ok(num) = input.parse::<usize>() {
        if num == 0 {
            eprintln!("❌ Cancelled.");
            return Ok(());
        }
        if num > sessions.len() {
            eprintln!("❌ Invalid selection.");
            return Ok(());
        }
        let entry = &sessions[num - 1];
        eprintln!("📂 Restoring session: {}", entry.uuid);
        oy_tui::run_tui(Some(entry.path.clone()))
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
    } else {
        eprintln!("❌ Invalid selection.");
    }

    Ok(())
}

// ── Sub-sessions command ──────────────────────────────────────

async fn run_sub_sessions() -> Result<(), anyhow::Error> {
    let sessions = list_sub_agent_sessions()?;

    if sessions.is_empty() {
        eprintln!("ℹ️  No sub-agent sessions found.");
        return Ok(());
    }

    // ── Interactive session selector ──
    eprintln!("\n📋 Select a sub-agent session to restore:\n");
    for (i, entry) in sessions.iter().enumerate() {
        let preview = get_session_preview(&entry.path)
            .ok()
            .flatten()
            .unwrap_or_else(|| "(no user message)".to_string());
        let uuid_str = entry.uuid.to_string();
        let uuid_short: String = uuid_str.chars().take(12).collect();
        eprintln!(
            "  [{:2}] {}... | {} | {}",
            i + 1,
            uuid_short,
            entry.project_name,
            preview
        );
    }
    eprintln!("\n  [0] Cancel");
    eprint!("\nEnter selection (0-{}): ", sessions.len());
    std::io::Write::flush(&mut std::io::stderr())?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if let Ok(num) = input.parse::<usize>() {
        if num == 0 {
            eprintln!("❌ Cancelled.");
            return Ok(());
        }
        if num > sessions.len() {
            eprintln!("❌ Invalid selection.");
            return Ok(());
        }
        let entry = &sessions[num - 1];
        eprintln!(
            "📂 Restoring sub-agent session: {} (project: {})",
            entry.uuid, entry.project_name
        );
        oy_tui::run_tui(Some(entry.path.clone()))
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
    } else {
        eprintln!("❌ Invalid selection.");
    }

    Ok(())
}

// ── Load session by path ───────────────────────────────────────

async fn run_session_path(path: &Path) -> Result<(), anyhow::Error> {
    if !path.exists() {
        eprintln!("❌ Session file not found: {}", path.display());
        std::process::exit(1);
    }
    if !path.is_file() {
        eprintln!("❌ Path is not a file: {}", path.display());
        std::process::exit(1);
    }

    // Validate that the file contains valid session data
    match oy_agent::infrastructure::persistence::load_session_messages(path) {
        Ok((uuid, _msgs)) => {
            eprintln!("📂 Loading session: {} ({})", uuid, path.display());
            oy_tui::run_tui(Some(path.to_path_buf()))
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
            Ok(())
        },
        Err(e) => {
            eprintln!("❌ Failed to load session file: {}", e);
            std::process::exit(1);
        },
    }
}
