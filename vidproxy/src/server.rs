use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{Duration, Utc};
use tokio::sync::{RwLock, watch};
use tokio_util::io::ReaderStream;

use crate::image_cache::ImageCache;
use crate::manifest::Manifest;
use crate::pipeline::PipelineStore;
use crate::registry::{ChannelContentState, ChannelId, ChannelRegistry, SourceState};
use crate::source;

/**
    Default timeout for waiting on source discovery (60 seconds)
*/
const SOURCE_WAIT_TIMEOUT: StdDuration = StdDuration::from_secs(60);

/**
    Default timeout for waiting on channel content resolution (120 seconds)
*/
const CONTENT_WAIT_TIMEOUT: StdDuration = StdDuration::from_secs(120);

/**
    Wait for a source to be ready, returning appropriate error if not.
    - Returns Ok(()) if the source is ready
    - Returns Err(NOT_FOUND) if the source doesn't exist
    - Returns Err(SERVICE_UNAVAILABLE) if the source failed
    - Returns Err(GATEWAY_TIMEOUT) if waiting timed out
*/
async fn wait_for_source_ready(
    registry: &ChannelRegistry,
    source_id: &str,
) -> Result<(), StatusCode> {
    match registry
        .wait_for_source(source_id, SOURCE_WAIT_TIMEOUT)
        .await
    {
        Some(SourceState::Ready) => Ok(()),
        Some(SourceState::Failed(err)) => {
            eprintln!("[server] Source '{}' failed: {}", source_id, err);
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
        Some(SourceState::Loading) => {
            // Timed out while still loading
            eprintln!(
                "[server] Timeout waiting for source '{}' to load",
                source_id
            );
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
        None => {
            // Unknown source
            Err(StatusCode::NOT_FOUND)
        }
    }
}

/**
    Extract the base URL (scheme + host) from request headers.

    Checks X-Forwarded-Proto for the scheme (used by reverse proxies like Cloudflare).
*/
fn get_base_url(headers: &HeaderMap) -> String {
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");

    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");

    format!("{scheme}://{host}")
}

/**
    Store for loaded manifests and their associated browsers, keyed by source name
*/
pub struct ManifestStore {
    manifests: RwLock<HashMap<String, Manifest>>,
    browsers: RwLock<HashMap<String, chrome_browser::ChromeBrowser>>,
}

impl ManifestStore {
    pub fn new() -> Self {
        Self {
            manifests: RwLock::new(HashMap::new()),
            browsers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add(&self, manifest: Manifest) {
        let mut manifests = self.manifests.write().await;
        manifests.insert(manifest.source.id.clone(), manifest);
    }

    pub async fn get(&self, source: &str) -> Option<Manifest> {
        self.manifests.read().await.get(source).cloned()
    }

    pub async fn list(&self) -> Vec<Manifest> {
        self.manifests.read().await.values().cloned().collect()
    }

    /**
        Store a browser instance for a source
    */
    pub async fn set_browser(&self, source: &str, browser: chrome_browser::ChromeBrowser) {
        let mut browsers = self.browsers.write().await;
        browsers.insert(source.to_string(), browser);
    }

    /**
        Get the browser instance for a source (cloning is cheap - it's Arc-based)
    */
    pub async fn get_browser(&self, source: &str) -> Option<chrome_browser::ChromeBrowser> {
        self.browsers.read().await.get(source).cloned()
    }

    /**
        Get tab 0 from the browser for a source
    */
    pub async fn get_browser_tab(&self, source: &str) -> Option<chrome_browser::ChromeBrowserTab> {
        let browsers = self.browsers.read().await;
        if let Some(browser) = browsers.get(source) {
            browser.get_tab(0).await
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct AppState {
    registry: Arc<ChannelRegistry>,
    pipeline_store: Arc<PipelineStore>,
    manifest_store: Arc<ManifestStore>,
    image_cache: Arc<ImageCache>,
}

/**
    Root endpoint - list all available sources with links.
*/
async fn index(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let base_url = get_base_url(&headers);

    let manifests = state.manifest_store.list().await;

    let sources: Vec<serde_json::Value> = manifests
        .iter()
        .map(|m| {
            let source_state = state.registry.get_source_state(&m.source.id);
            let status = match source_state {
                Some(SourceState::Ready) => "ready",
                Some(SourceState::Loading) => "loading",
                Some(SourceState::Failed(_)) => "failed",
                None => "unknown",
            };

            serde_json::json!({
                "id": m.source.id,
                "name": m.source.name,
                "status": status,
                "info": format!("{}/{}/info", base_url, m.source.id),
                "m3u": format!("{}/{}/channels.m3u", base_url, m.source.id),
                "epg": format!("{}/{}/epg.xml", base_url, m.source.id),
            })
        })
        .collect();

    let json = serde_json::json!({
        "sources": sources,
    });

    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json.to_string(),
    )
}

/**
    Get source info (JSON).
*/
async fn source_info(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let manifest = state
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let base_url = get_base_url(&headers);

    let source_state = state.registry.get_source_state(&source_id);
    let status = match &source_state {
        Some(SourceState::Ready) => "ready",
        Some(SourceState::Loading) => "loading",
        Some(SourceState::Failed(_)) => "failed",
        None => "unknown",
    };

    let error = match &source_state {
        Some(SourceState::Failed(err)) => Some(err.clone()),
        _ => None,
    };

    let channels = state.registry.list_by_source(&source_id);
    let channel_list: Vec<serde_json::Value> = channels
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.channel.id,
                "name": e.channel.name,
                "info": format!("{}/{}/{}/info", base_url, source_id, e.channel.id),
                "image": if e.channel.image.is_some() {
                    Some(format!("{}/{}/{}/image", base_url, source_id, e.channel.id))
                } else {
                    None
                },
                "playlist": format!("{}/{}/{}/playlist.m3u8", base_url, source_id, e.channel.id),
                "resolved": e.stream_info.is_some(),
            })
        })
        .collect();

    let json = serde_json::json!({
        "id": manifest.source.id,
        "name": manifest.source.name,
        "status": status,
        "error": error,
        "m3u": format!("{}/{}/channels.m3u", base_url, source_id),
        "epg": format!("{}/{}/epg.xml", base_url, source_id),
        "channels": channel_list,
    });

    Ok((
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json.to_string(),
    ))
}

/**
    Generate M3U playlist with channels from a specific source.
*/
async fn source_m3u(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    // Wait for source to be ready
    wait_for_source_ready(&state.registry, &source_id).await?;

    let manifest = state
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let channels = state.registry.list_by_source(&source_id);
    if channels.is_empty() {
        // Source is ready but has no channels (all failed during content phase)
        return Err(StatusCode::NOT_FOUND);
    }

    let base_url = get_base_url(&headers);

    let mut playlist = format!("#EXTM3U url-tvg=\"{}/{}/epg.xml\"\n", base_url, source_id);

    for entry in &channels {
        // Include all channels - content will be resolved on-demand when played
        let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);

        // Use local image URL if channel has an image
        let logo_attr = if entry.channel.image.is_some() {
            format!(
                " tvg-logo=\"{}/{}/{}/image\"",
                base_url, source_id, entry.channel.id
            )
        } else {
            String::new()
        };

        // Add country attribute if configured
        let country_attr = manifest
            .source
            .country
            .as_ref()
            .map(|c| format!(" tvg-country=\"{}\"", escape_xml(c)))
            .unwrap_or_default();

        // Add language attribute if configured
        let language_attr = manifest
            .source
            .language
            .as_ref()
            .map(|l| format!(" tvg-language=\"{}\"", escape_xml(l)))
            .unwrap_or_default();

        let channel_id = format!("{}:{}", source_id, entry.channel.id);

        // Use channel category if set, otherwise fall back to source name
        let group = entry
            .channel
            .category
            .as_ref()
            .unwrap_or(&manifest.source.name);

        playlist.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{id}\" tvg-name=\"{name}\" tvg-type=\"live\" group-title=\"{group}\"{logo}{country}{language},{name}\n\
             {base_url}/{source}/{channel}/playlist.m3u8\n",
            id = escape_xml(&channel_id),
            name = escape_xml(channel_name),
            group = escape_xml(group),
            logo = logo_attr,
            country = country_attr,
            language = language_attr,
            base_url = base_url,
            source = source_id,
            channel = entry.channel.id,
        ));
    }

    Ok(([(header::CONTENT_TYPE, "audio/x-mpegurl")], playlist))
}

