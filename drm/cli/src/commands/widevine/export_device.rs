use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use rsa::pkcs1::EncodeRsaPrivateKey;

use drm_widevine::proto::Message;

/**
    Export a .wvd device file to its raw components.

    Writes the RSA private key (PKCS#1 DER) and ClientIdentification
    protobuf blob to separate files, the inverse of `create-device`.
*/
#[derive(Args)]
pub struct ExportDeviceCommand {
    /// Path to the .wvd file.
    path: PathBuf,

    /// Output directory. Defaults to the current directory.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

impl ExportDeviceCommand {
    pub fn run(self) -> Result<()> {
        let data = std::fs::read(&self.path).context("failed to read WVD file")?;
        let device = drm_widevine::Device::from_bytes(&data).context("failed to parse WVD file")?;

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

        // Write RSA private key (PKCS#1 DER)
        let private_key_der = device
            .private_key()
            .to_pkcs1_der()
            .context("failed to serialize RSA private key")?;
        let key_path = out_path.join("private_key.der");
        std::fs::write(&key_path, private_key_der.as_bytes())
            .context("failed to write private key")?;
        eprintln!("Exported private key to {}", key_path.display());

        // Write ClientIdentification protobuf blob
        let client_id_bytes = device.client_id().encode_to_vec();
        let cid_path = out_path.join("client_id.bin");
        std::fs::write(&cid_path, &client_id_bytes).context("failed to write client ID")?;
        eprintln!("Exported client ID to {}", cid_path.display());

        // Print summary
        println!("Device Type:     {}", device.device_type);
        println!("Security Level:  {}", device.security_level);
        println!("Exported to:     {}", out_path.display());

        Ok(())
    }
}
