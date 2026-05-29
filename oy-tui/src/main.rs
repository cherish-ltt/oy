use crate::app::App;
use crossterm::execute;

pub mod app;
pub mod event;
mod load_config;
mod message;
pub mod ui;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;
    let terminal = ratatui::init();
    let result = App::new().run(terminal).await;
    ratatui::restore();
    execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
    result
}
