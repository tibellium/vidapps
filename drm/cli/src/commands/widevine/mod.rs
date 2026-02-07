mod create_device;
mod get_keys;
mod inspect_device;

use anyhow::Result;
use clap::{Args, Subcommand};

use self::create_device::CreateDeviceCommand;
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
    InspectDevice(InspectDeviceCommand),
    GetKeys(GetKeysCommand),
}

impl WidevineCommand {
    pub async fn run(self) -> Result<()> {
        match self.command {
            WidevineSubcommand::CreateDevice(cmd) => cmd.run(),
            WidevineSubcommand::InspectDevice(cmd) => cmd.run(),
            WidevineSubcommand::GetKeys(cmd) => cmd.run().await,
        }
    }
}
