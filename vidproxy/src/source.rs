use anyhow::{Result, anyhow};
use chrome_browser::{ChromeBrowser, ChromeBrowserTab, ChromeLaunchOptions};

use crate::manifest::{self, ChannelEntry, DiscoveredChannel, Manifest, StreamInfo, Transform};

/**
    Create a browser instance configured for a manifest's source.
*/
pub async fn create_browser(manifest: &Manifest) -> Result<ChromeBrowser> {
    let headless = manifest.source.headless;
    let mut options = ChromeLaunchOptions::default()
        .headless(headless)
        .devtools(false)
        .enable_gpu(headless); // Enable GPU acceleration in headless mode

    if let Some(ref proxy) = manifest.source.proxy {
        options = options.proxy_server(proxy);
    }

    ChromeBrowser::new(options).await
}

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

    NOTE: This is no longer used at startup (we use run_source_discovery_only instead),
    but kept for potential testing or future use.
*/
#[allow(dead_code)]
pub async fn run_source(manifest: &Manifest, headless: bool) -> Result<SourceResult> {
    const MAX_RETRIES: u32 = 3;
    const RETRY_DELAY_MS: u64 = 1000;

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
    Run discovery-only for a source (no content phase).

    This is used for fast startup - channels are registered with stream_info: None,
    and content is resolved on-demand when a channel is first requested.

    The browser is passed in and kept alive for later content resolution.
*/
pub async fn run_source_discovery_only(
    manifest: &Manifest,
    browser: &ChromeBrowser,
) -> Result<SourceResult> {
    let source_id = &manifest.source.id;
    let source_name = &manifest.source.name;
    println!(
        "[source] Starting source (discovery-only): {} ({})",
        source_name, source_id
    );

    // Get tab 0 for all operations
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
                    let name_match = filter.name.is_empty()
                        || c.name
                            .as_ref()
                            .map(|n| filter.name.contains(n))
                            .unwrap_or(false);
                    let id_match = filter.id.is_empty() || filter.id.contains(&c.id);
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
            }
        }
    }

    // Navigate to blank page to stop any streaming and save bandwidth
    let _ = tab.navigate("about:blank").await;

    // Create channel entries with stream_info: None (content resolved on-demand)
    let channel_entries: Vec<ChannelEntry> = channels
        .into_iter()
        .map(|channel| {
            let programmes = channel_programmes.remove(&channel.id).unwrap_or_default();
            ChannelEntry {
                channel,
                stream_info: None, // Resolved on-demand
                programmes,
                last_error: None,
            }
        })
        .collect();

    println!(
        "[source] Discovery-only completed '{}': {} channels (content on-demand)",
        source_id,
        channel_entries.len()
    );

    Ok(SourceResult {
        source_id: source_id.clone(),
        channels: channel_entries,
        discovery_expires_at: discovery_result.expires_at,
    })
}

/**
    Resolve content phase for a channel using an existing browser tab.

    This is used for on-demand content resolution after discovery has already run.
    Auth is already established from discovery, so we just run the content phase.
*/
pub async fn resolve_channel_content(
    manifest: &Manifest,
    channel: &DiscoveredChannel,
    tab: &ChromeBrowserTab,
) -> Result<StreamInfo> {
    let channel_name = channel.name.as_deref().unwrap_or(&channel.id);
    println!("[source] Resolving content for '{}'...", channel_name);

    // Run content phase using the channel data we already have
    let stream_info = manifest::execute_content(&manifest.content, tab, channel).await?;

    println!(
        "[source] Content resolved for '{}': {}",
        channel_name, stream_info.manifest_url
    );

    // Navigate to blank page to stop any streaming and save bandwidth
    let _ = tab.navigate("about:blank").await;

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
        Transform::Rename { name, id, to } => {
            for channel in channels.iter_mut() {
                if channel_matches(channel, name, id) {
                    channel.name = Some(to.clone());
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
