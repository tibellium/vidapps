use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;

use self::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    Cli::parse().run().await
}