/**
    Generate XMLTV EPG data for channels from a specific source.
*/
async fn source_epg(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    // Wait for source to be ready
    wait_for_source_ready(&state.registry, &source_id).await?;

    let manifest = state
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let channels = state.registry.list_by_source(&source_id);
    if channels.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let base_url = get_base_url(&headers);

    let now = Utc::now();
    let start_of_day = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let start = start_of_day.and_utc();

    let mut channel_elements = String::new();
    let mut programmes = String::new();

    // Language attribute for titles/descriptions if configured
    let lang_attr = manifest
        .source
        .language
        .as_ref()
        .map(|l| format!(" lang=\"{}\"", escape_xml(l)))
        .unwrap_or_default();

    for entry in &channels {
        // Include all channels - EPG data comes from metadata phase, not content phase

        let channel_name = entry.channel.name.as_deref().unwrap_or(&entry.channel.id);
        let channel_id = format!("{}:{}", source_id, entry.channel.id);

        // Use local image URL if channel has an image
        let icon_element = if entry.channel.image.is_some() {
            format!(
                "    <icon src=\"{}/{}/{}/image\"/>\n",
                base_url, source_id, entry.channel.id
            )
        } else {
            String::new()
        };

        channel_elements.push_str(&format!(
            "  <channel id=\"{id}\">\n\
             \x20   <display-name{lang}>{name}</display-name>\n\
             {icon}\
             \x20 </channel>\n",
            id = escape_xml(&channel_id),
            name = escape_xml(channel_name),
            lang = lang_attr,
            icon = icon_element,
        ));

        // Build category element if channel has a category
        let category_element = entry
            .channel
            .category
            .as_ref()
            .map(|c| format!("    <category{}>{}</category>\n", lang_attr, escape_xml(c)))
            .unwrap_or_default();

        // Use real programme data if available, otherwise generate placeholder
        if entry.programmes.is_empty() {
            // Use channel description if available, otherwise default
            let desc = entry
                .channel
                .description
                .as_deref()
                .unwrap_or("Live broadcast");

            // Generate 7 days of placeholder programming
            for day in 0..7 {
                let day_start = start + Duration::days(day);
                let day_end = day_start + Duration::days(1);

                programmes.push_str(&format!(
                    "  <programme start=\"{start}\" stop=\"{stop}\" channel=\"{id}\">\n\
                     \x20   <title{lang}>{name}</title>\n\
                     \x20   <desc{lang}>{desc}</desc>\n\
                     {category}\
                     \x20 </programme>\n",
                    start = day_start.format("%Y%m%d%H%M%S %z"),
                    stop = day_end.format("%Y%m%d%H%M%S %z"),
                    id = escape_xml(&channel_id),
                    category = category_element,
                    name = escape_xml(channel_name),
                    desc = escape_xml(desc),
                    lang = lang_attr,
                ));
            }
        } else {
            // Use real programme data from metadata phase
            for programme in &entry.programmes {
                // Parse ISO 8601 timestamps and convert to XMLTV format
                let start_formatted = format_xmltv_time(&programme.start_time);
                let stop_formatted = format_xmltv_time(&programme.end_time);

                // Build description element
                let desc_element = programme
                    .description
                    .as_ref()
                    .map(|d| format!("    <desc{}>{}</desc>\n", lang_attr, escape_xml(d)))
                    .unwrap_or_default();

                // Build category elements - use programme genres if available, otherwise channel category
                let category_elements: String = if programme.genres.is_empty() {
                    // Fall back to channel category
                    category_element.clone()
                } else {
                    programme
                        .genres
                        .iter()
                        .map(|g| {
                            format!("    <category{}>{}</category>\n", lang_attr, escape_xml(g))
                        })
                        .collect()
                };

                // Build episode-num element if available
                let episode_element = match (&programme.season, &programme.episode) {
                    (Some(s), Some(e)) => {
                        format!(
                            "    <episode-num system=\"onscreen\">S{}E{}</episode-num>\n",
                            s, e
                        )
                    }
                    (None, Some(e)) => {
                        format!(
                            "    <episode-num system=\"onscreen\">E{}</episode-num>\n",
                            e
                        )
                    }
                    _ => String::new(),
                };

                // Build icon element if programme has image (proxied through our server)
                let prog_icon = if let Some(url) = &programme.image {
                    let image_id = state.image_cache.register_proxy_url(url).await;
                    format!("    <icon src=\"{}/i/{}\"/>\n", base_url, image_id)
                } else {
                    String::new()
                };

                programmes.push_str(&format!(
                    "  <programme start=\"{start}\" stop=\"{stop}\" channel=\"{id}\">\n\
                     \x20   <title{lang}>{title}</title>\n\
                     {desc}\
                     {categories}\
                     {episode}\
                     {icon}\
                     \x20 </programme>\n",
                    start = start_formatted,
                    stop = stop_formatted,
                    id = escape_xml(&channel_id),
                    title = escape_xml(&programme.title),
                    lang = lang_attr,
                    desc = desc_element,
                    categories = category_elements,
                    episode = episode_element,
                    icon = prog_icon,
                ));
            }
        }
    }

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE tv SYSTEM \"xmltv.dtd\">\n\
         <tv generator-info-name=\"vidproxy\">\n\
         {channels}\
         {programmes}\
         </tv>\n",
        channels = channel_elements,
        programmes = programmes,
    );

    Ok(([(header::CONTENT_TYPE, "application/xml")], xml))
}

