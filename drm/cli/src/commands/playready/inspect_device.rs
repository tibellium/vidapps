use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use drm_playready::format::bcert::{AttributeData, CertType};

/**
    Inspect a PRD device file.
*/
#[derive(Args)]
pub struct InspectDeviceCommand {
    /// Path to the .prd file.
    pub path: PathBuf,
}

impl InspectDeviceCommand {
    pub fn run(self) -> Result<()> {
        let data = std::fs::read(&self.path).context("failed to read PRD file")?;
        let device =
            drm_playready::Device::from_bytes(&data).context("failed to parse PRD file")?;

        println!("Security Level:  {}", device.security_level);
        println!(
            "Group Key:       {}",
            if device.has_group_key() {
                "present"
            } else {
                "absent"
            }
        );

        let chain = device
            .group_certificate_chain()
            .context("failed to parse BCert chain")?;

        println!(
            "Certificate Chain: {} certificate(s)",
            chain.certificates.len()
        );

        for (i, cert) in chain.certificates.iter().enumerate() {
            println!();
            if i == 0 {
                println!("  [{i}] Leaf Certificate:");
            } else if i == chain.certificates.len() - 1 {
                println!("  [{i}] Root Certificate:");
            } else {
                println!("  [{i}] Intermediate Certificate:");
            }

            if let Some(info) = cert.basic_info() {
                let cert_type = CertType::from_u32(info.cert_type)
                    .map(|t| t.to_name().to_string())
                    .unwrap_or_else(|| format!("Unknown({})", info.cert_type));
                println!("    Type:             {cert_type}");
                println!("    Security Level:   {}", info.security_level);
                println!("    Cert ID:          {}", hex::encode(info.cert_id));
                println!("    Client ID:        {}", hex::encode(info.client_id));
                if info.expiration_date > 0 {
                    println!("    Expiration:       {}", info.expiration_date);
                }
            }

            for attr in &cert.attributes {
                if let AttributeData::Manufacturer(mfr) = &attr.data {
                    println!("    Manufacturer:     {}", mfr.name);
                    if !mfr.model_name.is_empty() {
                        println!("    Model Name:       {}", mfr.model_name);
                    }
                    if !mfr.model_number.is_empty() {
                        println!("    Model Number:     {}", mfr.model_number);
                    }
                }
            }

            if let Some(ki) = cert.key_info() {
                for key in &ki.keys {
                    let usages: Vec<_> = key.usages.iter().map(|u| format!("{u}")).collect();
                    println!(
                        "    Key:              {} bytes, usages=[{}]",
                        key.key.len(),
                        usages.join(", ")
                    );
                }
            }
        }

        Ok(())
    }
}
