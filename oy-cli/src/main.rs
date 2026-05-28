use clap::Parser;
use oy_cli::{CliArgs, run};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = CliArgs::parse();
    run(args).await
}
