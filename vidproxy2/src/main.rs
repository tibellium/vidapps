use anyhow::Result;
use clap::Parser;

mod channel;
mod cli;
mod engine;
mod media;
mod server;
mod util;

#[tokio::main]
async fn main() -> Result<()> {
    cli::Args::parse().run().await
}
