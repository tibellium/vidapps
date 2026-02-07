use anyhow::{Context, Result};
use clap::Args;

use drm_playready::PlayReadyExt;
use drm_widevine::WidevineExt;

/**
    Inspect a PSSH box.
*/
#[derive(Args)]
pub struct InspectPsshCommand {
    /// Base64-encoded PSSH box.
    pub base64: String,
}

impl InspectPsshCommand {
    pub fn run(self) -> Result<()> {
        let pssh =
            drm_core::PsshBox::from_base64(&self.base64).context("failed to parse PSSH box")?;

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

        // Widevine-specific data
        if let Ok(pssh_data) = pssh.widevine_pssh_data()
            && let Some(content_id) = &pssh_data.content_id
        {
            println!();
            println!("Content ID: {}", String::from_utf8_lossy(content_id));
        }

        // PlayReady-specific data
        if let Ok(wrm) = pssh.playready_wrm_header() {
            println!();
            println!("WRM Header: v{}", wrm.version);
            if let Some(url) = &wrm.la_url {
                println!("LA URL:     {url}");
            }
            if let Some(url) = &wrm.lui_url {
                println!("LUI URL:    {url}");
            }
            if let Some(ds_id) = &wrm.ds_id {
                println!("DS ID:      {ds_id}");
            }
            if !wrm.kids.is_empty() {
                println!();
                println!("PlayReady Key IDs ({}):", wrm.kids.len());
                for sk in &wrm.kids {
                    println!("  {}", hex::encode(sk.key_id));
                }
            }
        }

        Ok(())
    }
}
