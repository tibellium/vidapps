use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use prost::Message;
use rsa::{pkcs1::DecodeRsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};

/**
    Widevine L3 CDM command-line tool.
*/
#[derive(Parser)]
#[command(name = "wdv3")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect a WVD device file.
    Device {
        /// Path to the .wvd file.
        path: PathBuf,
    },
    /// Inspect a PSSH box.
    Pssh {
        /// Base64-encoded PSSH box.
        base64: String,
    },
    /// Create a .wvd device file from raw credential files.
    Create {
        /// RSA private key file (PEM or DER, PKCS#1 or PKCS#8).
        #[arg(short, long)]
        key: PathBuf,

        /// ClientIdentification protobuf blob file.
        #[arg(short, long)]
        client_id: PathBuf,

        /// Device type.
        #[arg(short = 't', long = "type", default_value = "android")]
        device_type: wdv3::DeviceType,

        /// Security level.
        #[arg(short, long, default_value = "3")]
        level: wdv3::SecurityLevel,

        /// Output file path. If omitted, auto-generates from client info.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Acquire content decryption keys from a license server.
    Keys {
        /// Path to the .wvd device file.
        #[arg(short, long)]
        device: PathBuf,

        /// Base64-encoded PSSH box.
        #[arg(short, long)]
        pssh: String,

        /// License server URL to POST the challenge to.
        #[arg(short, long)]
        url: String,

        /// License type: streaming (default), offline, or automatic.
        #[arg(short, long, default_value = "streaming")]
        license_type: wdv3::LicenseType,

        /// Enable privacy mode with a service certificate.
        /// Use "common" for Google's production cert, "staging" for Google's staging cert,
        /// or a file path to a custom certificate.
        #[arg(long)]
        privacy: Option<String>,

        /// Additional HTTP headers in "Key: Value" format. Can be repeated.
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Device { path } => cmd_device(&path),
        Command::Pssh { base64 } => cmd_pssh(&base64),
        Command::Create {
            key,
            client_id,
            device_type,
            level,
            output,
        } => cmd_create_device(&key, &client_id, device_type, level, output.as_deref()),
        Command::Keys {
            device,
            pssh,
            url,
            license_type,
            privacy,
            headers,
        } => {
            cmd_keys(
                &device,
                &pssh,
                &url,
                license_type,
                privacy.as_deref(),
                &headers,
            )
            .await
        }
    }
}

fn cmd_device(path: &PathBuf) -> Result<()> {
    let data = std::fs::read(path).context("failed to read WVD file")?;
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

fn cmd_pssh(base64: &str) -> Result<()> {
    let pssh = wdv3::PsshBox::from_base64(base64).context("failed to parse PSSH box")?;

    println!("Version:    {}", pssh.version);
    println!("System ID:  {}", hex::encode(pssh.system_id));
    println!("Data Size:  {} bytes", pssh.data.len());

    match pssh.key_ids() {
        Ok(kids) if !kids.is_empty() => {
            println!();
            println!("Key IDs ({}):", kids.len());
            for kid in &kids {
                println!("  {}", hex::encode(kid));
            }
        }
        _ => {}
    }

    if let Ok(pssh_data) = pssh.widevine_pssh_data()
        && let Some(content_id) = &pssh_data.content_id
    {
        println!();
        println!("Content ID: {}", String::from_utf8_lossy(content_id));
    }

    Ok(())
}

fn cmd_create_device(
    key_path: &PathBuf,
    client_id_path: &PathBuf,
    device_type: wdv3::DeviceType,
    security_level: wdv3::SecurityLevel,
    output: Option<&std::path::Path>,
) -> Result<()> {
    // Parse the RSA private key (try PEM then DER, PKCS#8 then PKCS#1)
    let key_data = std::fs::read(key_path).context("failed to read private key file")?;
    let private_key = parse_private_key(&key_data)
        .context("failed to parse RSA private key (expected PEM or DER, PKCS#1 or PKCS#8)")?;
    eprintln!("Loaded RSA private key ({} bits)", private_key.size() * 8);

    // Parse the ClientIdentification protobuf
    let cid_data = std::fs::read(client_id_path).context("failed to read client_id file")?;
    let client_id = wdv3_proto::ClientIdentification::decode(cid_data.as_slice())
        .context("failed to decode ClientIdentification protobuf")?;
    eprintln!("Loaded ClientIdentification ({} bytes)", cid_data.len());

    // Build the device
    let device = wdv3::Device::new(device_type, security_level, private_key, client_id);

    // Determine output path
    let output_path = match output {
        Some(p) => p.to_path_buf(),
        None => {
            let filename = derive_device_filename(&device);
            PathBuf::from(&filename)
        }
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

/**
    Try parsing an RSA private key from various formats.
    Attempts PEM (PKCS#8, PKCS#1) then DER (PKCS#8, PKCS#1).
*/
fn parse_private_key(data: &[u8]) -> Result<rsa::RsaPrivateKey> {
    // Try PEM formats first (if the data looks like text)
    if let Ok(pem_str) = std::str::from_utf8(data) {
        if let Ok(key) = rsa::RsaPrivateKey::from_pkcs8_pem(pem_str) {
            return Ok(key);
        }
        if let Ok(key) = rsa::RsaPrivateKey::from_pkcs1_pem(pem_str) {
            return Ok(key);
        }
    }

    // Try DER formats
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
fn derive_device_filename(device: &wdv3::Device) -> String {
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

async fn cmd_keys(
    device_path: &PathBuf,
    pssh_b64: &str,
    url: &str,
    license_type: wdv3::LicenseType,
    privacy: Option<&str>,
    headers: &[String],
) -> Result<()> {
    // Load device
    let wvd_data = std::fs::read(device_path).context("failed to read WVD file")?;
    let device = wdv3::Device::from_bytes(&wvd_data).context("failed to parse WVD file")?;

    eprintln!(
        "Loaded device: {} {}",
        device.device_type, device.security_level
    );

    // Create session
    let mut session = wdv3::Session::new(device);

    // Privacy mode
    if let Some(privacy) = privacy {
        match privacy {
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
    let pssh = wdv3::PsshBox::from_base64(pssh_b64).context("failed to parse PSSH box")?;

    let challenge = session
        .build_license_challenge(&pssh, license_type)
        .context("failed to build license challenge")?;
    eprintln!("Built challenge ({} bytes)", challenge.len());

    // Send to license server
    let client = reqwest::Client::new();
    let mut request = client.post(url).body(challenge);
    for h in headers {
        let (key, value) = parse_header(h)?;
        request = request.header(&key, &value);
    }

    eprintln!("Sending challenge to {url}");
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

fn parse_header(s: &str) -> Result<(String, String)> {
    let (key, value) = s
        .split_once(':')
        .context("header must be in 'Key: Value' format")?;
    Ok((key.trim().to_string(), value.trim().to_string()))
}
