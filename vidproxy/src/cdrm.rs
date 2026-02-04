use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};

const CDRM_API_URL: &str = "https://cdrm-project.com/api/decrypt";

#[derive(Debug, Serialize)]
struct CdrmRequest {
    pssh: String,
    licurl: String,
    headers: String,
}

#[derive(Debug, Deserialize)]
struct CdrmResponse {
    message: String,
}

/**
    Extract PSSH and default_KID from an MPD manifest
*/
pub fn extract_drm_info_from_mpd(
    mpd_url: &str,
    mpd_content: &str,
) -> Result<(String, Option<String>)> {
    use ffmpeg_source::reader::stream::StreamFormat;
    use ffmpeg_source::reader::stream::dash::DashFormat;

    let dash = DashFormat::from_manifest(mpd_url, mpd_content.as_bytes())
        .map_err(|e| anyhow!("Failed to parse MPD: {}", e))?;

    let drm_info = dash.drm_info();

    // Get Widevine PSSH first, fall back to any PSSH
    let pssh = drm_info
        .widevine_pssh()
        .into_iter()
        .next()
        .map(|p| &p.data_base64)
        .or_else(|| drm_info.pssh_boxes.first().map(|p| &p.data_base64))
        .ok_or_else(|| anyhow!("No PSSH found in MPD"))?;

    // Extract default_KID from MPD content using regex
    // Format: cenc:default_KID="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
    let default_kid = extract_default_kid_from_mpd(mpd_content);

    Ok((pssh.clone(), default_kid))
}

/**
    Extract the default_KID attribute from MPD XML content.
*/
fn extract_default_kid_from_mpd(mpd_content: &str) -> Option<String> {
    // Match cenc:default_KID="..." with UUID format (with or without dashes)
    let re = Regex::new(r#"default_KID="([0-9a-fA-F-]+)""#).ok()?;
    re.captures(mpd_content)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().replace('-', "").to_lowercase())
}

/**
    Fetch all decryption keys from CDRM API.

    Returns all keys in "kid:key" format.
*/
pub async fn fetch_decryption_keys(pssh: &str, license_url: &str) -> Result<Vec<String>> {
    println!("[cdrm] Requesting decryption keys from CDRM API...");

    let client = reqwest::Client::new();
    let cdrm_req = CdrmRequest {
        pssh: pssh.to_string(),
        licurl: license_url.to_string(),
        headers: format!(
            "{:?}",
            std::collections::HashMap::from([
                (
                    "User-Agent",
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
                ),
                ("Accept", "*/*"),
            ])
        ),
    };

    let resp = client.post(CDRM_API_URL).json(&cdrm_req).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow!("CDRM API error: {}", resp.status()));
    }

    let cdrm_resp: CdrmResponse = resp.json().await?;

    // Extract all keys (format is "kid:key" per line)
    let keys: Vec<String> = cdrm_resp
        .message
        .lines()
        .filter(|line| line.contains(':') && line.len() > 32)
        .map(|s| s.to_string())
        .collect();

    if keys.is_empty() {
        return Err(anyhow!("No decryption keys found in CDRM response"));
    }

    println!("[cdrm] Got {} decryption key(s)", keys.len());
    Ok(keys)
}

/**
    Fetch MPD content and extract PSSH, then get all decryption keys.

    Returns all keys in "kid:key" format.
*/
pub async fn get_decryption_keys(mpd_url: &str, license_url: &str) -> Result<Vec<String>> {
    println!("[cdrm] Fetching MPD to extract PSSH...");

    let client = reqwest::Client::new();
    let mpd_content = client.get(mpd_url).send().await?.text().await?;

    let (pssh, default_kid) = extract_drm_info_from_mpd(mpd_url, &mpd_content)?;
    println!("[cdrm] Extracted PSSH: {}...", &pssh[..pssh.len().min(30)]);
    if let Some(ref kid) = default_kid {
        println!("[cdrm] MPD default_KID: {}...", &kid[..kid.len().min(8)]);
    }

    fetch_decryption_keys(&pssh, license_url).await
}
