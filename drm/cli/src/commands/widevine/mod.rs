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
    Widevine DRM commands.
*/
#[derive(Args)]
pub struct WidevineCommand {
    #[command(subcommand)]
    command: WidevineSubcommand,
}

#[derive(Subcommand)]
enum WidevineSubcommand {
    CreateDevice(CreateDeviceCommand),
    ExportDevice(ExportDeviceCommand),
    InspectDevice(InspectDeviceCommand),
    GetKeys(GetKeysCommand),
}

impl WidevineCommand {
    pub async fn run(self) -> Result<()> {
        match self.command {
            WidevineSubcommand::CreateDevice(cmd) => cmd.run(),
            WidevineSubcommand::ExportDevice(cmd) => cmd.run(),
            WidevineSubcommand::InspectDevice(cmd) => cmd.run(),
            WidevineSubcommand::GetKeys(cmd) => cmd.run().await,
        }
    }
}