/**
    Resolve content (stream info) for a channel on-demand.

    This handles concurrency by tracking resolution state:
    - If no resolution is in progress, starts one
    - If another request is already resolving, waits for it to complete
*/
async fn resolve_channel_content(
    state: &AppState,
    id: &ChannelId,
    source_id: &str,
) -> Result<crate::manifest::StreamInfo, StatusCode> {
    // Check current content resolution state
    match state.registry.get_channel_content_state(id) {
        ChannelContentState::Resolving => {
            // Another request is already resolving - wait for it
            println!(
                "[server] Waiting for content resolution of {}...",
                id.to_string()
            );

            match state
                .registry
                .wait_for_channel_content(id, CONTENT_WAIT_TIMEOUT)
                .await
            {
                Some(ChannelContentState::Resolved) => {
                    // Content was resolved - get updated entry
                    let entry = state.registry.get(id).ok_or(StatusCode::NOT_FOUND)?;
                    entry.stream_info.ok_or_else(|| {
                        eprintln!(
                            "[server] Content resolved but stream_info still None for {}",
                            id.to_string()
                        );
                        StatusCode::SERVICE_UNAVAILABLE
                    })
                }
                Some(ChannelContentState::Failed(err)) => {
                    eprintln!(
                        "[server] Content resolution failed for {}: {}",
                        id.to_string(),
                        err
                    );
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                }
                _ => {
                    eprintln!(
                        "[server] Timeout waiting for content resolution of {}",
                        id.to_string()
                    );
                    Err(StatusCode::GATEWAY_TIMEOUT)
                }
            }
        }
        ChannelContentState::Pending | ChannelContentState::Failed(_) => {
            // We need to resolve it
            println!(
                "[server] Resolving content on-demand for {}...",
                id.to_string()
            );

            // Mark as resolving to prevent duplicate work
            state.registry.mark_channel_resolving(id);

            // Get manifest for this source
            let manifest = state.manifest_store.get(source_id).await.ok_or_else(|| {
                eprintln!("[server] No manifest found for source '{}'", source_id);
                state.registry.mark_channel_failed(id, "No manifest found");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

            // Get browser tab for this source
            let tab = state
                .manifest_store
                .get_browser_tab(source_id)
                .await
                .ok_or_else(|| {
                    eprintln!("[server] No browser found for source '{}'", source_id);
                    state
                        .registry
                        .mark_channel_failed(id, "No browser available");
                    StatusCode::SERVICE_UNAVAILABLE
                })?;

            // Get channel data from registry
            let entry = state.registry.get(id).ok_or_else(|| {
                state.registry.mark_channel_failed(id, "Channel not found");
                StatusCode::NOT_FOUND
            })?;

            // Run content phase for this channel using the existing browser
            match source::resolve_channel_content(&manifest, &entry.channel, &tab).await {
                Ok(stream_info) => {
                    println!(
                        "[server] Content resolved for {}: {}",
                        id.to_string(),
                        stream_info.manifest_url
                    );

                    // Update registry
                    state.registry.update_stream_info(id, stream_info.clone());
                    state.registry.mark_channel_resolved(id);

                    // Update pipeline if it exists (for refresh case)
                    if let Some(pipeline) = state.pipeline_store.get(id).await {
                        pipeline.update_stream_info(stream_info.clone()).await;
                        pipeline.stop().await;
                    }

                    Ok(stream_info)
                }
                Err(e) => {
                    eprintln!(
                        "[server] Failed to resolve content for {}: {}",
                        id.to_string(),
                        e
                    );
                    state.registry.set_error(id, e.to_string());
                    state.registry.mark_channel_failed(id, &e.to_string());
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                }
            }
        }
        ChannelContentState::Resolved => {
            // Content was already resolved - this shouldn't normally happen
            // since we check stream_info first, but handle it anyway
            let entry = state.registry.get(id).ok_or(StatusCode::NOT_FOUND)?;
            entry.stream_info.ok_or(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/**
    Serve the HLS playlist for a channel, starting the pipeline if needed.
*/
async fn stream_playlist(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    // Wait for source to be ready
    wait_for_source_ready(&state.registry, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);

    // Check if discovery has expired for this source - if so, re-run discovery only
    if state.registry.is_discovery_expired(&source_id) {
        println!(
            "[server] Discovery expired for source '{}', refreshing...",
            source_id
        );

        if let Some(manifest) = state.manifest_store.get(&source_id).await
            && let Some(browser) = state.manifest_store.get_browser(&source_id).await
        {
            match source::run_source_discovery_only(&manifest, &browser).await {
                Ok(result) => {
                    state.registry.register_source(
                        &result.source_id,
                        result.channels,
                        result.discovery_expires_at,
                    );
                    println!("[server] Refreshed source '{}'", source_id);
                }
                Err(e) => {
                    eprintln!("[server] Failed to refresh source '{}': {}", source_id, e);
                    // Continue with existing data
                }
            }
        }
    }

    // Check if channel exists
    let entry = state.registry.get(&id).ok_or(StatusCode::NOT_FOUND)?;

    // Check if pipeline exists and needs refresh due to auth error
    let pipeline_needs_refresh = if let Some(pipeline) = state.pipeline_store.get(&id).await {
        pipeline.needs_refresh()
    } else {
        false
    };

    // Resolve stream info - either from cache, on-demand, or refresh
    let stream_info = if let Some(ref existing) = entry.stream_info {
        // Stream info exists - check if it needs refresh
        if state.registry.is_stream_expired(&id) || pipeline_needs_refresh {
            if pipeline_needs_refresh {
                println!(
                    "[server] Pipeline auth error for {}, refreshing...",
                    id.to_string()
                );
            } else {
                println!(
                    "[server] Stream info expired for {}, refreshing...",
                    id.to_string()
                );
            }

            // Reset content state so we can re-resolve
            state.registry.reset_channel_content_state(&id);

            resolve_channel_content(&state, &id, &source_id).await?
        } else {
            // Use existing valid stream info
            existing.clone()
        }
    } else {
        // No stream info - resolve on-demand
        resolve_channel_content(&state, &id, &source_id).await?
    };

    // Get or create pipeline for this channel
    let pipeline = state
        .pipeline_store
        .get_or_create(&id, &stream_info)
        .await
        .map_err(|e| {
            eprintln!(
                "[server] Failed to create pipeline for {}: {}",
                id.to_string(),
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Ensure pipeline is running
    pipeline.ensure_running().await.map_err(|e| {
        eprintln!(
            "[server] Failed to start pipeline for {}: {}",
            id.to_string(),
            e
        );
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Wait for first segment
    pipeline.wait_for_ready().await.map_err(|e| {
        eprintln!(
            "[server] Timeout waiting for pipeline {}: {}",
            id.to_string(),
            e
        );
        StatusCode::GATEWAY_TIMEOUT
    })?;

    pipeline.record_activity();

    // Serve the playlist file
    let playlist_path = pipeline.output_dir().join("playlist.m3u8");
    serve_file(&playlist_path, "application/vnd.apple.mpegurl").await
}

/**
    Serve a segment file for a channel.
*/
async fn stream_segment(
    State(state): State<AppState>,
    Path((source_id, channel_id, filename)): Path<(String, String, String)>,
) -> Result<Response, StatusCode> {
    let id = ChannelId::new(&source_id, &channel_id);

    let pipeline = state
        .pipeline_store
        .get(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    pipeline.record_activity();

    let segment_path = pipeline.output_dir().join(&filename);
    serve_file(&segment_path, "video/mp2t").await
}

/**
    Get channel info (JSON).
*/
async fn channel_info(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    // Wait for source to be ready
    wait_for_source_ready(&state.registry, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);

    let entry = state.registry.get(&id).ok_or(StatusCode::NOT_FOUND)?;

    let stream_info = entry.stream_info.as_ref();

    let json = serde_json::json!({
        "id": id.to_string(),
        "source": source_id,
        "channel_id": channel_id,
        "name": entry.channel.name,
        "image": entry.channel.image,
        "manifest_url": stream_info.map(|s| &s.manifest_url),
        "license_url": stream_info.and_then(|s| s.license_url.as_ref()),
        "expires_at": stream_info.and_then(|s| s.expires_at),
        "error": entry.last_error,
    });

    Ok((
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json.to_string(),
    ))
}

/**
    Helper to serve a file
*/
async fn serve_file(path: &std::path::Path, content_type: &str) -> Result<Response, StatusCode> {
    let file = tokio::fs::File::open(path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            eprintln!("[server] Error opening file {:?}: {}", path, e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .unwrap())
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/**
    Convert ISO 8601 timestamp to XMLTV format (YYYYMMDDHHmmSS +0000).
*/
fn format_xmltv_time(iso_time: &str) -> String {
    // Try to parse ISO 8601 format (e.g., "2026-02-04T00:00:00.000Z")
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_time) {
        dt.format("%Y%m%d%H%M%S %z").to_string()
    } else {
        // Fallback: return as-is if parsing fails
        iso_time.to_string()
    }
}

/**
    Serve a channel's image, fetching and caching on first request.
*/
async fn channel_image(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    // Wait for source to be ready
    wait_for_source_ready(&state.registry, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);

    // Get channel entry to find the image URL
    let entry = state.registry.get(&id).ok_or(StatusCode::NOT_FOUND)?;

    let image_url = entry.channel.image.as_ref().ok_or(StatusCode::NOT_FOUND)?;

    // Get proxy setting from manifest
    let proxy = state
        .manifest_store
        .get(&source_id)
        .await
        .and_then(|m| m.source.proxy.clone());

    // Fetch from cache or download
    let cached = state
        .image_cache
        .get_or_fetch(&id, image_url, proxy.as_deref())
        .await
        .map_err(|e| {
            eprintln!(
                "[server] Failed to fetch image for {}: {}",
                id.to_string(),
                e
            );
            StatusCode::BAD_GATEWAY
        })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, cached.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from((*cached.data).clone()))
        .unwrap())
}

/**
    Serve a proxied image by its hash ID.
*/
async fn proxy_image(
    State(state): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<Response, StatusCode> {
    let cached = state.image_cache.get_by_id(&image_id).await.map_err(|e| {
        eprintln!("[server] Failed to fetch image {}: {}", image_id, e);
        StatusCode::NOT_FOUND
    })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, cached.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from((*cached.data).clone()))
        .unwrap())
}

/**
    Run the HTTP server.
*/
pub async fn run_server(
    addr: SocketAddr,
    registry: Arc<ChannelRegistry>,
    pipeline_store: Arc<PipelineStore>,
    manifest_store: Arc<ManifestStore>,
    image_cache: Arc<ImageCache>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState {
        registry,
        pipeline_store,
        manifest_store,
        image_cache,
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/i/{image_id}", get(proxy_image))
        .route("/{source_id}/info", get(source_info))
        .route("/{source_id}/channels.m3u", get(source_m3u))
        .route("/{source_id}/epg.xml", get(source_epg))
        .route("/{source_id}/{channel_id}/info", get(channel_info))
        .route("/{source_id}/{channel_id}/image", get(channel_image))
        .route(
            "/{source_id}/{channel_id}/playlist.m3u8",
            get(stream_playlist),
        )
        .route("/{source_id}/{channel_id}/{filename}", get(stream_segment))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown_rx.borrow_and_update() {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await?;

    Ok(())
}
