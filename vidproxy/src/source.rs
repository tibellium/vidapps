use anyhow::{Result, anyhow};
use chrome_browser::{ChromeBrowser, ChromeLaunchOptions};

use crate::manifest::{self, ChannelEntry, DiscoveredChannel, Manifest, StreamInfo, Transform};

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY_MS: u64 = 1000;

/**
    Result of running a source - all discovered channels with their stream info.
*/
pub struct SourceResult {
    /// Source ID
    pub source_id: String,
    /// Channel entries (discovery + content info)
    pub channels: Vec<ChannelEntry>,
    /// When discovery results expire (if any)
    pub discovery_expires_at: Option<u64>,
}

/**
    Run a complete source: discovery phase, then content phase for all channels.

    This launches a browser, runs discovery in tab 0, then runs content phase
    for each channel sequentially in the same tab (required for auth to work).
*/
pub async fn run_source(manifest: &Manifest, headless: bool) -> Result<SourceResult> {
    let source_id = &manifest.source.id;
    let source_name = &manifest.source.name;
    println!("[source] Starting source: {} ({})", source_name, source_id);

    // Launch browser
    let mut options = ChromeLaunchOptions::default()
        .headless(headless)
        .devtools(false);

    if let Some(ref proxy) = manifest.source.proxy {
        options = options.proxy_server(proxy);
    }

    let browser = ChromeBrowser::new(options).await?;

    // Get tab 0 for all operations (discovery + content must share same tab for auth)
    let tab = browser
        .get_tab(0)
        .await
        .ok_or_else(|| anyhow!("No browser tab available"))?;

    // Run discovery phase
    println!("[source] Running discovery phase...");
    let discovery_result =
        manifest::execute_discovery(&manifest.discovery, &tab, source_id).await?;

    let channels = discovery_result.channels;
    println!("[source] Discovery found {} channels", channels.len());

    // Apply processing phase if present (filter + transforms)
    let channels: Vec<DiscoveredChannel> = if let Some(ref process) = manifest.process {
        // First apply filter if present
        let mut channels: Vec<_> = if let Some(ref filter) = process.filter {
            let filtered: Vec<_> = channels
                .into_iter()
                .filter(|c| {
                    // If name filter is set, check if channel name matches any
                    let name_match = filter.name.is_empty()
                        || c.name
                            .as_ref()
                            .map(|n| filter.name.contains(n))
                            .unwrap_or(false);

                    // If id filter is set, check if channel id matches any
                    let id_match = filter.id.is_empty() || filter.id.contains(&c.id);

                    // Channel passes if it matches both filters (or filter is empty)
                    name_match && id_match
                })
                .collect();

            println!(
                "[source] Filter applied: {} channels remaining",
                filtered.len()
            );
            filtered
        } else {
            channels
        };

        // Then apply transforms
        for transform in &process.transforms {
            apply_transform(&mut channels, transform);
        }

        channels
    } else {
        channels
    };

    // Run metadata phase once if present (extracts EPG for all channels from single request)
    let mut channel_programmes: std::collections::HashMap<String, Vec<crate::manifest::Programme>> =
        std::collections::HashMap::new();

    if let Some(ref metadata_phase) = manifest.metadata {
        println!("[source] Running metadata phase...");

        match manifest::execute_metadata(metadata_phase, &tab).await {
            Ok(result) => {
                channel_programmes = result.programmes_by_channel;
            }
            Err(e) => {
                eprintln!("[source] Metadata phase failed: {}", e);
                // Continue without metadata - not fatal
            }
        }
    }

    // Run content phase for each channel sequentially in the same tab
    let mut channel_entries = Vec::new();

    for channel in &channels {
        let channel_name = channel.name.as_deref().unwrap_or(&channel.id);
        println!("[source] Running content phase for: {}", channel_name);

        let mut last_error = None;
        let mut stream_info = None;

        for attempt in 1..=MAX_RETRIES {
            match manifest::execute_content(&manifest.content, &tab, channel).await {
                Ok(info) => {
                    println!("[source] Content phase completed for: {}", channel_name);
                    stream_info = Some(info);
                    break;
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    if attempt < MAX_RETRIES {
                        eprintln!(
                            "[source] Content phase failed for '{}' (attempt {}/{}): {}",
                            channel_name, attempt, MAX_RETRIES, e
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
                    } else {
                        eprintln!(
                            "[source] Content phase failed for '{}' after {} attempts: {}",
                            channel_name, MAX_RETRIES, e
                        );
                    }
                }
            }
        }

        // Get programmes for this channel if available
        let programmes = channel_programmes.remove(&channel.id).unwrap_or_default();

        channel_entries.push(ChannelEntry {
            channel: channel.clone(),
            stream_info,
            programmes,
            last_error,
        });
    }

    // Close browser
    let _ = browser.close().await;

    let successful = channel_entries
        .iter()
        .filter(|c| c.stream_info.is_some())
        .count();
    let failed = channel_entries.len() - successful;

    println!(
        "[source] Completed '{}': {} channels OK, {} failed",
        source_id, successful, failed
    );

    Ok(SourceResult {
        source_id: source_id.clone(),
        channels: channel_entries,
        discovery_expires_at: discovery_result.expires_at,
    })
}

/**
    Run content phase for a single channel (used for refresh).

    Launches a browser, runs discovery in tab 0 to establish auth, then runs content
    for the specified channel in the same tab.
*/
pub async fn refresh_channel(
    manifest: &Manifest,
    channel_id: &str,
    headless: bool,
) -> Result<StreamInfo> {
    let source_id = &manifest.source.id;
    println!(
        "[source] Refreshing channel '{}' from '{}'",
        channel_id, source_id
    );

    // Launch browser
    let mut options = ChromeLaunchOptions::default()
        .headless(headless)
        .devtools(false);

    if let Some(ref proxy) = manifest.source.proxy {
        options = options.proxy_server(proxy);
    }

    let browser = ChromeBrowser::new(options).await?;

    // Use tab 0 for all operations (discovery + content must share same tab for auth)
    let tab = browser
        .get_tab(0)
        .await
        .ok_or_else(|| anyhow!("No browser tab available"))?;

    // Run discovery to establish auth/session
    println!("[source] Running discovery for auth...");
    let discovery_result =
        manifest::execute_discovery(&manifest.discovery, &tab, source_id).await?;

    // Find the channel we want to refresh
    let channel = discovery_result
        .channels
        .iter()
        .find(|c| c.id == channel_id)
        .ok_or_else(|| anyhow!("Channel '{}' not found in discovery results", channel_id))?;

    // Run content phase for this channel in the same tab
    println!("[source] Running content phase...");
    let stream_info = manifest::execute_content(&manifest.content, &tab, channel).await?;

    // Close browser
    let _ = browser.close().await;

    Ok(stream_info)
}

/**
    Apply a transform to a list of channels.
*/
fn apply_transform(channels: &mut [DiscoveredChannel], transform: &Transform) {
    match transform {
        Transform::AddCategory { name, id, category } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.category = Some(category.clone());
                }
            }
        }
        Transform::AddDescription {
            name,
            id,
            description,
        } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.description = Some(description.clone());
                }
            }
        }
    }
}

fn channel_matches(
    channel: &DiscoveredChannel,
    name: &Option<String>,
    id: &Option<String>,
) -> bool {
    let name_matches = name
        .as_ref()
        .map(|n| channel.name.as_ref() == Some(n))
        .unwrap_or(true);
    let id_matches = id.as_ref().map(|i| &channel.id == i).unwrap_or(true);

    // If both name and id are None, match all channels
    // Otherwise, require at least one to be specified and match
    if name.is_none() && id.is_none() {
        true
    } else {
        (name.is_some() && name_matches) || (id.is_some() && id_matches)
    }
}
