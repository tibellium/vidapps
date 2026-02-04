use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::{signal, sync::watch};

mod cdrm;
mod image_cache;
mod manifest;
mod pipeline;
mod proxy;
mod registry;
mod segments;
mod server;
mod source;

use image_cache::ImageCache;
use pipeline::{PipelineConfig, PipelineStore};
use registry::ChannelRegistry;
use server::ManifestStore;

#[derive(Parser, Debug)]
#[command(name = "vidproxy")]
#[command(about = "Multi-channel HLS proxy with automatic DRM key extraction")]
struct Args {
    /// List available sources and exit
    #[arg(long)]
    list_sources: bool,

    /// Run Chrome in headless mode
    #[arg(long)]
    headless: bool,

    /// HTTP server port
    #[arg(short, long, default_value = "8098")]
    port: u16,

    /// Number of segments to keep per channel
    #[arg(short = 'n', long, default_value = "32")]
    segment_count: usize,

    /// Segment duration in seconds
    #[arg(short = 'd', long, default_value = "4")]
    segment_duration: u64,

    /// Idle timeout in seconds (stop pipeline after no activity)
    #[arg(long, default_value = "30")]
    idle_timeout: u64,

    /// Startup timeout in seconds (max wait for first segment)
    #[arg(long, default_value = "30")]
    startup_timeout: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Handle --list-sources
    if args.list_sources {
        println!("Available sources:");
        for name in manifest::list_sources()? {
            println!("  - {}", name);
        }
        return Ok(());
    }

    // Create shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Create channel registry
    let registry = Arc::new(ChannelRegistry::new());

    // Create temp directory for segments
    let temp_dir = tempfile::tempdir()?;
    let base_output_dir = temp_dir.path().to_path_buf();

    // Create pipeline store
    let pipeline_config = PipelineConfig {
        segment_count: args.segment_count,
        segment_duration: Duration::from_secs(args.segment_duration),
        idle_timeout: Duration::from_secs(args.idle_timeout),
        startup_timeout: Duration::from_secs(args.startup_timeout),
        base_output_dir,
    };
    let pipeline_store = Arc::new(PipelineStore::new(pipeline_config, shutdown_rx.clone()));

    // Create manifest store for refresh operations
    let manifest_store = Arc::new(ManifestStore::new(args.headless));

    // Create image cache for on-demand image fetching
    let image_cache = Arc::new(ImageCache::new());

    // Load source manifests
    println!("Loading sources...");
    let manifests = manifest::load_all()?;

    if manifests.is_empty() {
        eprintln!("No source manifests found in channels/");
        return Ok(());
    }

    // Mark all sources as loading and store manifests
    for manifest in &manifests {
        println!("Source: {} ({})", manifest.source.name, manifest.source.id);
        registry.mark_source_loading(&manifest.source.id);
        manifest_store.add(manifest.clone()).await;
    }

    // Start HTTP server IMMEDIATELY (before discovery)
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));

    println!();
    println!("HTTP server listening on http://localhost:{}", args.port);
    println!("  Requests will wait for source discovery to complete");
    println!();

    let server_registry = Arc::clone(&registry);
    let server_pipeline_store = Arc::clone(&pipeline_store);
    let server_manifest_store = Arc::clone(&manifest_store);
    let server_image_cache = Arc::clone(&image_cache);
    let server_shutdown_rx = shutdown_rx.clone();

    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::run_server(
            addr,
            server_registry,
            server_pipeline_store,
            server_manifest_store,
            server_image_cache,
            server_shutdown_rx,
        )
        .await
        {
            eprintln!("[server] Error: {}", e);
        }
    });

    // Spawn discovery tasks in background
    let headless = args.headless;
    for manifest in manifests {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            println!(
                "[discovery] Starting source: {} ({})",
                manifest.source.name, manifest.source.id
            );

            match source::run_source(&manifest, headless).await {
                Ok(result) => {
                    let channel_count = result.channels.len();
                    registry.register_source(
                        &result.source_id,
                        result.channels,
                        result.discovery_expires_at,
                    );
                    println!(
                        "[discovery] Source '{}' ready: {} channels",
                        manifest.source.id, channel_count
                    );
                }
                Err(e) => {
                    eprintln!("[discovery] Source '{}' failed: {}", manifest.source.id, e);
                    registry.mark_source_failed(&manifest.source.id, e.to_string());
                }
            }
        });
    }

    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    println!("\nShutting down...");
    let _ = shutdown_tx.send(true);

    // Stop all pipelines
    pipeline_store.stop_all().await;

    let _ = server_handle.await;

    // Keep temp_dir alive until here
    drop(temp_dir);

    println!("Done.");
    Ok(())
}
