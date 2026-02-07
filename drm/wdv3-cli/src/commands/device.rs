use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

/**
    Inspect a WVD device file.
*/
#[derive(Args)]
pub struct DeviceCommand {
    /// Path to the .wvd file.
    pub path: PathBuf,
}

impl DeviceCommand {
    pub fn run(self) -> Result<()> {
        let data = std::fs::read(&self.path).context("failed to read WVD file")?;
        let device = wdv3::Device::from_bytes(&data).context("failed to parse WVD file")?;

        println!("Device Type:     {}", device.device_type);
        println!("Security Level:  {}", device.security_level);

        let client_id = device.client_id();

        if !client_id.client_info.is_empty() {
            println!();
            println!("Client Info:");
            for info in &client_id.client_info {
                let name = info.name.as_deref().unwrap_or("?");
                let value = info.value.as_deref().unwrap_or("?");
                println!("  {name}: {value}");
            }
        }

        if let Some(caps) = &client_id.client_capabilities {
            println!();
            println!("Capabilities:");
            if let Some(v) = caps.session_token {
                println!("  Session Token:       {v}");
            }
            if let Some(v) = caps.client_token {
                println!("  Client Token:        {v}");
            }
            if let Some(v) = caps.max_hdcp_version {
                println!("  Max HDCP Version:    {v}");
            }
            if let Some(v) = caps.oem_crypto_api_version {
                println!("  OEMCrypto API:       {v}");
            }
            if let Some(v) = caps.anti_rollback_usage_table {
                println!("  Anti-Rollback Table: {v}");
            }
        }

        Ok(())
    }
}
