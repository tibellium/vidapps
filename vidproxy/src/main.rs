use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::{signal, sync::watch};

mod coordinator;
mod credentials;
mod manifest;
mod proxy;
mod segments;
mod server;
mod sniffer;

#[derive(Parser, Debug)]
#[command(name = "vidproxy")]
#[command(about = "HLS remuxing proxy with automatic DRM key extraction")]
struct Args {
    /// Channel to stream (by name or filename)
    #[arg(short = 'C', long)]
    channel: Option<String>,

    /// List available channels and exit
    #[arg(long)]
    list_channels: bool,

    /// Override proxy from manifest
    #[arg(long)]
    proxy: Option<String>,

    /// Run Chrome in headless mode
    #[arg(long)]
    headless: bool,

    /// HTTP server port
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Number of segments to keep
    #[arg(short = 'n', long, default_value = "32")]
    segment_count: usize,

    /// Segment duration in seconds
    #[arg(short = 'd', long, default_value = "4")]
    segment_duration: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Handle --list-channels
    if args.list_channels {
        println!("Available channels:");
        for name in manifest::list_channels()? {
            println!("  - {}", name);
        }
        return Ok(());
    }

    // Load manifest
    let channel_name = args.channel.as_deref().unwrap_or("Canal RCN");
    let mut channel_manifest = manifest::find_by_name(channel_name)?;

    // Override proxy if specified
    if args.proxy.is_some() {
        channel_manifest.channel.proxy = args.proxy.clone();
    }

    let display_name = channel_manifest.channel.name.clone();

    // Create segment manager with temp directory
    let segment_manager = Arc::new(segments::SegmentManager::new(args.segment_count)?);
    let output_dir = segment_manager.output_dir().to_path_buf();

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start HTTP server
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let server_shutdown_rx = shutdown_rx.clone();
    let server_segment_manager = segment_manager.clone();
    let server_channel_name = display_name.clone();

    let server_handle = tokio::spawn(async move {
        server::run_server(
            addr,
            server_segment_manager,
            server_shutdown_rx,
            &server_channel_name,
        )
        .await
    });

    println!("Channel: {}", display_name);
    println!("Stream: http://localhost:{}/playlist.m3u8", args.port);
    println!("IPTV playlist: http://localhost:{}/channels.m3u", args.port);

    // Create channels for coordination
    let (credentials_tx, credentials_rx) = credentials::credentials_channel();
    let (refresh_tx, refresh_rx) = coordinator::refresh_channel();

    // Start sniffer task
    let sniffer_shutdown_rx = shutdown_rx.clone();
    let sniffer_manifest = channel_manifest.clone();
    let sniffer_headless = args.headless;

    let sniffer_handle = tokio::spawn(async move {
        let mut sniffer =
            sniffer::DrmSniffer::new(sniffer_manifest, sniffer_headless, credentials_tx);
        if let Err(e) = sniffer.run(sniffer_shutdown_rx, refresh_rx).await {
            eprintln!("[sniffer] Error: {}", e);
        }
    });

    // Start coordinator task
    let coord_shutdown_rx = shutdown_rx.clone();
    let segment_duration = Duration::from_secs(args.segment_duration);
    let coord_segment_manager = segment_manager.clone();
    let coord_output_dir = output_dir.clone();

    let coordinator_handle = tokio::spawn(async move {
        let mut coordinator = coordinator::Coordinator::new(
            credentials_rx,
            refresh_tx,
            coord_shutdown_rx,
            coord_segment_manager,
            coord_output_dir,
            segment_duration,
        );
        if let Err(e) = coordinator.run().await {
            eprintln!("[coordinator] Error: {}", e);
        }
    });

    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    println!("\nShutting down...");
    let _ = shutdown_tx.send(true);

    let _ = sniffer_handle.await;
    let _ = coordinator_handle.await;
    let _ = server_handle.await;

    println!("Done.");
    Ok(())
}
