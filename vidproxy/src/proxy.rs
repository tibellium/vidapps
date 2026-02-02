use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use ffmpeg_sink::{Sink, SinkConfig};
use ffmpeg_source::{NetworkOptions, Source, SourceConfig};

use crate::segments::SegmentManager;

/**
    Run the remux pipeline: read from source HLS/DASH, write to local HLS.
*/
pub fn run_remux_pipeline(
    input_url: &str,
    headers: &[(String, String)],
    decryption_key: Option<&str>,
    output_dir: &Path,
    segment_duration: Duration,
    segment_manager: Arc<SegmentManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ffmpeg_types::Error> {
    // Build network options with custom headers
    let mut network_opts = NetworkOptions::default();
    for (key, value) in headers {
        network_opts = network_opts.header(key, value);
    }
    network_opts = network_opts.reconnect();

    // Add CENC decryption key if provided
    if let Some(key) = decryption_key {
        network_opts = network_opts.cenc_decryption_key(key);
        println!("Using CENC decryption key: {}...", &key[..8.min(key.len())]);
    }

    // Open source
    let source_config = SourceConfig::default().with_network_options(network_opts);
    let mut source = Source::open(input_url, source_config)?;

    let media_info = source.media_info();
    println!(
        "Source: {}x{}, {:?}",
        media_info.video.as_ref().map(|v| v.width).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.height).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.codec_id),
    );

    // Configure HLS sink
    let playlist_path = output_dir.join("playlist.m3u8");
    let mut sink_config = SinkConfig::hls(segment_duration);

    if let Some(video_info) = media_info.video.clone() {
        sink_config = sink_config.with_video(video_info);
    }
    if let Some(audio_info) = media_info.audio.clone() {
        sink_config = sink_config.with_audio(audio_info);
    }

    let mut sink = Sink::file(&playlist_path, sink_config)?;

    println!("Writing HLS to: {}", output_dir.display());

    let mut packet_count = 0u64;
    let mut last_scan = std::time::Instant::now();

    // Remux loop
    loop {
        // Check for shutdown
        if *shutdown_rx.borrow() {
            break;
        }

        // Check for shutdown without blocking
        if shutdown_rx.has_changed().unwrap_or(false) {
            let _ = shutdown_rx.borrow_and_update();
            if *shutdown_rx.borrow() {
                break;
            }
        }

        // Read next packet
        let packet = match source.next_packet()? {
            Some(p) => p,
            None => {
                // End of stream (shouldn't happen for live)
                println!("Source ended");
                break;
            }
        };

        // Write to sink
        sink.write(&packet)?;
        packet_count += 1;

        // Periodically scan for new segments and log progress
        if last_scan.elapsed() > Duration::from_secs(2) {
            segment_manager.scan_for_new_segments();
            println!(
                "Packets: {}, Segments: {}",
                packet_count,
                segment_manager.segment_count()
            );
            last_scan = std::time::Instant::now();
        }
    }

    // Finalize
    sink.finish()?;
    println!("Remux pipeline stopped after {} packets", packet_count);

    Ok(())
}
