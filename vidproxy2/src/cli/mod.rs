use anyhow::Result;
use clap::{Parser, Subcommand};

mod list_sources;
mod serve;

pub use list_sources::ListSourcesCommand;
pub use serve::ServeCommand;

#[derive(Parser, Debug)]
#[command(name = "vidproxy")]
#[command(about = "Multi-channel HLS proxy with automatic DRM key extraction")]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the HTTP proxy server (default)
    Serve(ServeCommand),
    /// List available sources and exit
    ListSources(ListSourcesCommand),
}

impl Args {
    pub async fn run(self) -> Result<()> {
        let command = self
            .command
            .unwrap_or(Command::Serve(ServeCommand::default()));

        match command {
            Command::Serve(cmd) => cmd.run().await,
            Command::ListSources(cmd) => cmd.run().await,
        }
    }
}
