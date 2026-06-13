#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
// too_many_lines omitted for TUI crate — UI rendering functions are naturally long but low complexity.
// Threshold is configured in .clippy.toml as a guideline for non-UI crates.
pub mod app;
pub mod event;
pub mod ui;

mod agent;
mod command;
mod config;
mod load_config;
mod message;
mod theme;

use std::path::PathBuf;

use crate::app::App;
use crossterm::execute;

/// Shared TUI entry point — callable from oy-code-cli.
///
/// If `session_path` is `Some`, the TUI will load that session on startup.
pub async fn run_tui(session_path: Option<PathBuf>) -> color_eyre::Result<()> {
    color_eyre::install()?;
    execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste,
    )?;
    let terminal = ratatui::init();
    let result = App::new(session_path).await.run(terminal).await;
    ratatui::restore();
    execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    result
}
