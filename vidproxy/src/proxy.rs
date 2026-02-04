use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ffmpeg_sink::{Sink, SinkConfig};
use ffmpeg_source::{DecryptionKey, Source, SourceConfig};
use tokio::sync::watch;

use crate::segments::SegmentManager;

/**
    Run the remux pipeline: read from source HLS/DASH, write to local HLS.

    Note: Custom headers are no longer supported by the ffmpeg-source API.

    The `headers` parameter is kept for API compatibility but is currently ignored.
*/
pub async fn run_remux_pipeline(
    input_url: &str,
    #[allow(unused_variables)] headers: &[(String, String)],
    decryption_keys: &[String],
    output_dir: &Path,
    segment_duration: Duration,
    segment_manager: Arc<SegmentManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), ffmpeg_types::Error> {
    // Build source config with decryption keys if provided
    let mut source_config = SourceConfig::default();
    if !decryption_keys.is_empty() {
        let keys: Vec<DecryptionKey> = decryption_keys
            .iter()
            .filter_map(|key| {
                if let Some((key_id, key_value)) = key.split_once(':') {
                    Some(DecryptionKey {
                        key_id: key_id.to_string(),
                        key: key_value.to_string(),
                    })
                } else {
                    eprintln!("Warning: decryption key must be in 'key_id:key' format, ignoring");
                    None
                }
            })
            .collect();

        if !keys.is_empty() {
            println!("Using {} CENC decryption key(s)", keys.len());
            source_config = source_config.with_decryption_keys(keys);
        }
    }

    // Open source (now async)
    let mut source = Source::open(input_url, source_config).await?;

    let media_info = source.media_info();
    println!(
        "Source: {}x{}, {:?}",
        media_info.video.as_ref().map(|v| v.width).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.height).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.codec_id),
    );
    if let Some(ref video) = media_info.video {
        println!(
            "Video time_base: {}/{}",
            video.time_base.num, video.time_base.den
        );
    }
    if let Some(ref audio) = media_info.audio {
        println!(
            "Audio time_base: {}/{}",
            audio.time_base.num, audio.time_base.den
        );
    }

    // Configure HLS sink
    let playlist_path = output_dir.join("playlist.m3u8");
    let mut sink_config = SinkConfig::hls(segment_duration).rebase_timestamps();

    if let Some(video_info) = media_info.video.clone() {
        sink_config = sink_config.with_video(video_info);
    }
    if let Some(audio_info) = media_info.audio.clone() {
        sink_config = sink_config.with_audio(audio_info);
    }

    let mut sink = Sink::file(&playlist_path, sink_config)?;
    println!("Sink created: {:?}", sink);

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
