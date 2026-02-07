use anyhow::Result;
use clap::{Args, Subcommand};

mod create_device;
mod export_device;
mod get_keys;
mod inspect_device;

use self::create_device::CreateDeviceCommand;
use self::export_device::ExportDeviceCommand;
use self::get_keys::GetKeysCommand;
use self::inspect_device::InspectDeviceCommand;

/**
    PlayReady DRM commands.
*/
#[derive(Args)]
pub struct PlayReadyCommand {
    #[command(subcommand)]
    command: PlayReadySubcommand,
}

#[derive(Subcommand)]
enum PlayReadySubcommand {
    CreateDevice(CreateDeviceCommand),
    ExportDevice(ExportDeviceCommand),
    InspectDevice(InspectDeviceCommand),
    GetKeys(GetKeysCommand),
}

impl PlayReadyCommand {
    pub async fn run(self) -> Result<()> {
        match self.command {
            PlayReadySubcommand::CreateDevice(cmd) => cmd.run(),
            PlayReadySubcommand::ExportDevice(cmd) => cmd.run(),
            PlayReadySubcommand::InspectDevice(cmd) => cmd.run(),
            PlayReadySubcommand::GetKeys(cmd) => cmd.run().await,
        }
    }
}
