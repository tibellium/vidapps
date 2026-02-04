use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::get,
};
use chrono::{Duration, Utc};
use tokio::sync::watch;
use tower_http::services::ServeDir;

use crate::segments::SegmentManager;
use crate::stream_info::StreamInfoReceiver;

#[derive(Clone)]
struct AppState {
    fallback_channel_name: String,
    stream_info_rx: StreamInfoReceiver,
}

async fn channels_m3u(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:8080");

    let stream_info = state.stream_info_rx.borrow();
    let channel_name = stream_info
        .as_ref()
        .map(|info| info.channel_name.as_str())
        .unwrap_or(&state.fallback_channel_name);
    let thumbnail_url = stream_info
        .as_ref()
        .and_then(|info| info.thumbnail_url.as_deref());

    let logo_attr = thumbnail_url
        .map(|url| format!(" tvg-logo=\"{}\"", url))
        .unwrap_or_default();

    // url-tvg points to EPG data
    // tvg-id uses channel name as identifier for EPG matching
    // tvg-type="live" indicates 24/7 live stream (not VOD)
    // group-title categorizes the channel
    let playlist = format!(
        "#EXTM3U url-tvg=\"http://{host}/epg.xml\"\n\
         #EXTINF:-1 tvg-id=\"{name}\" tvg-name=\"{name}\" tvg-type=\"live\" group-title=\"Live TV\"{logo},{name}\n\
         http://{host}/playlist.m3u8\n",
        name = channel_name,
        logo = logo_attr,
        host = host,
    );

    ([(header::CONTENT_TYPE, "audio/x-mpegurl")], playlist)
}

/// Generate XMLTV EPG data for 24/7 live channels
async fn epg_xml(State(state): State<AppState>) -> impl IntoResponse {
    let stream_info = state.stream_info_rx.borrow();
    let channel_name = stream_info
        .as_ref()
        .map(|info| info.channel_name.as_str())
        .unwrap_or(&state.fallback_channel_name);
    let thumbnail_url = stream_info
        .as_ref()
        .and_then(|info| info.thumbnail_url.as_deref());

    let icon_element = thumbnail_url
        .map(|url| format!("    <icon src=\"{}\"/>\n", escape_xml(url)))
        .unwrap_or_default();

    // Generate program entries for the next 7 days (one entry per day)
    let now = Utc::now();
    let start_of_day = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let start = start_of_day.and_utc();

    let mut programmes = String::new();
    for day in 0..7 {
        let day_start = start + Duration::days(day);
        let day_end = day_start + Duration::days(1);

        let start_str = day_start.format("%Y%m%d%H%M%S %z");
        let end_str = day_end.format("%Y%m%d%H%M%S %z");

        programmes.push_str(&format!(
            "  <programme start=\"{}\" stop=\"{}\" channel=\"{}\">\n\
             \x20   <title lang=\"es\">{}</title>\n\
             \x20   <desc lang=\"es\">Transmisión en vivo 24/7</desc>\n\
             \x20 </programme>\n",
            start_str,
            end_str,
            escape_xml(channel_name),
            escape_xml(channel_name),
        ));
    }

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE tv SYSTEM \"xmltv.dtd\">\n\
         <tv generator-info-name=\"vidproxy\">\n\
         \x20 <channel id=\"{id}\">\n\
         \x20   <display-name lang=\"es\">{name}</display-name>\n\
         {icon}\
         \x20 </channel>\n\
         {programmes}\
         </tv>\n",
        id = escape_xml(channel_name),
        name = escape_xml(channel_name),
        icon = icon_element,
        programmes = programmes,
    );

    ([(header::CONTENT_TYPE, "application/xml")], xml)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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
        fallback_channel_name: channel_name.to_string(),
        stream_info_rx,
    };

    let app = Router::new()
        .route("/channels.m3u", get(channels_m3u))
        .route("/epg.xml", get(epg_xml))
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
