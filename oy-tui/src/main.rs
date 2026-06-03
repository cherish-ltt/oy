#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    oy_tui::run_tui(None).await
}
