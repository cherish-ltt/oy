#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
#![deny(clippy::too_many_lines)]
use clap::Parser;
use oy_agent::infrastructure::persistence::{
    SessionEntry, find_latest_session, get_session_preview, list_all_sessions,
    list_sub_agent_sessions,
};
use oy_agent::infrastructure::tools::edit::EditTool;
use oy_agent::infrastructure::tools::grep::GrepTool;
use oy_agent::infrastructure::tools::read::ReadTool;
use oy_agent::infrastructure::tools::write::WriteTool;
use oy_agent::infrastructure::tools::{ToolRegistry, bash::BashTool};
use oy_ai::AiConfig;
use oy_ai::ChatMessage;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

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
    /// Load a specific session file by path
    Session {
        /// Path to the session JSON file
        path: PathBuf,
    },
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
/// `api_key` is required: if none of the sources provide it, an error is returned.
pub fn build_provider_config(
    cli_config: &CliConfig,
    cli_args: &CliArgs,
) -> Result<AiConfig, String> {
    let api_key = cli_config.api_key.clone().ok_or_else(|| {
        "API key is not set. Set it in ~/.oy-ai-agent/config.toml:\n\n\
         [api_key]\n\
         api_key = \"sk-or-...\""
            .to_string()
    })?;

    let base_url = cli_config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());

    let model = cli_args
        .model
        .clone()
        .or_else(|| cli_config.model.clone())
        .unwrap_or_else(|| "anthropic/claude-haiku-4.5".to_string());

    Ok(AiConfig::new(base_url, api_key, model))
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

    // 2a. Session subcommand
    if let Some(Commands::Session { path }) = &args.command {
        return run_session_command(path).await;
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

    println!(
        "⏳ Running: npm install -g @ghyper9023/oy (timeout: {}s)...",
        timeout.as_secs()
    );

    if try_npm_install(None, timeout).await.is_ok() {
        return Ok(());
    }

    println!("⏳ Retrying with npm official registry...");
    match try_npm_install(Some("https://registry.npmjs.org/"), timeout).await {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Run a single npm install attempt, optionally with a custom registry.
async fn try_npm_install(registry: Option<&str>, timeout: Duration) -> Result<(), anyhow::Error> {
    let mut args = vec!["install", "-g", "@ghyper9023/oy"];
    if let Some(reg) = registry {
        args.push("--registry");
        args.push(reg);
    }

    match run_npm(&args, timeout).await {
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
            println!("⚠️  npm install failed: {}", e);
            Err(e)
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

    if let Some(idx) = select_session_interactively(&sessions, "Select a session to restore")? {
        let entry = &sessions[idx];
        eprintln!("📂 Restoring session: {}", entry.uuid);
        oy_tui::run_tui(Some(entry.path.clone()))
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
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

    if let Some(idx) =
        select_session_interactively(&sessions, "Select a sub-agent session to restore")?
    {
        let entry = &sessions[idx];
        eprintln!(
            "📂 Restoring sub-agent session: {} (project: {})",
            entry.uuid, entry.project_name
        );
        oy_tui::run_tui(Some(entry.path.clone()))
            .await
            .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
    }

    Ok(())
}

// ── Load session by path ───────────────────────────────────────

/// Validate that a path points to a valid session file and load its messages.
fn validate_session_file(path: &Path) -> Result<(Uuid, Vec<ChatMessage>), String> {
    if !path.exists() {
        return Err(format!("Session file not found: {}", path.display()));
    }
    if !path.is_file() {
        return Err(format!("Path is not a file: {}", path.display()));
    }
    oy_agent::infrastructure::persistence::load_session_messages(path)
        .map_err(|e| format!("Failed to load session file: {}", e))
}

async fn run_session_path(path: &Path) -> Result<(), anyhow::Error> {
    match validate_session_file(path) {
        Ok((uuid, _msgs)) => {
            eprintln!("📂 Loading session: {} ({})", uuid, path.display());
            oy_tui::run_tui(Some(path.to_path_buf()))
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
            Ok(())
        },
        Err(e) => Err(anyhow::anyhow!("{}", e)),
    }
}

/// Load a session by path (subcommand version).
/// Prints friendly messages instead of returning errors for invalid files.
async fn run_session_command(path: &Path) -> Result<(), anyhow::Error> {
    match validate_session_file(path) {
        Ok((uuid, _msgs)) => {
            eprintln!("📂 Loading session: {} ({})", uuid, path.display());
            oy_tui::run_tui(Some(path.to_path_buf()))
                .await
                .map_err(|e| anyhow::Error::msg(format!("{}", e)))?;
            Ok(())
        },
        Err(e) => {
            eprintln!("❌ '{}' is not a valid OY session file.", path.display());
            eprintln!("   Reason: {}", e);
            eprintln!("ℹ️  No new conversation will be created.");
            Ok(())
        },
    }
}

/// Show an interactive session selector and return the selected index (0-based).
/// Returns `None` if the user cancels or the selection is invalid.
fn select_session_interactively(
    sessions: &[SessionEntry],
    title: &str,
) -> Result<Option<usize>, anyhow::Error> {
    eprintln!("\n📋 {}:\n", title);
    for (i, entry) in sessions.iter().enumerate() {
        let preview = get_session_preview(&entry.path)
            .ok()
            .flatten()
            .unwrap_or_else(|| "(no user message)".to_string());
        eprintln!(
            "  [{:2}] {}... | {} | {}",
            i + 1,
            entry.uuid.to_string().chars().take(12).collect::<String>(),
            entry.project_name,
            preview
        );
    }
    eprintln!("\n  [0] Cancel");
    eprint!("Enter selection (0-{}): ", sessions.len());
    std::io::Write::flush(&mut std::io::stderr())?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    match input.trim().parse::<usize>() {
        Ok(0) => {
            eprintln!("❌ Cancelled.");
            Ok(None)
        },
        Ok(num) if num <= sessions.len() => Ok(Some(num - 1)),
        _ => {
            eprintln!("❌ Invalid selection.");
            Ok(None)
        },
    }
}
