pub mod epg;
pub mod images;
pub mod m3u;
pub mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, routing::get};
use tokio::sync::watch;

use crate::channel::Resolver;
use crate::media::PipelineStore;

pub use images::ImageCache;

#[derive(Clone)]
pub struct AppState {
    pub resolver: Arc<Resolver>,
    pub pipeline_store: Arc<PipelineStore>,
    pub image_cache: Arc<ImageCache>,
}

/// Run the HTTP server.
pub async fn run_server(
    addr: SocketAddr,
    resolver: Arc<Resolver>,
    pipeline_store: Arc<PipelineStore>,
    image_cache: Arc<ImageCache>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState {
        resolver,
        pipeline_store,
        image_cache,
    };

    let app = Router::new()
        .route("/", get(routes::index))
        .route("/i/{image_id}", get(routes::proxy_image))
        .route("/{source_id}/info", get(routes::source_info))
        .route("/{source_id}/channels.m3u", get(routes::source_m3u))
        .route("/{source_id}/epg.xml", get(routes::source_epg))
        .route("/{source_id}/{channel_id}/info", get(routes::channel_info))
        .route(
            "/{source_id}/{channel_id}/image",
            get(routes::channel_image),
        )
        .route(
            "/{source_id}/{channel_id}/playlist.m3u8",
            get(routes::stream_playlist),
        )
        .route(
            "/{source_id}/{channel_id}/{filename}",
            get(routes::stream_segment),
        )
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
