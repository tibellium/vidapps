use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::get,
};
use tokio::sync::watch;
use tower_http::services::ServeDir;

use crate::segments::SegmentManager;
use crate::stream_info::StreamInfoReceiver;

#[derive(Clone)]
struct AppState {
    channel_name: String,
    stream_info_rx: StreamInfoReceiver,
}

async fn channels_m3u(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");

    let thumbnail_url = state
        .stream_info_rx
        .borrow()
        .as_ref()
        .and_then(|info| info.thumbnail_url.clone());

    let logo_attr = thumbnail_url
        .map(|url| format!(" tvg-logo=\"{}\"", url))
        .unwrap_or_default();

    let playlist = format!(
        "#EXTM3U\n#EXTINF:-1 tvg-name=\"{name}\"{logo},{name}\nhttp://{host}/playlist.m3u8\n",
        name = state.channel_name,
        logo = logo_attr,
        host = host,
    );

    ([(header::CONTENT_TYPE, "audio/x-mpegurl")], playlist)
}

/**
    Run the HTTP server that serves HLS content.
*/
pub async fn run_server(
    addr: SocketAddr,
    segment_manager: Arc<SegmentManager>,
    mut shutdown_rx: watch::Receiver<bool>,
    channel_name: &str,
    stream_info_rx: StreamInfoReceiver,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let serve_dir =
        ServeDir::new(segment_manager.output_dir()).append_index_html_on_directories(false);

    let state = AppState {
        channel_name: channel_name.to_string(),
        stream_info_rx,
    };

    let app = Router::new()
        .route("/channels.m3u", get(channels_m3u))
        .with_state(state)
        .fallback_service(serve_dir);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("HTTP server listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // Wait for shutdown signal
            while !*shutdown_rx.borrow_and_update() {
                if shutdown_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await?;

    Ok(())
}
