use std::path::Path;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ffmpeg_next::ffi;
use ffmpeg_next::format::input;
use ffmpeg_next::media::Type;
use ffmpeg_next::software::scaling::{
    context::Context as ScalerContext, flag::Flags as ScalerFlags,
};
use ffmpeg_next::util::frame::video::Video as VideoFrameFFmpeg;
use ffmpeg_next::{Rational, codec};

use super::frame::VideoFrame;
use super::queue::FrameQueue;

/// Error type for video decoding operations
#[derive(Debug)]
pub enum DecoderError {
    NoVideoStream,
    Ffmpeg(ffmpeg_next::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderError::Ffmpeg(e) => write!(f, "FFmpeg error: {}", e),
            DecoderError::NoVideoStream => write!(f, "No video stream found"),
            DecoderError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for DecoderError {}

impl From<ffmpeg_next::Error> for DecoderError {
    fn from(e: ffmpeg_next::Error) -> Self {
        DecoderError::Ffmpeg(e)
    }
}

impl From<std::io::Error> for DecoderError {
    fn from(e: std::io::Error) -> Self {
        DecoderError::Io(e)
    }
}

/// Information about a video file
pub struct VideoInfo {
    pub duration: Duration,
    pub width: u32,
    pub height: u32,
}

/// Get video info without fully opening for decoding
pub fn get_video_info<P: AsRef<Path>>(path: P) -> Result<VideoInfo, DecoderError> {
    ffmpeg_next::init()?;

    let input_ctx = input(&path)?;

    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or(DecoderError::NoVideoStream)?;

    let time_base = video_stream.time_base();
    let duration_ts = video_stream.duration();

    let duration = if duration_ts > 0 {
        let seconds =
            duration_ts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
        Duration::from_secs_f64(seconds)
    } else {
        let container_duration = input_ctx.duration();
        if container_duration > 0 {
            Duration::from_micros(container_duration as u64)
        } else {
            Duration::ZERO
        }
    };

    let codec_params = video_stream.parameters();
    let decoder_ctx = codec::context::Context::from_parameters(codec_params)?;
    let decoder = decoder_ctx.decoder().video()?;

    Ok(VideoInfo {
        duration,
        width: decoder.width(),
        height: decoder.height(),
    })
}

/// Convert a PTS timestamp to Duration
fn pts_to_duration(pts: i64, time_base: Rational) -> Duration {
    if pts < 0 {
        return Duration::ZERO;
    }
    let seconds = pts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
    Duration::from_secs_f64(seconds.max(0.0))
}

/// Create a VideoToolbox hardware device context (macOS only)
#[cfg(target_os = "macos")]
fn create_hw_device_ctx() -> Option<*mut ffi::AVBufferRef> {
    unsafe {
        let mut hw_device_ctx: *mut ffi::AVBufferRef = ptr::null_mut();
        let ret = ffi::av_hwdevice_ctx_create(
            &mut hw_device_ctx,
            ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            ptr::null(),
            ptr::null_mut(),
            0,
        );
        if ret < 0 {
            eprintln!("Failed to create VideoToolbox device context: {}", ret);
            return None;
        }
        Some(hw_device_ctx)
    }
}

#[cfg(not(target_os = "macos"))]
fn create_hw_device_ctx() -> Option<*mut ffi::AVBufferRef> {
    None
}

/// Check if a frame is in hardware format and needs transfer
fn is_hw_frame(frame: &VideoFrameFFmpeg) -> bool {
    let format = unsafe { (*frame.as_ptr()).format };
    // VideoToolbox uses AV_PIX_FMT_VIDEOTOOLBOX
    format == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32
}

/// Transfer hardware frame to software frame
fn transfer_hw_frame(hw_frame: &VideoFrameFFmpeg) -> Result<VideoFrameFFmpeg, DecoderError> {
    unsafe {
        let mut sw_frame = VideoFrameFFmpeg::empty();
        let ret = ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), hw_frame.as_ptr(), 0);
        if ret < 0 {
            return Err(DecoderError::Ffmpeg(ffmpeg_next::Error::from(ret)));
        }
        // Copy timing info
        (*sw_frame.as_mut_ptr()).pts = (*hw_frame.as_ptr()).pts;
        Ok(sw_frame)
    }
}

