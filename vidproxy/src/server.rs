use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use tokio::sync::watch;
use tower_http::services::ServeDir;

use crate::segments::SegmentManager;

/**
    Run the HTTP server that serves HLS content.
*/
pub async fn run_server(
    addr: SocketAddr,
    segment_manager: Arc<SegmentManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let serve_dir =
        ServeDir::new(segment_manager.output_dir()).append_index_html_on_directories(false);

    let app = Router::new().fallback_service(serve_dir);

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
