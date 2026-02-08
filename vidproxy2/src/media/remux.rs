use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ffmpeg_sink::{Sink, SinkConfig};
use ffmpeg_source::{ContentKey, Source, SourceConfig};
use tokio::sync::watch;

use super::segments::SegmentManager;

/**
    Typed remux error for structured error handling.
*/
#[derive(Debug)]
pub enum RemuxError {
    /// Authentication / credential error — caller should refresh stream info.
    Auth(String),
    /// Network error — transient, may be retryable.
    Network(String),
    /// Format / codec error — likely not retryable.
    Format(String),
    /// Pipeline was shut down by request.
    Shutdown,
}

impl std::fmt::Display for RemuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemuxError::Auth(msg) => write!(f, "auth error: {}", msg),
            RemuxError::Network(msg) => write!(f, "network error: {}", msg),
            RemuxError::Format(msg) => write!(f, "format error: {}", msg),
            RemuxError::Shutdown => write!(f, "shutdown"),
        }
    }
}

impl std::error::Error for RemuxError {}

/**
    Classify an ffmpeg error into a typed RemuxError.
*/
fn classify_error(error: ffmpeg_types::Error) -> RemuxError {
    let msg = error.to_string();
    let lower = msg.to_lowercase();

    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("410")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("expired")
        || lower.contains("invalid token")
        || lower.contains("access denied")
    {
        RemuxError::Auth(msg)
    } else if lower.contains("connection")
        || lower.contains("timeout")
        || lower.contains("network")
        || lower.contains("dns")
    {
        RemuxError::Network(msg)
    } else {
        RemuxError::Format(msg)
    }
}

/**
    Run the remux pipeline: read from source HLS/DASH, write to local HLS.
*/
pub async fn run_remux_pipeline(
    input_url: &str,
    headers: &[(String, String)],
    decryption_keys: &[String],
    output_dir: &Path,
    segment_duration: Duration,
    segment_manager: Arc<SegmentManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), RemuxError> {
    let mut source_config = SourceConfig::default();
    if !decryption_keys.is_empty() {
        let keys: Vec<ContentKey> = decryption_keys
            .iter()
            .filter_map(|key| match key.parse::<ContentKey>() {
                Ok(k) => Some(k),
                Err(e) => {
                    eprintln!("Warning: invalid decryption key '{key}': {e}");
                    None
                }
            })
            .collect();

        if !keys.is_empty() {
            println!("Using {} CENC decryption key(s)", keys.len());
            source_config = source_config.with_decryption_keys(keys);
        }
    }

    if !headers.is_empty() {
        source_config = source_config.with_headers(headers.to_vec());
    }

    let mut source = Source::open(input_url, source_config)
        .await
        .map_err(classify_error)?;

    let media_info = source.media_info();
    println!(
        "Source: {}x{}, {:?}",
        media_info.video.as_ref().map(|v| v.width).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.height).unwrap_or(0),
        media_info.video.as_ref().map(|v| v.codec_id),
    );

    let playlist_path = output_dir.join("playlist.m3u8");
    let mut sink_config = SinkConfig::hls(segment_duration).rebase_timestamps();

    if let Some(video_info) = media_info.video.clone() {
        sink_config = sink_config.with_video(video_info);
    }
    if let Some(audio_info) = media_info.audio.clone() {
        sink_config = sink_config.with_audio(audio_info);
    }

    let mut sink = Sink::file(&playlist_path, sink_config).map_err(classify_error)?;

    println!("Writing HLS to: {}", output_dir.display());

    let mut packet_count = 0u64;
    let mut last_scan = std::time::Instant::now();

    loop {
        if *shutdown_rx.borrow() {
            return Err(RemuxError::Shutdown);
        }

        if shutdown_rx.has_changed().unwrap_or(false) {
            let _ = shutdown_rx.borrow_and_update();
            if *shutdown_rx.borrow() {
                return Err(RemuxError::Shutdown);
            }
        }

        let packet = match source.next_packet().map_err(classify_error)? {
            Some(p) => p,
            None => {
                println!("Source ended");
                break;
            }
        };

        sink.write(&packet).map_err(classify_error)?;
        packet_count += 1;

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

    sink.finish().map_err(classify_error)?;
    println!("Remux pipeline stopped after {} packets", packet_count);

    Ok(())
}
