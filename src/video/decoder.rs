use std::path::Path;
use std::ptr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use ffmpeg_next::{
    ChannelLayout, Rational, codec, ffi,
    format::input,
    media::Type,
    software::resampling::context::Context as ResamplerContext,
    software::scaling::{context::Context as ScalerContext, flag::Flags as ScalerFlags},
    util::frame::audio::Audio as AudioFrameFFmpeg,
    util::frame::video::Video as VideoFrameFFmpeg,
};

use super::frame::VideoFrame;
use super::queue::FrameQueue;
use crate::audio::{AudioStreamProducer, DEFAULT_SAMPLE_RATE};

/**
    Error type for video decoding operations
*/
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

/**
    Information about a video file
*/
pub struct VideoInfo {
    pub duration: Duration,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
}

/**
    Get video info without fully opening for decoding
*/
pub fn get_video_info<P: AsRef<Path>>(path: P) -> Result<VideoInfo, DecoderError> {
    ffmpeg_next::init()?;

    let input_ctx = input(&path)?;

    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or(DecoderError::NoVideoStream)?;

    let has_audio = input_ctx.streams().best(Type::Audio).is_some();

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
        has_audio,
    })
}

/**
    Convert a PTS timestamp to Duration
*/
fn pts_to_duration(pts: i64, time_base: Rational) -> Duration {
    if pts < 0 {
        return Duration::ZERO;
    }
    let seconds = pts as f64 * time_base.numerator() as f64 / time_base.denominator() as f64;
    Duration::from_secs_f64(seconds.max(0.0))
}

/**
    Create a VideoToolbox hardware device context (macOS only)
*/
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

/**
    Check if a frame is in hardware format and needs transfer
*/
fn is_hw_frame(frame: &VideoFrameFFmpeg) -> bool {
    let format = unsafe { (*frame.as_ptr()).format };
    // VideoToolbox uses AV_PIX_FMT_VIDEOTOOLBOX
    format == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX as i32
}

/**
    Transfer hardware frame to software frame
*/
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

