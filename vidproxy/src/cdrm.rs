use anyhow::{Result, anyhow};
use regex::Regex;

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
    Attempt to fetch a service certificate from the license server and set it
    on the session for privacy mode. Returns Ok if privacy mode was enabled,
    or an error if it couldn't be enabled (caller should fall back to plaintext).
*/
async fn try_enable_privacy_mode(
    session: &mut drm_widevine::Session,
    license_url: &str,
) -> Result<()> {
    let cert_request = drm_widevine::Session::service_certificate_request();
    let cert_response = license_request(license_url, cert_request).await?;
    session
        .set_service_certificate(&cert_response)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/**
    POST raw bytes to the license server and return the response body.
*/
async fn license_request(license_url: &str, body: Vec<u8>) -> Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(license_url)
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("License server error: {}", resp.status()));
    }

    Ok(resp.bytes().await?.to_vec())
}

/**
    Fetch decryption keys by performing local Widevine license acquisition.

    Fetches the server's service certificate first (for privacy mode), then
    builds a license challenge using a random embedded CDM device, POSTs it to the
    license server, and extracts content keys from the response.

    Returns all content keys in "kid:key" hex format.
*/
pub async fn fetch_decryption_keys(pssh_b64: &str, license_url: &str) -> Result<Vec<String>> {
    println!("[cdrm] Performing local license acquisition...");

    let pssh = drm_widevine::PsshBox::from_base64(pssh_b64)
        .map_err(|e| anyhow!("Failed to parse PSSH: {e}"))?;

    let device = drm_widevine::static_devices::random();
    let mut session = drm_widevine::Session::new(device);

    // Try to enable privacy mode by fetching the server's service certificate.
    // If the server doesn't support it or the cert fails to parse, fall back
    // to non-privacy mode (plaintext ClientIdentification).
    match try_enable_privacy_mode(&mut session, license_url).await {
        Ok(()) => println!("[cdrm] Privacy mode enabled"),
        Err(e) => println!("[cdrm] Privacy mode unavailable, using plaintext: {e}"),
    }

    // Build and send the license challenge
    let challenge = session
        .build_license_challenge(&pssh, drm_widevine::LicenseType::Streaming)
        .map_err(|e| anyhow!("Failed to build license challenge: {e}"))?;

    let response_bytes = license_request(license_url, challenge).await?;
    let keys = session
        .parse_license_response(&response_bytes)
        .map_err(|e| anyhow!("Failed to parse license response: {e}"))?;

    let content_keys: Vec<String> = keys
        .iter()
        .filter(|k| k.key_type == drm_widevine::KeyType::Content)
        .map(|k| format!("{}:{}", k.kid_hex(), k.key_hex()))
        .collect();

    if content_keys.is_empty() {
        return Err(anyhow!("No content keys found in license response"));
    }

    println!("[cdrm] Got {} content key(s)", content_keys.len());
    Ok(content_keys)
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
