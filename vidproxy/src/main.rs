use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::{signal, sync::watch};

mod proxy;
mod segments;
mod server;

#[derive(Parser, Debug)]
#[command(name = "vidproxy")]
#[command(about = "HLS remuxing proxy - strips authentication from HLS streams")]
struct Args {
    /// Source HLS URL
    #[arg(short, long)]
    input: String,

    /// HTTP server port
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Custom HTTP headers (format: "Name: Value")
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,

    /// Number of segments to keep
    #[arg(short = 'n', long, default_value = "32")]
    segment_count: usize,

    /// Segment duration in seconds
    #[arg(short = 'd', long, default_value = "4")]
    segment_duration: u64,

    /// CENC decryption key (hex string, 32 chars for AES-128)
    #[arg(short = 'k', long)]
    decryption_key: Option<String>,
}

fn parse_headers(headers: &[String]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|h| {
            let mut parts = h.splitn(2, ':');
            let key = parts.next()?.trim().to_string();
            let value = parts.next()?.trim().to_string();
            Some((key, value))
        })
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Parse custom headers
    let headers = parse_headers(&args.headers);

    // Create segment manager with temp directory
    let segment_manager = Arc::new(segments::SegmentManager::new(args.segment_count)?);

    let output_dir = segment_manager.output_dir().to_path_buf();

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start the remux pipeline in a blocking task
    let proxy_segment_manager = segment_manager.clone();
    let proxy_shutdown_rx = shutdown_rx.clone();
    let input_url = args.input.clone();
    let segment_duration = Duration::from_secs(args.segment_duration);
    let decryption_key = args.decryption_key.clone();

    let proxy_handle = tokio::task::spawn_blocking(move || {
        proxy::run_remux_pipeline(
            &input_url,
            &headers,
            decryption_key.as_deref(),
            &output_dir,
            segment_duration,
            proxy_segment_manager,
            proxy_shutdown_rx,
        )
    });

    // Start HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let server_shutdown_rx = shutdown_rx.clone();

    let server_handle =
        tokio::spawn(
            async move { server::run_server(addr, segment_manager, server_shutdown_rx).await },
        );

    println!(
        "Proxying {} -> http://localhost:{}/playlist.m3u8",
        args.input, args.port
    );

    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    println!("\nShutting down...");

    // Signal shutdown
    let _ = shutdown_tx.send(true);

    // Wait for tasks to finish
    let _ = proxy_handle.await;
    let _ = server_handle.await;

    println!("Done.");
    Ok(())
}
