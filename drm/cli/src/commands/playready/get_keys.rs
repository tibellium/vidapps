use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;

use drm_playready::PlayReadyExt;

/**
    Acquire content decryption keys from a PlayReady license server.
*/
#[derive(Args)]
pub struct GetKeysCommand {
    /// Path to the .prd device file.
    #[arg(short, long)]
    device: PathBuf,

    /// Base64-encoded PSSH box.
    #[arg(short, long)]
    pssh: String,

    /// License server URL to POST the challenge to.
    #[arg(short, long)]
    url: String,

    /// Additional HTTP headers in "Key: Value" format. Can be repeated.
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
}

impl GetKeysCommand {
    pub async fn run(self) -> Result<()> {
        // Load device
        let prd_data = std::fs::read(&self.device).context("failed to read PRD file")?;
        let device =
            drm_playready::Device::from_bytes(&prd_data).context("failed to parse PRD file")?;

        eprintln!("Loaded device: {}", device.security_level);

        // Create session
        let mut session = drm_playready::Session::new(device);

        // Parse PSSH and build challenge
        let pssh =
            drm_core::PsshBox::from_base64(&self.pssh).context("failed to parse PSSH box")?;

        let kid_count = pssh.playready_key_ids().map(|k| k.len()).unwrap_or(0);
        eprintln!("PSSH contains {kid_count} key ID(s)");

        let challenge = session
            .build_license_challenge(&pssh)
            .context("failed to build license challenge")?;
        eprintln!("Built challenge ({} bytes)", challenge.len());

        // Send to license server
        let client = reqwest::Client::new();
        let mut request = client
            .post(&self.url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .body(challenge);
        for h in &self.headers {
            let (key, value) = parse_header(h)?;
            request = request.header(&key, &value);
        }

        eprintln!("Sending challenge to {}", self.url);
        let response = request.send().await.context("HTTP request failed")?;
        let status = response.status();
        if !status.is_success() {
            bail!("license server returned HTTP {status}");
        }

        let response_bytes = response.bytes().await.context("failed to read response")?;
        eprintln!("Received response ({} bytes)", response_bytes.len());

        // Parse response
        let keys = session
            .parse_license_response(&response_bytes)
            .context("failed to parse license response")?;

        eprintln!("Extracted {} keys:", keys.len());
        eprintln!();

        for key in keys {
            println!("{key:?}");
        }

        let content_keys: Vec<_> = session.content_keys();
        if !content_keys.is_empty() {
            eprintln!();
            eprintln!("Content keys:");
            for key in &content_keys {
                println!("{key}");
            }
        }

        Ok(())
    }
}

fn parse_header(s: &str) -> Result<(String, String)> {
    let (key, value) = s
        .split_once(':')
        .context("header must be in 'Key: Value' format")?;
    Ok((key.trim().to_string(), value.trim().to_string()))
}
