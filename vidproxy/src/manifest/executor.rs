use anyhow::{Result, anyhow};
use chrome_browser::{ChromeBrowser, ChromeBrowserTab, NetworkRequestStream};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::extractors::extract;
use super::interpolate::InterpolationContext;
use super::types::{Manifest, ManifestOutputs, Step, StepKind};

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

/// Execute a manifest using the given browser.
pub async fn execute(manifest: &Manifest, browser: &ChromeBrowser) -> Result<ManifestOutputs> {
    let tab = browser
        .get_tab(0)
        .await
        .ok_or_else(|| anyhow!("No browser tab available"))?;

    let mut context = InterpolationContext::new();

    // Start monitoring network requests
    let mut requests = tab.network().requests();

    for step in &manifest.steps {
        println!("[executor] Running step: {}", step.name);

        match step.kind {
            StepKind::Navigate => {
                execute_navigate(step, &tab, &context).await?;
            }
            StepKind::Sniff => {
                execute_sniff(step, &mut requests, &mut context).await?;
            }
            StepKind::CdrmRequest => {
                execute_cdrm_request(step, &manifest.channel.name, &mut context).await?;
            }
        }
    }

    // Resolve final outputs
    let mpd_url = context.interpolate(&manifest.outputs.mpd_url)?;
    let decryption_key = context.interpolate(&manifest.outputs.decryption_key)?;
    let thumbnail_url = manifest
        .outputs
        .thumbnail_url
        .as_ref()
        .map(|t| context.interpolate(t))
        .transpose()?;

    Ok(ManifestOutputs {
        mpd_url,
        decryption_key,
        thumbnail_url,
    })
}

/// Execute a Navigate step.
async fn execute_navigate(
    step: &Step,
    tab: &ChromeBrowserTab,
    context: &InterpolationContext,
) -> Result<()> {
    let url_template = step
        .url
        .as_ref()
        .ok_or_else(|| anyhow!("Navigate step '{}' requires 'url'", step.name))?;

    let url = context.interpolate(url_template)?;
    println!("[executor] Navigating to: {}", url);
    tab.navigate(&url).await?;

    // Wait for condition if specified
    if let Some(wait_for) = &step.wait_for {
        if let Some(selector) = &wait_for.selector {
            println!("[executor] Waiting for selector: {}", selector);
            tab.wait_for_selector(selector).await?;
        }
        if let Some(expr) = &wait_for.function {
            println!("[executor] Waiting for function: {}", expr);
            tab.wait_for_function(expr).await?;
        }
        if let Some(delay) = wait_for.delay {
            println!("[executor] Waiting {} seconds", delay);
            tokio::time::sleep(std::time::Duration::from_secs_f64(delay)).await;
        }
    }

    Ok(())
}

/// Execute a Sniff step.
async fn execute_sniff(
    step: &Step,
    requests: &mut NetworkRequestStream,
    context: &mut InterpolationContext,
) -> Result<()> {
    use std::time::Duration;

    let request_match = step
        .request
        .as_ref()
        .ok_or_else(|| anyhow!("Sniff step '{}' requires 'request'", step.name))?;

    let url_pattern = &request_match.url;
    let method_filter = request_match.method.as_deref();
    let timeout_secs = request_match.timeout.unwrap_or(30.0);

    let url_regex = Regex::new(url_pattern)
        .map_err(|e| anyhow!("Invalid URL regex '{}': {}", url_pattern, e))?;

    println!(
        "[executor] Waiting for request matching: {} (timeout: {}s)",
        url_pattern, timeout_secs
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);

    // Wait for matching request
    loop {
        let next_request = tokio::time::timeout_at(deadline, requests.next()).await;

        let request = match next_request {
            Ok(Some(req)) => req,
            Ok(None) => {
                return Err(anyhow!(
                    "Network stream closed before finding match for step '{}'",
                    step.name
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "Timeout waiting for request matching '{}' in step '{}'",
                    url_pattern,
                    step.name
                ));
            }
        };

        let url = request.url().to_string();
        let method = request.method();

        // Check URL pattern (regex)
        if !url_regex.is_match(&url) {
            continue;
        }

        // Check method filter
        if let Some(expected_method) = method_filter
            && method.as_str() != expected_method
        {
            continue;
        }

        println!("[executor] Matched request: {}", &url[..url.len().min(80)]);

        // Get response body
        let body = if let Ok(response) = request.response().await {
            response.text().await.unwrap_or_default()
        } else {
            String::new()
        };

        // Run extractors - all must succeed for this request to be accepted
        let mut extracted = std::collections::HashMap::new();
        let mut all_succeeded = true;

        for (output_name, extractor) in &step.extract {
            match extract(extractor, &body, &url) {
                Ok(value) => {
                    extracted.insert(output_name.clone(), value);
                }
                Err(_) => {
                    all_succeeded = false;
                    break;
                }
            }
        }

        if all_succeeded {
            // All extractors succeeded, commit the results
            for (output_name, value) in extracted {
                println!("[executor] Extracted {}.{}", step.name, output_name);
                context.set(&step.name, &output_name, value);
            }
            return Ok(());
        }

        // Extraction failed, try next matching request
        println!("[executor] Extraction failed, trying next request...");
    }
}

/// Execute a CdrmRequest step.
async fn execute_cdrm_request(
    step: &Step,
    _channel_name: &str,
    context: &mut InterpolationContext,
) -> Result<()> {
    let pssh_template = step
        .pssh
        .as_ref()
        .ok_or_else(|| anyhow!("CdrmRequest step '{}' requires 'pssh'", step.name))?;

    let license_url_template = step
        .license_url
        .as_ref()
        .ok_or_else(|| anyhow!("CdrmRequest step '{}' requires 'license_url'", step.name))?;

    let pssh = context.interpolate(pssh_template)?;
    let license_url = context.interpolate(license_url_template)?;

    println!("[executor] Requesting decryption keys from CDRM API...");

    let client = reqwest::Client::new();
    let cdrm_req = CdrmRequest {
        pssh,
        licurl: license_url,
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

    // Run extractors on the response
    for (output_name, extractor) in &step.extract {
        match extract(extractor, &cdrm_resp.message, "") {
            Ok(value) => {
                println!("[executor] Extracted {}.{}", step.name, output_name);
                context.set(&step.name, output_name, value);
            }
            Err(e) => {
                return Err(anyhow!(
                    "Failed to extract {}.{}: {}",
                    step.name,
                    output_name,
                    e
                ));
            }
        }
    }

    Ok(())
}
