use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::VideoInfo;

/**
    Error type for video probing operations.
*/
#[derive(Debug)]
pub enum ProbeError {
    /// Failed to execute ffprobe
    ExecutionFailed(std::io::Error),
    /// ffprobe returned non-zero exit code
    NonZeroExit(i32),
    /// Failed to parse ffprobe JSON output
    ParseFailed(serde_json::Error),
    /// No video stream found in the file
    NoVideoStream,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::ExecutionFailed(e) => write!(f, "Failed to execute ffprobe: {}", e),
            ProbeError::NonZeroExit(code) => write!(f, "ffprobe exited with code {}", code),
            ProbeError::ParseFailed(e) => write!(f, "Failed to parse ffprobe output: {}", e),
            ProbeError::NoVideoStream => write!(f, "No video stream found"),
        }
    }
}

impl std::error::Error for ProbeError {}

/**
    JSON structure for ffprobe stream output
*/
#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: String,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
}

/**
    Probe a video file using ffprobe to get its metadata.

    This function runs ffprobe as a subprocess and parses the JSON output
    to extract video dimensions and duration.

    Returns an error if:
    - ffprobe is not installed or fails to execute
    - The file is not a valid video (e.g., a .ts TypeScript file)
    - No video stream is found in the file
*/
pub fn probe_video(path: &Path) -> Result<VideoInfo, ProbeError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-select_streams",
            "v:0", // Select only the first video stream
        ])
        .arg(path)
        .output()
        .map_err(ProbeError::ExecutionFailed)?;

    if !output.status.success() {
        return Err(ProbeError::NonZeroExit(output.status.code().unwrap_or(-1)));
    }

    let probe_output: FfprobeOutput =
        serde_json::from_slice(&output.stdout).map_err(ProbeError::ParseFailed)?;

    // Find the video stream
    let video_stream = probe_output
        .streams
        .into_iter()
        .find(|s| s.codec_type == "video")
        .ok_or(ProbeError::NoVideoStream)?;

    let width = video_stream.width.ok_or(ProbeError::NoVideoStream)?;
    let height = video_stream.height.ok_or(ProbeError::NoVideoStream)?;

    // Parse duration if available
    let duration = video_stream.duration.and_then(|d: String| {
        d.parse::<f64>()
            .ok()
            .map(|secs| Duration::from_secs_f64(secs))
    });

    Ok(VideoInfo::new(path.to_path_buf(), width, height, duration))
}
