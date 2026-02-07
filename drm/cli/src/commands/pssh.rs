use anyhow::{Context, Result};
use clap::Args;
use drm_widevine::WidevineExt;

/**
    Inspect a PSSH box.
*/
#[derive(Args)]
pub struct PsshCommand {
    /// Base64-encoded PSSH box.
    pub base64: String,
}

impl PsshCommand {
    pub fn run(self) -> Result<()> {
        let pssh =
            drm_widevine::PsshBox::from_base64(&self.base64).context("failed to parse PSSH box")?;

        println!("Version:    {}", pssh.version);
        println!("System ID:  {}", hex::encode(pssh.system_id));
        println!("Data Size:  {} bytes", pssh.data.len());

        let kids = pssh.key_ids();
        if !kids.is_empty() {
            println!();
            println!("Key IDs ({}):", kids.len());
            for kid in kids {
                println!("  {}", hex::encode(kid));
            }
        }

        if let Ok(pssh_data) = pssh.widevine_pssh_data()
            && let Some(content_id) = &pssh_data.content_id
        {
            println!();
            println!("Content ID: {}", String::from_utf8_lossy(content_id));
        }

        Ok(())
    }
}