/**
    Decode a video file, pushing frames to the queues until stopped or EOF.
    If audio_producer is provided, audio will also be decoded and pushed directly to the ring buffer.
*/
pub fn decode_video<P: AsRef<Path>>(
    path: P,
    video_queue: Arc<FrameQueue>,
    audio_producer: Option<Arc<AudioStreamProducer>>,
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
    let video_time_base = video_stream.time_base();

    // Create video decoder
    let video_codec_params = video_stream.parameters();
    let video_decoder_ctx = codec::context::Context::from_parameters(video_codec_params)?;
    let mut video_decoder = video_decoder_ctx.decoder().video()?;

    // Try to enable hardware acceleration
    let hw_device_ctx = create_hw_device_ctx();
    if let Some(hw_ctx) = hw_device_ctx {
        unsafe {
            (*video_decoder.as_mut_ptr()).hw_device_ctx = ffi::av_buffer_ref(hw_ctx);
        }
        eprintln!("VideoToolbox hardware acceleration enabled");
    } else {
        eprintln!("Using software decoding");
    }

    // Find audio stream (optional)
    let audio_stream_info = if audio_producer.is_some() {
        input_ctx.streams().best(Type::Audio).map(|stream| {
            let index = stream.index();
            let time_base = stream.time_base();
            let params = stream.parameters();
            (index, time_base, params)
        })
    } else {
        None
    };

    // Create audio decoder if audio stream exists
    let mut audio_decoder: Option<codec::decoder::Audio> = None;
    let mut audio_resampler: Option<ResamplerContext> = None;
    let mut audio_stream_index: Option<usize> = None;

    if let Some((index, _time_base, params)) = audio_stream_info {
        match codec::context::Context::from_parameters(params) {
            Ok(ctx) => match ctx.decoder().audio() {
                Ok(decoder) => {
                    audio_stream_index = Some(index);
                    audio_decoder = Some(decoder);
                }
                Err(e) => {
                    eprintln!("Failed to create audio decoder: {}", e);
                }
            },
            Err(e) => {
                eprintln!("Failed to create audio codec context: {}", e);
            }
        }
    }

    // Video state
    let mut scaler: Option<ScalerContext> = None;
    let mut scaler_src_format: Option<ffmpeg_next::format::Pixel> = None;
    let mut scaler_src_width: u32 = 0;
    let mut scaler_src_height: u32 = 0;
    let mut decoded_video_frame = VideoFrameFFmpeg::empty();
    let mut bgra_frame = VideoFrameFFmpeg::empty();

    // Audio state
    let mut decoded_audio_frame = AudioFrameFFmpeg::empty();
    let mut resampled_audio_frame = AudioFrameFFmpeg::empty();

    // Process all packets
    for (stream, packet) in input_ctx.packets() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let stream_index = stream.index();

        // Video packet
        if stream_index == video_stream_index {
            video_decoder.send_packet(&packet)?;

            // Receive all available video frames
            while video_decoder
                .receive_frame(&mut decoded_video_frame)
                .is_ok()
            {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }

                // If this is a hardware frame, transfer to software
                let sw_frame = if is_hw_frame(&decoded_video_frame) {
                    transfer_hw_frame(&decoded_video_frame)?
                } else {
                    let mut copy = VideoFrameFFmpeg::empty();
                    copy.clone_from(&decoded_video_frame);
                    copy
                };

                // Initialize or reinitialize scaler if frame properties changed
                let src_width = sw_frame.width();
                let src_height = sw_frame.height();
                let src_format = sw_frame.format();

                let needs_new_scaler = scaler.is_none()
                    || scaler_src_format != Some(src_format)
                    || scaler_src_width != src_width
                    || scaler_src_height != src_height;

                if needs_new_scaler {
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
                    scaler_src_format = Some(src_format);
                    scaler_src_width = src_width;
                    scaler_src_height = src_height;
                }

                // Scale/convert to BGRA (native format for GPUI)
                let scaler = scaler.as_mut().unwrap();
                scaler.run(&sw_frame, &mut bgra_frame)?;

                let dst_width = bgra_frame.width();
                let dst_height = bgra_frame.height();
                let data = bgra_frame.data(0);
                let stride = bgra_frame.stride(0);
                let pts = pts_to_duration(sw_frame.pts().unwrap_or(0), video_time_base);

                // Copy data accounting for stride
                let mut bgra_data = Vec::with_capacity((dst_width * dst_height * 4) as usize);
                for y in 0..dst_height as usize {
                    let row_start = y * stride;
                    let row_end = row_start + (dst_width as usize * 4);
                    bgra_data.extend_from_slice(&data[row_start..row_end]);
                }

                let frame = VideoFrame::new(bgra_data, dst_width, dst_height, pts);

                // Push to queue (blocks if full - natural backpressure)
                if !video_queue.push(frame) {
                    // Queue was closed
                    return Ok(());
                }
            }
        }
        // Audio packet
        else if Some(stream_index) == audio_stream_index {
            if let (Some(decoder), Some(producer)) = (&mut audio_decoder, &audio_producer) {
                decoder.send_packet(&packet)?;

                // Receive all available audio frames
                while decoder.receive_frame(&mut decoded_audio_frame).is_ok() {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    // Initialize or reinitialize resampler if needed
                    let needs_new_resampler = audio_resampler.is_none();

                    if needs_new_resampler {
                        let src_format = decoder.format();
                        let src_channel_layout = decoder.channel_layout();
                        let src_rate = decoder.rate();

                        match ResamplerContext::get(
                            src_format,
                            src_channel_layout,
                            src_rate,
                            ffmpeg_next::format::Sample::F32(
                                ffmpeg_next::format::sample::Type::Packed,
                            ),
                            ChannelLayout::STEREO,
                            DEFAULT_SAMPLE_RATE,
                        ) {
                            Ok(resampler) => {
                                audio_resampler = Some(resampler);
                            }
                            Err(e) => {
                                eprintln!("Failed to create audio resampler: {}", e);
                                continue;
                            }
                        }
                    }

                    if let Some(ref mut resampler) = audio_resampler {
                        // Run resampler
                        if let Err(e) =
                            resampler.run(&decoded_audio_frame, &mut resampled_audio_frame)
                        {
                            eprintln!("Audio resampling error: {}", e);
                            continue;
                        }

                        // Extract f32 samples from resampled frame
                        let samples = resampled_audio_frame.samples();
                        let channels = 2u16; // Stereo output
                        let plane_data = resampled_audio_frame.data(0);

                        // Convert bytes to f32 samples
                        let float_samples: Vec<f32> = plane_data
                            .chunks_exact(4)
                            .take(samples * channels as usize)
                            .map(|chunk| {
                                f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                            })
                            .collect();

                        if !float_samples.is_empty() {
                            // Push directly to ring buffer (lock-free, non-blocking)
                            // If buffer is full, we drop samples - this shouldn't happen
                            // if the ring buffer is sized appropriately
                            producer.push(&float_samples);
                        }
                    }
                }
            }
        }
    }

    // Flush video decoder
    video_decoder.send_eof()?;
    while video_decoder
        .receive_frame(&mut decoded_video_frame)
        .is_ok()
    {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let sw_frame = if is_hw_frame(&decoded_video_frame) {
            transfer_hw_frame(&decoded_video_frame)?
        } else {
            let mut copy = VideoFrameFFmpeg::empty();
            copy.clone_from(&decoded_video_frame);
            copy
        };

        let src_width = sw_frame.width();
        let src_height = sw_frame.height();
        let src_format = sw_frame.format();

        let needs_new_scaler = scaler.is_none()
            || scaler_src_format != Some(src_format)
            || scaler_src_width != src_width
            || scaler_src_height != src_height;

        if needs_new_scaler {
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
            scaler_src_format = Some(src_format);
            scaler_src_width = src_width;
            scaler_src_height = src_height;
        }

        if let Some(ref mut scaler) = scaler {
            scaler.run(&sw_frame, &mut bgra_frame)?;

            let dst_width = bgra_frame.width();
            let dst_height = bgra_frame.height();
            let data = bgra_frame.data(0);
            let stride = bgra_frame.stride(0);
            let pts = pts_to_duration(sw_frame.pts().unwrap_or(0), video_time_base);

            let mut bgra_data = Vec::with_capacity((dst_width * dst_height * 4) as usize);
            for y in 0..dst_height as usize {
                let row_start = y * stride;
                let row_end = row_start + (dst_width as usize * 4);
                bgra_data.extend_from_slice(&data[row_start..row_end]);
            }

            let frame = VideoFrame::new(bgra_data, dst_width, dst_height, pts);
            if !video_queue.push(frame) {
                break;
            }
        }
    }

    // Flush audio decoder
    if let Some(ref mut decoder) = audio_decoder {
        let _ = decoder.send_eof();
        while decoder.receive_frame(&mut decoded_audio_frame).is_ok() {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            if let (Some(resampler), Some(producer)) = (&mut audio_resampler, &audio_producer) {
                if let Err(e) = resampler.run(&decoded_audio_frame, &mut resampled_audio_frame) {
                    eprintln!("Audio resampling error during flush: {}", e);
                    continue;
                }

                let samples = resampled_audio_frame.samples();
                let channels = 2u16;
                let plane_data = resampled_audio_frame.data(0);

                let float_samples: Vec<f32> = plane_data
                    .chunks_exact(4)
                    .take(samples * channels as usize)
                    .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                if !float_samples.is_empty() {
                    producer.push(&float_samples);
                }
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
    video_queue.close();
    if let Some(ref producer) = audio_producer {
        producer.close();
    }

    Ok(())
}