/// Decode a video file, pushing frames to the queue until stopped or EOF
pub fn decode_video<P: AsRef<Path>>(
    path: P,
    queue: Arc<FrameQueue>,
    stop_flag: Arc<AtomicBool>,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<(), DecoderError> {
    ffmpeg_next::init()?;

    let mut input_ctx = input(&path)?;

    // Find video stream
    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or(DecoderError::NoVideoStream)?;

    let video_stream_index = video_stream.index();
    let time_base = video_stream.time_base();

    // Create decoder
    let codec_params = video_stream.parameters();
    let decoder_ctx = codec::context::Context::from_parameters(codec_params)?;
    let mut decoder = decoder_ctx.decoder().video()?;

    // Try to enable hardware acceleration
    let hw_device_ctx = create_hw_device_ctx();
    if let Some(hw_ctx) = hw_device_ctx {
        unsafe {
            (*decoder.as_mut_ptr()).hw_device_ctx = ffi::av_buffer_ref(hw_ctx);
        }
        eprintln!("VideoToolbox hardware acceleration enabled");
    } else {
        eprintln!("Using software decoding");
    }

    let mut scaler: Option<ScalerContext> = None;
    let mut decoded_frame = VideoFrameFFmpeg::empty();
    let mut bgra_frame = VideoFrameFFmpeg::empty();

    // Process all packets
    for (stream, packet) in input_ctx.packets() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        if stream.index() != video_stream_index {
            continue;
        }

        decoder.send_packet(&packet)?;

        // Receive all available frames
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            // If this is a hardware frame, transfer to software
            let sw_frame = if is_hw_frame(&decoded_frame) {
                transfer_hw_frame(&decoded_frame)?
            } else {
                // For software frames, we need to copy the data
                let mut copy = VideoFrameFFmpeg::empty();
                copy.clone_from(&decoded_frame);
                copy
            };

            // Initialize scaler on first frame
            if scaler.is_none() {
                let src_width = sw_frame.width();
                let src_height = sw_frame.height();
                let src_format = sw_frame.format();

                let dst_width = target_width.unwrap_or(src_width);
                let dst_height = target_height.unwrap_or(src_height);

                scaler = Some(ScalerContext::get(
                    src_format,
                    src_width,
                    src_height,
                    ffmpeg_next::format::Pixel::BGRA,
                    dst_width,
                    dst_height,
                    ScalerFlags::BILINEAR,
                )?);
            }

            // Scale/convert to BGRA (native format for GPUI)
            let scaler = scaler.as_mut().unwrap();
            scaler.run(&sw_frame, &mut bgra_frame)?;

            let dst_width = bgra_frame.width();
            let dst_height = bgra_frame.height();
            let data = bgra_frame.data(0);
            let stride = bgra_frame.stride(0);
            let pts = pts_to_duration(sw_frame.pts().unwrap_or(0), time_base);

            // Copy data accounting for stride (already in BGRA format for GPUI)
            let mut bgra_data = Vec::with_capacity((dst_width * dst_height * 4) as usize);
            for y in 0..dst_height as usize {
                let row_start = y * stride;
                let row_end = row_start + (dst_width as usize * 4);
                bgra_data.extend_from_slice(&data[row_start..row_end]);
            }

            let frame = VideoFrame::new(bgra_data, dst_width, dst_height, pts);

            // Push to queue (blocks if full)
            if !queue.push(frame) {
                // Queue was closed
                return Ok(());
            }
        }
    }

    // Flush decoder
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        // If this is a hardware frame, transfer to software
        let sw_frame = if is_hw_frame(&decoded_frame) {
            transfer_hw_frame(&decoded_frame)?
        } else {
            let mut copy = VideoFrameFFmpeg::empty();
            copy.clone_from(&decoded_frame);
            copy
        };

        if let Some(ref mut scaler) = scaler {
            scaler.run(&sw_frame, &mut bgra_frame)?;

            let dst_width = bgra_frame.width();
            let dst_height = bgra_frame.height();
            let data = bgra_frame.data(0);
            let stride = bgra_frame.stride(0);
            let pts = pts_to_duration(sw_frame.pts().unwrap_or(0), time_base);

            let mut bgra_data = Vec::with_capacity((dst_width * dst_height * 4) as usize);
            for y in 0..dst_height as usize {
                let row_start = y * stride;
                let row_end = row_start + (dst_width as usize * 4);
                bgra_data.extend_from_slice(&data[row_start..row_end]);
            }

            let frame = VideoFrame::new(bgra_data, dst_width, dst_height, pts);
            if !queue.push(frame) {
                break;
            }
        }
    }

    // Clean up hardware device context
    if let Some(hw_ctx) = hw_device_ctx {
        unsafe {
            ffi::av_buffer_unref(&mut (hw_ctx as *mut _));
        }
    }

    // Signal that decoding is complete
    queue.close();

    Ok(())
}
