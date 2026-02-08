use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::{signal, sync::watch};

mod channel;
mod engine;
mod media;
mod server;
mod util;

use channel::{ChannelRegistry, ManifestStore, Resolver};
use media::{PipelineConfig, PipelineStore};
use server::ImageCache;

#[derive(Parser, Debug)]
#[command(name = "vidproxy")]
#[command(about = "Multi-channel HLS proxy with automatic DRM key extraction")]
struct Args {
    /// List available sources and exit
    #[arg(long)]
    list_sources: bool,

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
        for name in engine::list_sources()? {
            println!("  - {}", name);
        }
        return Ok(());
    }

    // Shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Core state
    let registry = Arc::new(ChannelRegistry::new());
    let manifest_store = Arc::new(ManifestStore::new());
    let resolver = Arc::new(Resolver::new(
        Arc::clone(&registry),
        Arc::clone(&manifest_store),
    ));

    // Temp directory for HLS segments
    let temp_dir = tempfile::tempdir()?;

    // Pipeline store
    let pipeline_config = PipelineConfig {
        segment_count: args.segment_count,
        segment_duration: Duration::from_secs(args.segment_duration),
        idle_timeout: Duration::from_secs(args.idle_timeout),
        startup_timeout: Duration::from_secs(args.startup_timeout),
        base_output_dir: temp_dir.path().to_path_buf(),
    };
    let pipeline_store = Arc::new(PipelineStore::new(pipeline_config, shutdown_rx.clone()));

    // Image cache
    let image_cache = Arc::new(ImageCache::new());

    // Load manifests
    println!("Loading sources...");
    let manifests = engine::load_all()?;

    if manifests.is_empty() {
        eprintln!("No source manifests found in channels/");
        return Ok(());
    }

    for manifest in &manifests {
        println!("Source: {} ({})", manifest.source.name, manifest.source.id);
        registry.mark_source_loading(&manifest.source.id);
        manifest_store.add(manifest.clone()).await;
    }

    // Start HTTP server immediately (before discovery)
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));

    println!();
    println!("HTTP server listening on http://localhost:{}", args.port);
    println!("  Requests will wait for source discovery to complete");
    println!();

    let server_handle = {
        let resolver = Arc::clone(&resolver);
        let pipeline_store = Arc::clone(&pipeline_store);
        let image_cache = Arc::clone(&image_cache);
        let shutdown_rx = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) =
                server::run_server(addr, resolver, pipeline_store, image_cache, shutdown_rx).await
            {
                eprintln!("[server] Error: {}", e);
            }
        })
    };

    // Run discovery tasks in parallel
    {
        let resolver = Arc::clone(&resolver);
        tokio::spawn(async move {
            let mut handles = Vec::new();

            for manifest in manifests {
                let resolver = Arc::clone(&resolver);
                handles.push(tokio::spawn(async move {
                    println!(
                        "[discovery] Starting source: {} ({})",
                        manifest.source.name, manifest.source.id
                    );

                    match resolver.run_initial_discovery(&manifest).await {
                        Ok(()) => {
                            let count = resolver.registry.list_by_source(&manifest.source.id).len();
                            println!(
                                "[discovery] Source '{}' ready: {} channels (content on-demand)",
                                manifest.source.id, count
                            );
                        }
                        Err(e) => {
                            eprintln!("[discovery] Source '{}' failed: {}", manifest.source.id, e);
                        }
                    }
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    }

    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    println!("\nShutting down...");
    let _ = shutdown_tx.send(true);

    pipeline_store.stop_all().await;
    let _ = server_handle.await;

    drop(temp_dir);

    println!("Done.");
    Ok(())
}
