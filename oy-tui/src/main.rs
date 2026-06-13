#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
// too_many_lines omitted for TUI crate — UI rendering functions are naturally long but low complexity.
// Threshold is configured in .clippy.toml as a guideline for non-UI crates.
#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    oy_tui::run_tui(None).await
}
