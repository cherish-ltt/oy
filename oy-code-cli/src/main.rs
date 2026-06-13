#![deny(clippy::cognitive_complexity)]
#![deny(clippy::too_many_arguments)]
#![deny(clippy::too_many_lines)]
use clap::Parser;
use oy_code_cli::{CliArgs, run};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = CliArgs::parse();
    run(args).await
}
