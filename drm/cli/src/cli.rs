use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{CreateCommand, DeviceCommand, KeysCommand, PsshCommand};

/**
    Widevine L3 CDM command-line tool.
*/
#[derive(Parser)]
#[command(name = "wdv3")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Device(DeviceCommand),
    Pssh(PsshCommand),
    Create(CreateCommand),
    Keys(KeysCommand),
}

impl Cli {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Command::Device(cmd) => cmd.run(),
            Command::Pssh(cmd) => cmd.run(),
            Command::Create(cmd) => cmd.run(),
            Command::Keys(cmd) => cmd.run().await,
        }
    }
}
