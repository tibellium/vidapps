/*!
    Probing functionality for extracting media metadata.
*/

use std::path::Path;
use std::time::Duration;

use ffmpeg_next::{ffi, format::context::Input as InputContext, media::Type};

use ffmpeg_types::{AudioStreamInfo, CodecId, Error, MediaInfo, Result, VideoStreamInfo};

use crate::convert::{
    channel_layout_from_count, codec_id_from_ffmpeg, pixel_format_from_ffmpeg,
    rational_from_ffmpeg, sample_format_from_ffmpeg,
};

/**
    Probe a media file to extract metadata without fully opening for playback.

    This is a lightweight operation that reads just enough of the file to
    determine stream information, duration, and codec details.

    # Example

    ```ignore
    let info = probe("video.mp4")?;
    if let Some(video) = &info.video {
        println!("Video: {}x{}", video.width, video.height);
    }
    ```
*/
pub fn probe<P: AsRef<Path>>(path: P) -> Result<MediaInfo> {
    ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

    let input_ctx = ffmpeg_next::format::input(&path).map_err(|e| {
        if e.to_string().contains("No such file") {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                e.to_string(),
            ))
        } else {
            Error::codec(e.to_string())
        }
    })?;

    extract_media_info(&input_ctx)
}

/**
    Extract MediaInfo from an already-opened input context.
*/
pub(crate) fn extract_media_info(input_ctx: &InputContext) -> Result<MediaInfo> {
    let video = extract_video_stream_info(input_ctx);
    let audio = extract_audio_stream_info(input_ctx);

    // Get overall duration from container if available
    let duration = if input_ctx.duration() > 0 {
        Some(Duration::from_micros(input_ctx.duration() as u64))
    } else {
        // Try to get from video or audio stream
        video
            .as_ref()
            .and_then(|v| v.duration)
            .or_else(|| audio.as_ref().and_then(|a| a.duration))
    };

    Ok(MediaInfo {
        duration,
        video,
        audio,
    })
}

/**
    Extract video stream info from input context.
*/
fn extract_video_stream_info(input_ctx: &InputContext) -> Option<VideoStreamInfo> {
    let stream = input_ctx.streams().best(Type::Video)?;

    let time_base = rational_from_ffmpeg(stream.time_base());

    // Get duration from stream or container
    let duration = if stream.duration() > 0 {
        let seconds = stream.duration() as f64 * time_base.num as f64 / time_base.den as f64;
        Some(Duration::from_secs_f64(seconds))
    } else if input_ctx.duration() > 0 {
        Some(Duration::from_micros(input_ctx.duration() as u64))
    } else {
        None
    };

    // Get codec parameters
    let codec_params = stream.parameters();

    // Create a decoder context to get dimensions and format
    let decoder_ctx = ffmpeg_next::codec::context::Context::from_parameters(codec_params).ok()?;
    let decoder = decoder_ctx.decoder().video().ok()?;

    let pixel_format = pixel_format_from_ffmpeg(decoder.format())?;
    let codec_id = codec_id_from_ffmpeg(stream.parameters().id()).unwrap_or(CodecId::H264); // Default to H264 if unknown

    // Get frame rate
    let frame_rate = if stream.avg_frame_rate().numerator() != 0 {
        Some(rational_from_ffmpeg(stream.avg_frame_rate()))
    } else if stream.rate().numerator() != 0 {
        Some(rational_from_ffmpeg(stream.rate()))
    } else {
        None
    };

    // Extract extradata, bitrate, profile, level from codec parameters
    // SAFETY: We're reading from a valid AVCodecParameters pointer that FFmpeg owns
    let (extradata, bitrate, profile, level) = unsafe {
        let ptr = stream.parameters().as_ptr();

        // Extract extradata (SPS/PPS for H.264, etc.)
        let extradata = if (*ptr).extradata_size > 0 && !(*ptr).extradata.is_null() {
            let slice =
                std::slice::from_raw_parts((*ptr).extradata, (*ptr).extradata_size as usize);
            Some(slice.to_vec())
        } else {
            None
        };

        // Extract bitrate
        let bitrate = if (*ptr).bit_rate > 0 {
            Some((*ptr).bit_rate as u64)
        } else {
            None
        };

        // Extract profile
        let profile = if (*ptr).profile != ffi::FF_PROFILE_UNKNOWN {
            Some((*ptr).profile)
        } else {
            None
        };

        // Extract level
        let level = if (*ptr).level != ffi::AV_LEVEL_UNKNOWN {
            Some((*ptr).level)
        } else {
            None
        };

        (extradata, bitrate, profile, level)
    };

    Some(VideoStreamInfo {
        width: decoder.width(),
        height: decoder.height(),
        pixel_format,
        frame_rate,
        time_base,
        duration,
        codec_id,
        extradata,
        bitrate,
        profile,
        level,
    })
}

/**
    Extract audio stream info from input context.
*/
fn extract_audio_stream_info(input_ctx: &InputContext) -> Option<AudioStreamInfo> {
    let stream = input_ctx.streams().best(Type::Audio)?;

    let time_base = rational_from_ffmpeg(stream.time_base());

    // Get duration from stream or container
    let duration = if stream.duration() > 0 {
        let seconds = stream.duration() as f64 * time_base.num as f64 / time_base.den as f64;
        Some(Duration::from_secs_f64(seconds))
    } else if input_ctx.duration() > 0 {
        Some(Duration::from_micros(input_ctx.duration() as u64))
    } else {
        None
    };

    // Get codec parameters
    let codec_params = stream.parameters();

    // Create a decoder context to get format info
    let decoder_ctx = ffmpeg_next::codec::context::Context::from_parameters(codec_params).ok()?;
    let decoder = decoder_ctx.decoder().audio().ok()?;

    let sample_format = sample_format_from_ffmpeg(decoder.format())?;
    let channels = channel_layout_from_count(decoder.channels());
    let codec_id = codec_id_from_ffmpeg(stream.parameters().id()).unwrap_or(CodecId::Aac); // Default to AAC if unknown

    // Extract extradata, bitrate, profile from codec parameters
    // SAFETY: We're reading from a valid AVCodecParameters pointer that FFmpeg owns
    let (extradata, bitrate, profile) = unsafe {
        let ptr = stream.parameters().as_ptr();

        // Extract extradata (AudioSpecificConfig for AAC, etc.)
        let extradata = if (*ptr).extradata_size > 0 && !(*ptr).extradata.is_null() {
            let slice =
                std::slice::from_raw_parts((*ptr).extradata, (*ptr).extradata_size as usize);
            Some(slice.to_vec())
        } else {
            None
        };

        // Extract bitrate
        let bitrate = if (*ptr).bit_rate > 0 {
            Some((*ptr).bit_rate as u64)
        } else {
            None
        };

        // Extract profile
        let profile = if (*ptr).profile != ffi::FF_PROFILE_UNKNOWN {
            Some((*ptr).profile)
        } else {
            None
        };

        (extradata, bitrate, profile)
    };

    Some(AudioStreamInfo {
        sample_rate: decoder.rate(),
        channels,
        sample_format,
        time_base,
        duration,
        codec_id,
        extradata,
        bitrate,
        profile,
    })
}
