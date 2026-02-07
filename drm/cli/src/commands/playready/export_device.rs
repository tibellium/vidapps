use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

/**
    Export a .prd device file to its raw components.

    Writes the encryption key, signing key, group key, and group certificate
    chain to separate files, the inverse of `create-device`.
*/
#[derive(Args)]
pub struct ExportDeviceCommand {
    /// Path to the .prd file.
    path: PathBuf,

    /// Output directory. Defaults to the current directory.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

impl ExportDeviceCommand {
    pub fn run(self) -> Result<()> {
        let data = std::fs::read(&self.path).context("failed to read PRD file")?;
        let device =
            drm_playready::Device::from_bytes(&data).context("failed to parse PRD file")?;

        let stem = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("device");

        let out_dir = match &self.output {
            Some(p) => p.clone(),
            None => PathBuf::from("."),
        };

        let out_path = out_dir.join(stem);
        if out_path.exists() {
            let has_entries = std::fs::read_dir(&out_path)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
            if has_entries {
                anyhow::bail!("output directory {} is not empty", out_path.display());
            }
        } else {
            std::fs::create_dir_all(&out_path).context("failed to create output directory")?;
        }

        // Write encryption key (32 bytes private scalar)
        let enc_path = out_path.join("zprivencr.dat");
        std::fs::write(&enc_path, device.encryption_private_key())
            .context("failed to write encryption key")?;
        eprintln!("Exported encryption key to {}", enc_path.display());

        // Write signing key (32 bytes private scalar)
        let sign_path = out_path.join("zprivsig.dat");
        std::fs::write(&sign_path, device.signing_private_key())
            .context("failed to write signing key")?;
        eprintln!("Exported signing key to {}", sign_path.display());

        // Write group key if present (32 bytes private scalar)
        if let Some(group_key) = device.group_private_key() {
            let gk_path = out_path.join("zgpriv.dat");
            std::fs::write(&gk_path, group_key).context("failed to write group key")?;
            eprintln!("Exported group key to {}", gk_path.display());
        } else {
            eprintln!("No group key (v2 device), skipping zgpriv.dat");
        }

        // Write group certificate chain (raw BCert bytes)
        let cert_path = out_path.join("bgroupcert.dat");
        std::fs::write(&cert_path, device.group_certificate_bytes())
            .context("failed to write group certificate")?;
        eprintln!("Exported group certificate to {}", cert_path.display());

        // Print summary
        println!("Security Level:  {}", device.security_level);
        println!(
            "Group Key:       {}",
            if device.has_group_key() {
                "present"
            } else {
                "absent"
            }
        );
        println!("Exported to:     {}", out_path.display());

        Ok(())
    }
}
