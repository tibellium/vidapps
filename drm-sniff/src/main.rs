use anyhow::{anyhow, Result};
use base64::{prelude::BASE64_STANDARD, Engine};
use chrome_browser::{ChromeBrowser, ChromeLaunchOptions};
use serde::{Deserialize, Serialize};

const HOME_URL: &str = "https://www.canalrcn.com";
const TARGET_TITLE: &str = "Señal Principal";
const CDRM_API_URL: &str = "https://cdrm-project.com/api/decrypt";

#[derive(Debug, Deserialize)]
struct ApiResponse {
    result: Vec<ContentItem>,
}

#[derive(Debug, Deserialize)]
struct ContentItem {
    id: String,
    content: ContentInfo,
}

#[derive(Debug, Deserialize)]
struct ContentInfo {
    title: String,
}

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

/// Extract PSSH box (base64) from MPD content
fn extract_pssh(mpd: &str) -> Option<String> {
    for line in mpd.lines() {
        if line.contains("cenc:pssh") || line.contains("<pssh>") {
            // Extract base64 content between tags
            if let Some(start) = line.find('>') {
                if let Some(end) = line[start + 1..].find('<') {
                    let pssh = &line[start + 1..start + 1 + end];
                    if !pssh.is_empty()
                        && pssh
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
                    {
                        return Some(pssh.to_string());
                    }
                }
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("Launching Chrome with SOCKS5 proxy...");

    let options = ChromeLaunchOptions::default()
        .headless(false)
        .devtools(false)
        .proxy_server("socks5://127.0.0.1:1080");

    let browser = ChromeBrowser::new(options).await?;

    println!("Getting tab...");
    let tab = browser.get_tab(0).await.ok_or_else(|| anyhow!("no tab"))?;

    // Start monitoring network requests BEFORE navigating
    println!("Starting network monitor...");
    let mut requests = tab.network().requests();

    // Navigate to homepage
    println!("Navigating to: {}", HOME_URL);
    tab.navigate(HOME_URL).await?;

    // Phase 1: Find the content ID by sniffing API responses
    println!(
        "\nPhase 1: Looking for '{}' in API responses...\n",
        TARGET_TITLE
    );

    let content_id = loop {
        let Some(request) = requests.next().await else {
            return Err(anyhow!("Network stream closed before finding content"));
        };

        let url = request.url().to_string();

        // Look for the unity.tbxapis.com items endpoint
        if url.contains("unity.tbxapis.com") && url.contains("/items/") && url.contains(".json") {
            println!("[API] {}", &url[..url.len().min(100)]);

            if let Ok(response) = request.response().await {
                if let Ok(body) = response.text().await {
                    // Try to parse as our expected format
                    if let Ok(api_response) = serde_json::from_str::<ApiResponse>(&body) {
                        for item in &api_response.result {
                            println!("  Found: {} (id: {})", item.content.title, item.id);
                            if item.content.title == TARGET_TITLE {
                                println!("\n*** Found target content! ID: {} ***\n", item.id);
                                break;
                            }
                        }
                        // Check if we found it
                        if let Some(item) = api_response
                            .result
                            .iter()
                            .find(|i| i.content.title == TARGET_TITLE)
                        {
                            break item.id.clone();
                        }
                    }
                }
            }
        }
    };

    // Phase 2: Navigate to the player page
    let player_url = format!("https://www.canalrcn.com/co/player/{}", content_id);
    println!("Phase 2: Navigating to player: {}", player_url);
    tab.navigate(&player_url).await?;

    // Phase 3: Monitor for DRM-related requests and extract keys
    println!("\nPhase 3: Monitoring for DRM/license requests...\n");

    let mut pssh: Option<String> = None;
    let mut mpd_url: Option<String> = None;
    let mut license_url: Option<String> = None;

    while let Some(request) = requests.next().await {
        let url = request.url().to_string();
        let method = request.method().clone();

        let is_license = url.contains("license") && url.contains("widevine");
        let is_mpd = url.contains(".mpd");

        if is_mpd && pssh.is_none() {
            // Capture MPD manifest (contains PSSH boxes)
            if let Ok(response) = request.response().await {
                if let Ok(body) = response.text().await {
                    println!("=== DASH MANIFEST (.mpd) ===");
                    println!("URL: {}", url);

                    if let Some(extracted_pssh) = extract_pssh(&body) {
                        println!("PSSH: {}", extracted_pssh);

                        // Decode and show KID
                        if let Ok(decoded) = BASE64_STANDARD.decode(&extracted_pssh) {
                            if decoded.len() >= 52 {
                                let kid: String = decoded[32..48]
                                    .iter()
                                    .map(|b| format!("{:02x}", b))
                                    .collect();
                                println!("KID: {}", kid);
                            }
                        }

                        pssh = Some(extracted_pssh);
                        mpd_url = Some(url.clone());

                        // Save MPD URL to file
                        if std::fs::write("mpd_url.txt", &url).is_ok() {
                            println!("Saved MPD URL to: mpd_url.txt");
                        }

                        // Save MPD to file
                        if std::fs::write("manifest.mpd", &body).is_ok() {
                            println!("Saved manifest to: manifest.mpd");
                        }
                    }
                    println!();
                }
            }
        } else if is_license && method == "POST" && license_url.is_none() {
            println!("=== LICENSE URL ===");
            println!("URL: {}", url);
            license_url = Some(url.clone());
            println!();

            // Once we have both PSSH and license URL, get the keys
            if let (Some(ref p), Some(ref l)) = (&pssh, &license_url) {
                println!("=== REQUESTING DECRYPTION KEYS FROM CDRM ===\n");

                let client = reqwest::Client::new();
                let cdrm_req = CdrmRequest {
                    pssh: p.clone(),
                    licurl: l.clone(),
                    headers: format!(
                        "{:?}",
                        std::collections::HashMap::from([
                            ("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"),
                            ("Accept", "*/*"),
                            ("Origin", "https://www.canalrcn.com"),
                            ("Referer", "https://www.canalrcn.com/"),
                        ])
                    ),
                };

                println!("PSSH: {}", cdrm_req.pssh);
                println!("License URL: {}", cdrm_req.licurl);
                println!();

                match client.post(CDRM_API_URL).json(&cdrm_req).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            match resp.json::<CdrmResponse>().await {
                                Ok(cdrm_resp) => {
                                    println!("=== DECRYPTION KEYS ===");
                                    println!("{}", cdrm_resp.message);

                                    // Save keys to file
                                    if std::fs::write("keys.txt", &cdrm_resp.message).is_ok() {
                                        println!("\nSaved keys to: keys.txt");
                                    }

                                    // Test FFmpeg decryption
                                    if let Some(ref mpd) = mpd_url {
                                        println!("\n=== TESTING FFMPEG DECRYPTION ===\n");

                                        // Parse keys (format: "kid:key\nkid:key\n...")
                                        let keys: Vec<&str> = cdrm_resp
                                            .message
                                            .lines()
                                            .filter(|l| l.contains(':'))
                                            .collect();

                                        if let Some(first_key_pair) = keys.first() {
                                            // Extract just the key (after the colon)
                                            let key_only = first_key_pair
                                                .split(':')
                                                .nth(1)
                                                .unwrap_or(first_key_pair);

                                            println!("Using key: {}", key_only);
                                            println!("MPD URL: {}", mpd);

                                            // Run FFmpeg to test decryption (5 seconds of video)
                                            // Use -cenc_decryption_key (the actual FFmpeg option name)
                                            let output = std::process::Command::new("ffmpeg")
                                                .args([
                                                    "-y",
                                                    "-cenc_decryption_key",
                                                    key_only,
                                                    "-i",
                                                    mpd,
                                                    "-t",
                                                    "5",
                                                    "-c",
                                                    "copy",
                                                    "test_output.ts",
                                                ])
                                                .output();

                                            match output {
                                                Ok(out) => {
                                                    if out.status.success() {
                                                        println!("FFmpeg decryption SUCCESS!");
                                                        println!("Output saved to: test_output.ts");
                                                    } else {
                                                        println!("FFmpeg decryption FAILED:");
                                                        println!(
                                                            "{}",
                                                            String::from_utf8_lossy(&out.stderr)
                                                        );
                                                    }
                                                }
                                                Err(e) => println!("Failed to run FFmpeg: {}", e),
                                            }
                                        }
                                    }
                                }
                                Err(e) => println!("Failed to parse CDRM response: {}", e),
                            }
                        } else {
                            println!("CDRM API error: {}", resp.status());
                            if let Ok(text) = resp.text().await {
                                println!("Response: {}", text);
                            }
                        }
                    }
                    Err(e) => println!("Failed to contact CDRM: {}", e),
                }

                // We're done
                break;
            }
        }
    }

    browser.close().await?;
    Ok(())
}
