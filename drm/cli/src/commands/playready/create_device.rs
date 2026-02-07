use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::Args;

use drm_playready::format::bcert::CertType;

/**
    Create a .prd device file from raw credential files.

    Key files can be either 32 bytes (private key only, public key derived via P-256)
    or 96 bytes (32-byte private + 64-byte public).
*/
#[derive(Args)]
pub struct CreateDeviceCommand {
    /// Encryption key file (32 bytes private-only, or 96 bytes private + public).
    #[arg(short, long)]
    encryption_key: PathBuf,

    /// Signing key file (32 bytes private-only, or 96 bytes private + public).
    #[arg(short, long)]
    signing_key: PathBuf,

    /// Group certificate chain file (raw BCert chain, starts with "CHAI").
    #[arg(short, long)]
    certificate: PathBuf,

    /// Group key file (32 bytes private-only, or 96 bytes private + public). Optional.
    #[arg(short, long)]
    group_key: Option<PathBuf>,

    /// Output file path. If omitted, auto-generates from certificate info.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

impl CreateDeviceCommand {
    pub fn run(self) -> Result<()> {
        // Read encryption key
        let enc_data =
            std::fs::read(&self.encryption_key).context("failed to read encryption key file")?;
        eprintln!("Loaded encryption key ({} bytes)", enc_data.len());

        // Read signing key
        let sign_data =
            std::fs::read(&self.signing_key).context("failed to read signing key file")?;
        eprintln!("Loaded signing key ({} bytes)", sign_data.len());

        // Read group certificate
        let cert_data =
            std::fs::read(&self.certificate).context("failed to read certificate file")?;
        ensure!(
            cert_data.len() >= 4 && &cert_data[..4] == b"CHAI",
            "certificate file does not appear to be a BCert chain (expected CHAI magic)"
        );
        eprintln!("Loaded certificate chain ({} bytes)", cert_data.len());

        // Read optional group key
        let gk_data = if let Some(ref path) = self.group_key {
            let data = std::fs::read(path).context("failed to read group key file")?;
            eprintln!("Loaded group key ({} bytes)", data.len());
            Some(data)
        } else {
            None
        };

        // Build the device — dispatch based on key file sizes
        let device = build_device(&enc_data, &sign_data, gk_data.as_deref(), cert_data)
            .context("failed to create device")?;

        // Determine output path
        let output_path = match &self.output {
            Some(p) => p.clone(),
            None => PathBuf::from(derive_device_filename(&device)),
        };

        // Serialize and write
        let prd_bytes = device.to_bytes();
        std::fs::write(&output_path, &prd_bytes).context("failed to write PRD file")?;

        eprintln!(
            "Created {} ({} bytes)",
            output_path.display(),
            prd_bytes.len()
        );

        // Print device info
        println!("Security Level:  {}", device.security_level);
        println!(
            "Group Key:       {}",
            if device.has_group_key() {
                "present"
            } else {
                "absent"
            }
        );

        if let Ok(chain) = device.group_certificate_chain()
            && let Some(leaf) = chain.leaf()
            && let Some(info) = leaf.basic_info()
        {
            let cert_type = CertType::from_u32(info.cert_type)
                .map(|t| t.to_name().to_string())
                .unwrap_or_else(|| format!("Unknown({})", info.cert_type));
            println!("Cert Type:       {cert_type}");
        }

        Ok(())
    }
}

/**
    Build a device from key files that are either 32 bytes (private only) or 96 bytes (keypair).
*/
fn build_device(
    enc_data: &[u8],
    sign_data: &[u8],
    gk_data: Option<&[u8]>,
    cert_data: Vec<u8>,
) -> Result<drm_playready::Device> {
    match (enc_data.len(), sign_data.len()) {
        (32, 32) => {
            let enc: [u8; 32] = enc_data.try_into().unwrap();
            let sign: [u8; 32] = sign_data.try_into().unwrap();
            let gk = gk_data
                .map(|d| {
                    let k: [u8; 32] = d.try_into().map_err(|_| {
                        anyhow::anyhow!("group key must be 32 or 96 bytes, got {}", d.len())
                    })?;
                    Ok::<_, anyhow::Error>(k)
                })
                .transpose()?;
            Ok(drm_playready::Device::from_private_keys(
                enc, sign, gk, cert_data,
            )?)
        }
        (96, 96) => {
            let enc: [u8; 96] = enc_data.try_into().unwrap();
            let sign: [u8; 96] = sign_data.try_into().unwrap();
            let gk = gk_data
                .map(|d| {
                    let k: [u8; 96] = d.try_into().map_err(|_| {
                        anyhow::anyhow!("group key must be 32 or 96 bytes, got {}", d.len())
                    })?;
                    Ok::<_, anyhow::Error>(k)
                })
                .transpose()?;
            Ok(drm_playready::Device::new(enc, sign, gk, cert_data)?)
        }
        (n1, n2) => {
            anyhow::bail!(
                "key files must be 32 bytes (private only) or 96 bytes (keypair)\
                \ngot {n1} bytes for enc_data\
                \ngot {n2} bytes for sign_data"
            )
        }
    }
}

/**
    Derive a filename from the device's certificate metadata.
    Format: {manufacturer}_{model}_{security_level}.prd
*/
fn derive_device_filename(device: &drm_playready::Device) -> String {
    use drm_playready::format::bcert::AttributeData;

    let mut manufacturer = String::new();
    let mut model = String::new();

    if let Ok(chain) = device.group_certificate_chain()
        && let Some(leaf) = chain.leaf()
    {
        for attr in &leaf.attributes {
            if let AttributeData::Manufacturer(mfr) = &attr.data {
                manufacturer = mfr.name.clone();
                model = mfr.model_name.clone();
                break;
            }
        }
    }

    let manufacturer = if manufacturer.is_empty() {
        "unknown".to_string()
    } else {
        manufacturer.to_lowercase().replace(' ', "_")
    };
    let model = if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_lowercase().replace(' ', "_")
    };

    let sl = device.security_level.to_u32();
    format!("{manufacturer}_{model}_sl{sl}.prd")
}
