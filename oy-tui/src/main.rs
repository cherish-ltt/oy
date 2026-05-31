use crate::app::App;
use crossterm::execute;

mod agent;
pub mod app;
mod command;
pub mod event;
mod load_config;
mod message;
mod theme;
pub mod ui;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
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
