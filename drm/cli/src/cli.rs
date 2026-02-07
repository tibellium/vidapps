use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{InspectPsshCommand, PlayReadyCommand, WidevineCommand};

/**
    DRM command-line tool.
*/
#[derive(Parser)]
#[command(name = "drm-cli")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Widevine DRM commands.
    Widevine(WidevineCommand),
    /// PlayReady DRM commands.
    PlayReady(PlayReadyCommand),
    /// Inspect a PSSH box.
    InspectPssh(InspectPsshCommand),
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Widevine(cmd) => cmd.run().await,
            Command::PlayReady(cmd) => cmd.run().await,
            Command::InspectPssh(cmd) => cmd.run(),
        }
    }
}
