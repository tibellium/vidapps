use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use drm_widevine::proto::Message;
use rsa::{pkcs1::DecodeRsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};

/**
    Create a .wvd device file from raw credential files.
*/
#[derive(Args)]
pub struct CreateDeviceCommand {
    /// RSA private key file (PEM or DER, PKCS#1 or PKCS#8).
    #[arg(short, long)]
    key: PathBuf,

    /// ClientIdentification protobuf blob file.
    #[arg(short, long)]
    client_id: PathBuf,

    /// Device type.
    #[arg(short = 't', long = "type", default_value = "android")]
    device_type: drm_widevine::DeviceType,

    /// Security level.
    #[arg(short, long, default_value = "3")]
    level: drm_widevine::SecurityLevel,

    /// Output file path. If omitted, auto-generates from client info.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

impl CreateDeviceCommand {
    pub fn run(self) -> Result<()> {
        // Parse the RSA private key (try PEM then DER, PKCS#8 then PKCS#1)
        let key_data = std::fs::read(&self.key).context("failed to read private key file")?;
        let private_key = parse_private_key(&key_data)
            .context("failed to parse RSA private key (expected PEM or DER, PKCS#1 or PKCS#8)")?;
        eprintln!("Loaded RSA private key ({} bits)", private_key.size() * 8);

        // Parse the ClientIdentification protobuf
        let cid_data = std::fs::read(&self.client_id).context("failed to read client_id file")?;
        let client_id = drm_widevine::proto::ClientIdentification::decode(cid_data.as_slice())
            .context("failed to decode ClientIdentification protobuf")?;
        eprintln!("Loaded ClientIdentification ({} bytes)", cid_data.len());

        // Build the device
        let device =
            drm_widevine::Device::new(self.device_type, self.level, private_key, client_id);

        // Determine output path
        let output_path = match &self.output {
            Some(p) => p.clone(),
            None => PathBuf::from(derive_device_filename(&device)),
        };

        // Serialize and write
        let wvd_bytes = device.to_bytes().context("failed to serialize WVD")?;
        std::fs::write(&output_path, &wvd_bytes).context("failed to write WVD file")?;

        eprintln!(
            "Created {} ({} bytes)",
            output_path.display(),
            wvd_bytes.len()
        );

        // Print device info
        println!("Device Type:     {}", device.device_type);
        println!("Security Level:  {}", device.security_level);
        let cid = device.client_id();
        for info in &cid.client_info {
            let name = info.name.as_deref().unwrap_or("?");
            let value = info.value.as_deref().unwrap_or("?");
            println!("  {name}: {value}");
        }

        Ok(())
    }
}

/**
    Try parsing an RSA private key from various formats.
    Attempts PEM (PKCS#8, PKCS#1) then DER (PKCS#8, PKCS#1).
*/
fn parse_private_key(data: &[u8]) -> Result<rsa::RsaPrivateKey> {
    if let Ok(pem_str) = std::str::from_utf8(data) {
        if let Ok(key) = rsa::RsaPrivateKey::from_pkcs8_pem(pem_str) {
            return Ok(key);
        }
        if let Ok(key) = rsa::RsaPrivateKey::from_pkcs1_pem(pem_str) {
            return Ok(key);
        }
    }

    if let Ok(key) = rsa::RsaPrivateKey::from_pkcs8_der(data) {
        return Ok(key);
    }
    if let Ok(key) = rsa::RsaPrivateKey::from_pkcs1_der(data) {
        return Ok(key);
    }

    bail!("unrecognized key format")
}

/**
    Derive a filename from the device's client_id metadata.
    Format: {company}_{model}_{security_level}.wvd
*/
fn derive_device_filename(device: &drm_widevine::Device) -> String {
    let cid = device.client_id();
    let mut company = String::new();
    let mut model = String::new();

    for info in &cid.client_info {
        match info.name.as_deref() {
            Some("company_name") => {
                company = info.value.clone().unwrap_or_default();
            }
            Some("model_name") => {
                model = info.value.clone().unwrap_or_default();
            }
            _ => {}
        }
    }

    let company = if company.is_empty() {
        "unknown".to_string()
    } else {
        company.to_lowercase().replace(' ', "_")
    };
    let model = if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_lowercase().replace(' ', "_")
    };

    format!("{company}_{model}_{}.wvd", device.security_level)
}
