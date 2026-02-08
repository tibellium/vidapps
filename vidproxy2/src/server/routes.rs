use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use tokio_util::io::ReaderStream;

use crate::channel::{ChannelId, SourceState};

use super::AppState;

const SOURCE_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Wait for a source to finish loading, returning an error status if it fails.
async fn wait_for_source_ready(state: &AppState, source_id: &str) -> Result<(), StatusCode> {
    match state
        .resolver
        .registry
        .wait_for_source(source_id, SOURCE_WAIT_TIMEOUT)
        .await
    {
        Some(SourceState::Ready) => Ok(()),
        Some(SourceState::Failed(err)) => {
            eprintln!("[server] Source '{}' failed: {}", source_id, err);
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
        Some(SourceState::Loading) => {
            eprintln!(
                "[server] Timeout waiting for source '{}' to load",
                source_id
            );
            Err(StatusCode::GATEWAY_TIMEOUT)
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

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

/// Root endpoint — list all sources.
pub async fn index(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let base_url = get_base_url(&headers);
    let manifests = state.resolver.manifest_store.list().await;

    let sources: Vec<serde_json::Value> = manifests
        .iter()
        .map(|m| {
            let status = match state.resolver.registry.get_source_state(&m.source.id) {
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

    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({ "sources": sources }).to_string(),
    )
}

/// Source info endpoint.
pub async fn source_info(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    let manifest = state
        .resolver
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let base_url = get_base_url(&headers);
    let source_state = state.resolver.registry.get_source_state(&source_id);
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

    let channels = state.resolver.registry.list_by_source(&source_id);
    let channel_list: Vec<serde_json::Value> = channels
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.channel.id,
                "name": e.channel.name,
                "info": format!("{}/{}/{}/info", base_url, source_id, e.channel.id),
                "image": e.channel.image.as_ref().map(|_| format!("{}/{}/{}/image", base_url, source_id, e.channel.id)),
                "playlist": format!("{}/{}/{}/playlist.m3u8", base_url, source_id, e.channel.id),
                "resolved": e.stream_info.is_some(),
            })
        })
        .collect();

    Ok((
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "id": manifest.source.id,
            "name": manifest.source.name,
            "status": status,
            "error": error,
            "m3u": format!("{}/{}/channels.m3u", base_url, source_id),
            "epg": format!("{}/{}/epg.xml", base_url, source_id),
            "channels": channel_list,
        })
        .to_string(),
    ))
}

/// M3U playlist endpoint.
pub async fn source_m3u(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    wait_for_source_ready(&state, &source_id).await?;

    let manifest = state
        .resolver
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let channels = state.resolver.registry.list_by_source(&source_id);
    if channels.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let base_url = get_base_url(&headers);
    let playlist = super::m3u::generate_m3u(&channels, &manifest.source, &base_url);

    Ok(([(header::CONTENT_TYPE, "audio/x-mpegurl")], playlist))
}

/// EPG XML endpoint.
pub async fn source_epg(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    wait_for_source_ready(&state, &source_id).await?;

    // Refresh metadata if expired (EPG data goes stale without this)
    let _ = state.resolver.refresh_metadata_if_needed(&source_id).await;

    let manifest = state
        .resolver
        .manifest_store
        .get(&source_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let channels = state.resolver.registry.list_by_source(&source_id);
    if channels.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let base_url = get_base_url(&headers);
    let xml =
        super::epg::generate_epg(&channels, &manifest.source, &base_url, &state.image_cache).await;

    Ok(([(header::CONTENT_TYPE, "application/xml")], xml))
}

/// Channel info endpoint.
pub async fn channel_info(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, StatusCode> {
    wait_for_source_ready(&state, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);
    let entry = state
        .resolver
        .registry
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let stream_info = entry.stream_info.as_ref();

    Ok((
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "id": id.to_string(),
            "source": source_id,
            "channel_id": channel_id,
            "name": entry.channel.name,
            "image": entry.channel.image,
            "manifest_url": stream_info.map(|s| &s.manifest_url),
            "license_url": stream_info.and_then(|s| s.license_url.as_ref()),
            "expires_at": stream_info.and_then(|s| s.expires_at),
            "error": entry.last_error,
        })
        .to_string(),
    ))
}

/// Channel image endpoint.
pub async fn channel_image(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    wait_for_source_ready(&state, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);
    let entry = state
        .resolver
        .registry
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let image_url = entry.channel.image.as_ref().ok_or(StatusCode::NOT_FOUND)?;

    let proxy = state
        .resolver
        .manifest_store
        .get(&source_id)
        .await
        .and_then(|m| m.source.proxy.clone());

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
        .header(header::CONTENT_TYPE, &cached.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from((*cached.data).clone()))
        .unwrap())
}

/// Proxied image endpoint (for EPG programme icons).
pub async fn proxy_image(
    State(state): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<Response, StatusCode> {
    let cached = state.image_cache.get_by_id(&image_id).await.map_err(|e| {
        eprintln!("[server] Failed to fetch image {}: {}", image_id, e);
        StatusCode::NOT_FOUND
    })?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &cached.content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from((*cached.data).clone()))
        .unwrap())
}

/// HLS playlist endpoint — resolves content on-demand and starts pipeline.
pub async fn stream_playlist(
    State(state): State<AppState>,
    Path((source_id, channel_id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    wait_for_source_ready(&state, &source_id).await?;

    let id = ChannelId::new(&source_id, &channel_id);

    // Refresh discovery if expired
    let _ = state.resolver.refresh_discovery_if_needed(&source_id).await;

    // Check channel exists
    let _entry = state
        .resolver
        .registry
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?;

    // Check if pipeline needs refresh due to auth error
    let pipeline_needs_refresh = if let Some(pipeline) = state.pipeline_store.get(&id).await {
        pipeline.needs_refresh()
    } else {
        false
    };

    // If pipeline needs refresh, reset content state so resolver will re-resolve
    if pipeline_needs_refresh {
        println!(
            "[server] Pipeline auth error for {}, refreshing...",
            id.to_string()
        );
        state.resolver.registry.reset_channel_content_state(&id);
    }

    // Resolve stream info (on-demand with concurrent coalescing)
    let stream_info = state.resolver.ensure_stream_info(&id).await.map_err(|e| {
        eprintln!(
            "[server] Content resolution failed for {}: {}",
            id.to_string(),
            e
        );
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    // Update pipeline stream info if it was refreshed
    if pipeline_needs_refresh && let Some(pipeline) = state.pipeline_store.get(&id).await {
        pipeline.update_stream_info(stream_info.clone()).await;
        pipeline.stop().await;
    }

    // Get or create pipeline
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

    pipeline.ensure_running().await.map_err(|e| {
        eprintln!(
            "[server] Failed to start pipeline for {}: {}",
            id.to_string(),
            e
        );
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    pipeline.wait_for_ready().await.map_err(|e| {
        eprintln!(
            "[server] Timeout waiting for pipeline {}: {}",
            id.to_string(),
            e
        );
        StatusCode::GATEWAY_TIMEOUT
    })?;

    pipeline.record_activity();

    let playlist_path = pipeline.output_dir().join("playlist.m3u8");
    serve_file(&playlist_path, "application/vnd.apple.mpegurl").await
}

/// HLS segment endpoint.
pub async fn stream_segment(
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
