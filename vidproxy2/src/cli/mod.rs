use anyhow::Result;
use clap::{Parser, Subcommand};

mod list_sources;
mod serve;
mod test_source;

pub use list_sources::ListSourcesCommand;
pub use serve::ServeCommand;
pub use test_source::TestSourceCommand;

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
    /// Test a source by running all phases and printing results
    TestSource(TestSourceCommand),
}

impl Args {
    pub async fn run(self) -> Result<()> {
        let command = self
            .command
            .unwrap_or(Command::Serve(ServeCommand::default()));

        match command {
            Command::Serve(cmd) => cmd.run().await,
            Command::ListSources(cmd) => cmd.run().await,
            Command::TestSource(cmd) => cmd.run().await,
        }
    }
}
