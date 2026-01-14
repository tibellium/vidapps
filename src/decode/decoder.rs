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
    packet::Mut as PacketMut,
    software::resampling::context::Context as ResamplerContext,
    software::scaling::{context::Context as ScalerContext, flag::Flags as ScalerFlags},
    util::frame::audio::Audio as AudioFrameFFmpeg,
    util::frame::video::Video as VideoFrameFFmpeg,
};

use super::packet_queue::{Packet, PacketQueue};
use crate::audio::{AudioStreamProducer, DEFAULT_SAMPLE_RATE};
use crate::playback::{FrameQueue, VideoFrame};

/**
    Error type for video decoding operations
*/
#[derive(Debug)]
pub enum DecoderError {
    NoVideoStream,
    NoAudioStream,
    Ffmpeg(ffmpeg_next::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderError::Ffmpeg(e) => write!(f, "FFmpeg error: {}", e),
            DecoderError::NoVideoStream => write!(f, "No video stream found"),
            DecoderError::NoAudioStream => write!(f, "No audio stream found"),
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
    Audio-specific stream info for independent audio pipeline
*/
pub struct AudioStreamInfo {
    pub time_base: Rational,
    pub codec_params: codec::Parameters,
}

/**
    Video-specific stream info for independent video pipeline
*/
pub struct VideoStreamInfo {
    pub time_base: Rational,
    pub codec_params: codec::Parameters,
    pub width: u32,
    pub height: u32,
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
    Get audio stream info (returns error if no audio stream)
*/
pub fn get_audio_stream_info<P: AsRef<Path>>(path: P) -> Result<AudioStreamInfo, DecoderError> {
    ffmpeg_next::init()?;

    let input_ctx = input(&path)?;

    let audio_stream = input_ctx
        .streams()
        .best(Type::Audio)
        .ok_or(DecoderError::NoAudioStream)?;

    Ok(AudioStreamInfo {
        time_base: audio_stream.time_base(),
        codec_params: audio_stream.parameters(),
    })
}

/**
    Get video stream info (returns error if no video stream)
*/
pub fn get_video_stream_info<P: AsRef<Path>>(path: P) -> Result<VideoStreamInfo, DecoderError> {
    ffmpeg_next::init()?;

    let input_ctx = input(&path)?;

    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or(DecoderError::NoVideoStream)?;

    let codec_params = video_stream.parameters();
    let decoder_ctx = codec::context::Context::from_parameters(codec_params.clone())?;
    let decoder = decoder_ctx.decoder().video()?;

    Ok(VideoStreamInfo {
        time_base: video_stream.time_base(),
        codec_params,
        width: decoder.width(),
        height: decoder.height(),
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
    Demux only audio packets from a video file.
    Opens its own file handle - completely independent from video demux.
    This is part of the separated pipeline architecture to prevent deadlocks.

    If `start_position` is provided, seeks to that position before demuxing.
*/
pub fn audio_demux<P: AsRef<Path>>(
    path: P,
    audio_packets: Arc<PacketQueue>,
    stop_flag: Arc<AtomicBool>,
    start_position: Option<Duration>,
) -> Result<(), DecoderError> {
    ffmpeg_next::init()?;

    let mut input_ctx = input(&path)?;

    let audio_stream_index = input_ctx
        .streams()
        .best(Type::Audio)
        .ok_or(DecoderError::NoAudioStream)?
        .index();

    // Seek to start position if specified
    if let Some(pos) = start_position {
        let ts = (pos.as_secs_f64() * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;
        input_ctx.seek(ts, ..ts)?;
    }

    let mut pkt_count = 0u64;

    // Process all packets, but only extract audio
    for (stream, packet) in input_ctx.packets() {
        if stop_flag.load(Ordering::Relaxed) {
            eprintln!("[audio_demux] stopped by flag");
            break;
        }

        // ONLY process audio packets - skip everything else
        if stream.index() == audio_stream_index {
            let pkt = Packet::new(
                packet.data().map(|d| d.to_vec()).unwrap_or_default(),
                packet.pts().unwrap_or(0),
                packet.dts().unwrap_or(0),
                packet.duration(),
                packet.flags().bits(),
            );
            if !audio_packets.push(pkt) {
                eprintln!("[audio_demux] queue closed");
                break;
            }
            pkt_count += 1;
            if pkt_count % 500 == 0 {
                eprintln!("[audio_demux] packets: {}", pkt_count);
            }
        }
    }

    eprintln!("[audio_demux] finished - packets: {}", pkt_count);

    audio_packets.close();
    Ok(())
}

/**
    Demux only video packets from a video file.
    Opens its own file handle - completely independent from audio demux.
    This is part of the separated pipeline architecture to prevent deadlocks.

    If `start_position` is provided, seeks to that position before demuxing.
*/
pub fn video_demux<P: AsRef<Path>>(
    path: P,
    video_packets: Arc<PacketQueue>,
    stop_flag: Arc<AtomicBool>,
    start_position: Option<Duration>,
) -> Result<(), DecoderError> {
    ffmpeg_next::init()?;

    let mut input_ctx = input(&path)?;

    let video_stream_index = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or(DecoderError::NoVideoStream)?
        .index();

    // Seek to start position if specified
    if let Some(pos) = start_position {
        let ts = (pos.as_secs_f64() * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;
        input_ctx.seek(ts, ..ts)?;
    }

    let mut pkt_count = 0u64;

    // Process all packets, but only extract video
    for (stream, packet) in input_ctx.packets() {
        if stop_flag.load(Ordering::Relaxed) {
            eprintln!("[video_demux] stopped by flag");
            break;
        }

        // ONLY process video packets - skip everything else
        if stream.index() == video_stream_index {
            let pkt = Packet::new(
                packet.data().map(|d| d.to_vec()).unwrap_or_default(),
                packet.pts().unwrap_or(0),
                packet.dts().unwrap_or(0),
                packet.duration(),
                packet.flags().bits(),
            );
            if !video_packets.push(pkt) {
                eprintln!("[video_demux] queue closed");
                break;
            }
            pkt_count += 1;
            if pkt_count % 500 == 0 {
                eprintln!("[video_demux] packets: {}", pkt_count);
            }
        }
    }

    eprintln!("[video_demux] finished - packets: {}", pkt_count);

    video_packets.close();
    Ok(())
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
        (*sw_frame.as_mut_ptr()).pts = (*hw_frame.as_ptr()).pts;
        Ok(sw_frame)
    }
}

/**
    Decode video packets to frames.
    Runs until packet queue is closed and empty, or stop flag is set.
*/
pub fn decode_video_packets(
    packets: Arc<PacketQueue>,
    frames: Arc<FrameQueue>,
    codec_params: codec::Parameters,
    time_base: Rational,
    stop_flag: Arc<AtomicBool>,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<(), DecoderError> {
    ffmpeg_next::init()?;

    // Create decoder
    let decoder_ctx = codec::context::Context::from_parameters(codec_params)?;
    let mut decoder = decoder_ctx.decoder().video()?;

    // Try hardware acceleration
    let hw_device_ctx = create_hw_device_ctx();
    if let Some(hw_ctx) = hw_device_ctx {
        unsafe {
            (*decoder.as_mut_ptr()).hw_device_ctx = ffi::av_buffer_ref(hw_ctx);
        }
        eprintln!("VideoToolbox hardware acceleration enabled");
    } else {
        eprintln!("Using software decoding");
    }

    // Scaler state
    let mut scaler: Option<ScalerContext> = None;
    let mut scaler_src_format: Option<ffmpeg_next::format::Pixel> = None;
    let mut scaler_src_width: u32 = 0;
    let mut scaler_src_height: u32 = 0;

    let mut decoded_frame = VideoFrameFFmpeg::empty();
    let mut bgra_frame = VideoFrameFFmpeg::empty();

    let mut frame_count = 0u64;

    // Process packets
    while let Some(pkt) = packets.pop() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        // Create FFmpeg packet from our packet
        let mut ffmpeg_pkt = ffmpeg_next::Packet::empty();
        if !pkt.data.is_empty() {
            ffmpeg_pkt = ffmpeg_next::Packet::copy(&pkt.data);
        }
        unsafe {
            (*ffmpeg_pkt.as_mut_ptr()).pts = pkt.pts;
            (*ffmpeg_pkt.as_mut_ptr()).dts = pkt.dts;
            (*ffmpeg_pkt.as_mut_ptr()).duration = pkt.duration;
        }

        decoder.send_packet(&ffmpeg_pkt)?;

        // Receive all available frames
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            // Transfer from hardware if needed
            let sw_frame = if is_hw_frame(&decoded_frame) {
                transfer_hw_frame(&decoded_frame)?
            } else {
                let mut copy = VideoFrameFFmpeg::empty();
                copy.clone_from(&decoded_frame);
                copy
            };

            // Initialize/reinitialize scaler if needed
            let src_width = sw_frame.width();
            let src_height = sw_frame.height();
            let src_format = sw_frame.format();

            // Validate frame dimensions - swscale will assert on invalid dimensions
            if src_width == 0 || src_height == 0 {
                eprintln!(
                    "[video_decode] skipping frame with invalid dimensions: {}x{}",
                    src_width, src_height
                );
                continue;
            }

            // Check for unsupported pixel format (None indicates unknown format)
            if src_format == ffmpeg_next::format::Pixel::None {
                eprintln!("[video_decode] skipping frame with unknown pixel format");
                continue;
            }

            let needs_new_scaler = scaler.is_none()
                || scaler_src_format != Some(src_format)
                || scaler_src_width != src_width
                || scaler_src_height != src_height;

            if needs_new_scaler {
                let dst_width = target_width.unwrap_or(src_width);
                let dst_height = target_height.unwrap_or(src_height);

                // Ensure destination dimensions are also valid
                if dst_width == 0 || dst_height == 0 {
                    eprintln!(
                        "[video_decode] skipping frame with invalid target dimensions: {}x{}",
                        dst_width, dst_height
                    );
                    continue;
                }

                match ScalerContext::get(
                    src_format,
                    src_width,
                    src_height,
                    ffmpeg_next::format::Pixel::BGRA,
                    dst_width,
                    dst_height,
                    ScalerFlags::BILINEAR,
                ) {
                    Ok(s) => {
                        scaler = Some(s);
                        scaler_src_format = Some(src_format);
                        scaler_src_width = src_width;
                        scaler_src_height = src_height;
                    }
                    Err(e) => {
                        eprintln!(
                            "[video_decode] failed to create scaler for format {:?} {}x{}: {}",
                            src_format, src_width, src_height, e
                        );
                        continue;
                    }
                }
            }

            // Scale to BGRA
            let scaler = scaler.as_mut().unwrap();
            if let Err(e) = scaler.run(&sw_frame, &mut bgra_frame) {
                eprintln!("[video_decode] scaler error: {}", e);
                continue;
            }

            let dst_width = bgra_frame.width();
            let dst_height = bgra_frame.height();
            let data = bgra_frame.data(0);
            let stride = bgra_frame.stride(0);
            let pts = pts_to_duration(sw_frame.pts().unwrap_or(0), time_base);

            // Copy data accounting for stride
            let mut bgra_data = Vec::with_capacity((dst_width * dst_height * 4) as usize);
            for y in 0..dst_height as usize {
                let row_start = y * stride;
                let row_end = row_start + (dst_width as usize * 4);
                bgra_data.extend_from_slice(&data[row_start..row_end]);
            }

            let frame = VideoFrame::new(bgra_data, dst_width, dst_height, pts);

            // Push to frame queue (blocks if full - this is fine, doesn't affect audio)
            if !frames.push(frame) {
                eprintln!("[video_decode] frame queue closed");
                break; // Queue closed
            }
            frame_count += 1;
            if frame_count % 100 == 0 {
                eprintln!("[video_decode] frames decoded: {}", frame_count);
            }
        }
    }

    // Flush decoder
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let sw_frame = if is_hw_frame(&decoded_frame) {
            transfer_hw_frame(&decoded_frame)?
        } else {
            let mut copy = VideoFrameFFmpeg::empty();
            copy.clone_from(&decoded_frame);
            copy
        };

        // Validate frame before scaling
        let src_width = sw_frame.width();
        let src_height = sw_frame.height();
        if src_width == 0 || src_height == 0 {
            continue;
        }

        if let Some(ref mut scaler) = scaler {
            if let Err(e) = scaler.run(&sw_frame, &mut bgra_frame) {
                eprintln!("[video_decode] scaler error during flush: {}", e);
                continue;
            }

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
            if !frames.push(frame) {
                break;
            }
        }
    }

    // Clean up hardware context
    if let Some(hw_ctx) = hw_device_ctx {
        unsafe {
            ffi::av_buffer_unref(&mut (hw_ctx as *mut _));
        }
    }

    // Signal completion
    frames.close();

    Ok(())
}

/**
    Decode audio packets to samples.
    Runs until packet queue is closed and empty, or stop flag is set.
*/
pub fn decode_audio_packets(
    packets: Arc<PacketQueue>,
    producer: Arc<AudioStreamProducer>,
    codec_params: codec::Parameters,
    _time_base: Rational,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), DecoderError> {
    ffmpeg_next::init()?;

    // Create decoder
    let decoder_ctx = codec::context::Context::from_parameters(codec_params)?;
    let mut decoder = decoder_ctx.decoder().audio()?;

    let mut resampler: Option<ResamplerContext> = None;
    let mut decoded_frame = AudioFrameFFmpeg::empty();
    let mut resampled_frame = AudioFrameFFmpeg::empty();

    let mut packet_count = 0u64;
    let mut sample_count = 0u64;

    // Process packets
    while let Some(pkt) = packets.pop() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        // Create FFmpeg packet
        let mut ffmpeg_pkt = ffmpeg_next::Packet::empty();
        if !pkt.data.is_empty() {
            ffmpeg_pkt = ffmpeg_next::Packet::copy(&pkt.data);
        }
        unsafe {
            (*ffmpeg_pkt.as_mut_ptr()).pts = pkt.pts;
            (*ffmpeg_pkt.as_mut_ptr()).dts = pkt.dts;
            (*ffmpeg_pkt.as_mut_ptr()).duration = pkt.duration;
        }

        decoder.send_packet(&ffmpeg_pkt)?;

        // Receive all available frames
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            // Initialize resampler if needed
            if resampler.is_none() {
                let src_format = decoder.format();
                let src_channel_layout = decoder.channel_layout();
                let src_rate = decoder.rate();

                match ResamplerContext::get(
                    src_format,
                    src_channel_layout,
                    src_rate,
                    ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                    ChannelLayout::STEREO,
                    DEFAULT_SAMPLE_RATE,
                ) {
                    Ok(r) => resampler = Some(r),
                    Err(e) => {
                        eprintln!("Failed to create audio resampler: {}", e);
                        continue;
                    }
                }
            }

            if let Some(ref mut resampler) = resampler {
                if let Err(e) = resampler.run(&decoded_frame, &mut resampled_frame) {
                    eprintln!("Audio resampling error: {}", e);
                    continue;
                }

                let samples = resampled_frame.samples();
                let channels = 2u16;
                let plane_data = resampled_frame.data(0);

                let float_samples: Vec<f32> = plane_data
                    .chunks_exact(4)
                    .take(samples * channels as usize)
                    .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                if !float_samples.is_empty() {
                    // Push to ring buffer (blocks if full)
                    if !producer.push(&float_samples) {
                        eprintln!("[audio_decode] producer closed");
                        break;
                    }
                    sample_count += float_samples.len() as u64;
                }
            }
        }
        packet_count += 1;
        if packet_count % 500 == 0 {
            eprintln!(
                "[audio_decode] packets: {}, samples: {}",
                packet_count, sample_count
            );
        }
    }

    eprintln!(
        "[audio_decode] finished - packets: {}, samples: {}",
        packet_count, sample_count
    );

    // Flush decoder
    let _ = decoder.send_eof();
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        if let Some(ref mut resampler) = resampler {
            if let Err(e) = resampler.run(&decoded_frame, &mut resampled_frame) {
                eprintln!("Audio resampling error during flush: {}", e);
                continue;
            }

            let samples = resampled_frame.samples();
            let channels = 2u16;
            let plane_data = resampled_frame.data(0);

            let float_samples: Vec<f32> = plane_data
                .chunks_exact(4)
                .take(samples * channels as usize)
                .map(|chunk| f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            if !float_samples.is_empty() && !producer.push(&float_samples) {
                break;
            }
        }
    }

    // Signal completion
    producer.close();

    Ok(())
}
