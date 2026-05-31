pub mod app;
pub mod event;
pub mod ui;

mod agent;
mod command;
mod load_config;
mod message;
mod theme;

use crate::app::App;
use crossterm::execute;

/// Shared TUI entry point — callable from oy-code-cli.
pub async fn run_tui() -> color_eyre::Result<()> {
    color_eyre::install()?;
    execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste,
    )?;
    let terminal = ratatui::init();
    let result = App::new().await.run(terminal).await;
    ratatui::restore();
    execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
    )?;
    result
}
