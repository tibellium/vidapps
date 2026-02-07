use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;

/**
    Acquire content decryption keys from a license server.
*/
#[derive(Args)]
pub struct GetKeysCommand {
    /**
        Path to the .wvd device file.
    */
    #[arg(short, long)]
    device: PathBuf,

    /**
        Base64-encoded PSSH box.
    */
    #[arg(short, long)]
    pssh: String,

    /**
        License server URL to POST the challenge to.
    */
    #[arg(short, long)]
    url: String,

    /**
        License type: streaming (default), offline, or automatic.
    */
    #[arg(short, long, default_value = "streaming")]
    license_type: drm_widevine::LicenseType,

    /**
        Enable privacy mode with a service certificate.
        Use "common" for Google's production cert, "staging" for Google's staging cert,
        or a file path to a custom certificate.
    */
    #[arg(long)]
    privacy: Option<String>,

    /**
        Additional HTTP headers in "Key: Value" format. Can be repeated.
    */
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
}

impl GetKeysCommand {
    pub async fn run(self) -> Result<()> {
        // Load device
        let wvd_data = std::fs::read(&self.device).context("failed to read WVD file")?;
        let device =
            drm_widevine::Device::from_bytes(&wvd_data).context("failed to parse WVD file")?;

        eprintln!(
            "Loaded device: {} {}",
            device.device_type, device.security_level
        );

        // Create session
        let mut session = drm_widevine::Session::new(device);

        // Privacy mode
        if let Some(ref privacy) = self.privacy {
            match privacy.as_str() {
                "common" => {
                    session
                        .set_service_certificate_common()
                        .context("failed to set common service certificate")?;
                    eprintln!("Privacy mode: common (license.widevine.com)");
                }
                "staging" => {
                    session
                        .set_service_certificate_staging()
                        .context("failed to set staging service certificate")?;
                    eprintln!("Privacy mode: staging (staging.google.com)");
                }
                path => {
                    let cert_data =
                        std::fs::read(path).context("failed to read service certificate file")?;
                    session
                        .set_service_certificate(&cert_data)
                        .context("failed to verify service certificate")?;
                    eprintln!("Privacy mode: custom certificate");
                }
            }
        }

        // Parse PSSH and build challenge
        let pssh =
            drm_widevine::PsshBox::from_base64(&self.pssh).context("failed to parse PSSH box")?;

        let challenge = session
            .build_license_challenge(&pssh, self.license_type)
            .context("failed to build license challenge")?;
        eprintln!("Built challenge ({} bytes)", challenge.len());

        // Send to license server
        let client = reqwest::Client::new();
        let mut request = client.post(&self.url).body(challenge);
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
