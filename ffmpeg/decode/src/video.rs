/*!
    Video decoder implementation.
*/

use ffmpeg_next::{
    codec::{self, decoder::Video as VideoDecoderFFmpeg},
    ffi,
    packet::Mut as PacketMut,
    util::frame::video::Video as VideoFrameFFmpeg,
};

use ffmpeg_source::CodecConfig;
use ffmpeg_types::{Error, Packet, PixelFormat, Pts, Rational, Result, VideoFrame};

use crate::config::VideoDecoderConfig;
use crate::hw::{HwDeviceContext, is_hw_frame, transfer_hw_frame};

/**
    Video decoder.

    Decodes video packets into frames. Supports hardware acceleration
    when available and configured.
*/
pub struct VideoDecoder {
    decoder: VideoDecoderFFmpeg,
    time_base: Rational,
    /**
        Kept alive to prevent the hardware device context from being dropped
        while the decoder is using it. Not accessed directly after initialization.
    */
    _hw_context: Option<HwDeviceContext>,
    is_hw_accelerated: bool,
}

impl VideoDecoder {
    /**
        Create a new video decoder from codec configuration.

        # Arguments

        * `codec_config` - Codec configuration from the source
        * `time_base` - Time base for the video stream
        * `config` - Decoder configuration (hardware acceleration, etc.)
    */
    pub fn new(
        codec_config: CodecConfig,
        time_base: Rational,
        config: VideoDecoderConfig,
    ) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        let parameters = codec_config.into_parameters();

        let decoder_ctx = codec::context::Context::from_parameters(parameters)
            .map_err(|e| Error::codec(e.to_string()))?;

        let mut decoder = decoder_ctx
            .decoder()
            .video()
            .map_err(|e| Error::codec(e.to_string()))?;

        // Try to set up hardware acceleration
        let (hw_context, is_hw_accelerated) = if config.prefer_hw {
            if let Some(hw_ctx) = HwDeviceContext::try_create(config.hw_device) {
                unsafe {
                    (*decoder.as_mut_ptr()).hw_device_ctx = hw_ctx.create_ref();
                }
                (Some(hw_ctx), true)
            } else {
                (None, false)
            }
        } else {
            (None, false)
        };

        Ok(Self {
            decoder,
            time_base,
            _hw_context: hw_context,
            is_hw_accelerated,
        })
    }

    /**
        Check if hardware acceleration is active.
    */
    pub fn is_hw_accelerated(&self) -> bool {
        self.is_hw_accelerated
    }

    /**
        Get the time base for this decoder.
    */
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /**
        Decode a packet, returning decoded frames.

        May return zero, one, or multiple frames depending on codec buffering.
        B-frames cause the decoder to buffer frames internally.
    */
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<VideoFrame>> {
        // Create FFmpeg packet from our packet
        let mut ffmpeg_pkt = if packet.data.is_empty() {
            ffmpeg_next::Packet::empty()
        } else {
            ffmpeg_next::Packet::copy(&packet.data)
        };

        // Set timing info
        unsafe {
            let pkt_ptr = ffmpeg_pkt.as_mut_ptr();
            if let Some(pts) = packet.pts {
                (*pkt_ptr).pts = pts.0;
            }
            if let Some(dts) = packet.dts {
                (*pkt_ptr).dts = dts.0;
            }
            (*pkt_ptr).duration = packet.duration.0;
        }

        // Send packet to decoder
        // EAGAIN means decoder buffer is full - receive frames first then retry
        match self.decoder.send_packet(&ffmpeg_pkt) {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => {
                // Decoder buffer full - drain frames first
                let mut all_frames = self.receive_frames()?;
                // Retry sending packet - if still EAGAIN, just return what we have
                match self.decoder.send_packet(&ffmpeg_pkt) {
                    Ok(()) => {
                        all_frames.extend(self.receive_frames()?);
                    }
                    Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => {
                        // Still can't send - just return the frames we drained
                    }
                    Err(e) => return Err(Error::codec(e.to_string())),
                }
                return Ok(all_frames);
            }
            Err(e) => return Err(Error::codec(e.to_string())),
        }

        // Receive all available frames
        self.receive_frames()
    }

    /**
        Flush the decoder to get any remaining buffered frames.

        Call this at end of stream to retrieve frames the decoder has buffered.
    */
    pub fn flush(&mut self) -> Result<Vec<VideoFrame>> {
        // First drain any pending frames
        let mut all_frames = self.receive_frames()?;

        // Send EOF - EAGAIN here means we need to drain more frames first
        match self.decoder.send_eof() {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => {
                // Drain frames and retry
                all_frames.extend(self.receive_frames()?);
                let _ = self.decoder.send_eof(); // Ignore error on retry
            }
            Err(ffmpeg_next::Error::Eof) => {
                // Already at EOF, that's fine
            }
            Err(e) => return Err(Error::codec(e.to_string())),
        }

        // Receive remaining frames after EOF
        all_frames.extend(self.receive_frames()?);
        Ok(all_frames)
    }

    /**
        Reset the decoder after a seek.

        Clears internal buffers. Call this after seeking to discard
        frames from the old position.
    */
    pub fn reset(&mut self) {
        self.decoder.flush();
    }

    /**
        Receive all available frames from the decoder.
    */
    fn receive_frames(&mut self) -> Result<Vec<VideoFrame>> {
        let mut frames = Vec::new();
        let mut decoded_frame = VideoFrameFFmpeg::empty();

        loop {
            match self.decoder.receive_frame(&mut decoded_frame) {
                Ok(()) => {
                    // Convert frame
                    match self.convert_frame(&decoded_frame) {
                        Ok(frame) => frames.push(frame),
                        Err(e) => {
                            // Log but continue - some frames may be corrupt
                            eprintln!("[video_decode] frame conversion error: {}", e);
                        }
                    }
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => {
                    // Need more input
                    break;
                }
                Err(ffmpeg_next::Error::Eof) => {
                    // No more frames
                    break;
                }
                Err(e) => {
                    // Only return error if we haven't collected any frames yet
                    // Otherwise just break and return what we have
                    if frames.is_empty() {
                        return Err(Error::codec(e.to_string()));
                    }
                    break;
                }
            }
        }

        Ok(frames)
    }

    /**
        Convert an FFmpeg frame to our VideoFrame type.
    */
    fn convert_frame(&self, frame: &VideoFrameFFmpeg) -> Result<VideoFrame> {
        // Handle hardware frames
        let sw_frame = if is_hw_frame(frame) {
            transfer_hw_frame(frame).map_err(|e| Error::codec(e.to_string()))?
        } else {
            let mut copy = VideoFrameFFmpeg::empty();
            copy.clone_from(frame);
            copy
        };

        // Validate dimensions
        let width = sw_frame.width();
        let height = sw_frame.height();

        if width == 0 || height == 0 {
            return Err(Error::invalid_data("frame has zero dimensions"));
        }

        // Get pixel format
        let ffmpeg_format = sw_frame.format();
        let format = pixel_format_from_ffmpeg(ffmpeg_format).ok_or_else(|| {
            Error::unsupported_format(format!("unsupported pixel format: {:?}", ffmpeg_format))
        })?;

        // Get PTS
        let pts = sw_frame.pts().map(Pts);

        // Copy frame data
        let data = copy_frame_data(&sw_frame, format)?;

        Ok(VideoFrame::new(
            data,
            width,
            height,
            format,
            pts,
            self.time_base,
        ))
    }
}

/**
    Copy frame data from FFmpeg frame to a contiguous buffer.
*/
fn copy_frame_data(frame: &VideoFrameFFmpeg, format: PixelFormat) -> Result<Vec<u8>> {
    match format {
        // Packed formats - single plane
        PixelFormat::Bgra | PixelFormat::Rgba => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let bytes_per_pixel = 4;
            let stride = frame.stride(0);
            let data = frame.data(0);

            let mut output = Vec::with_capacity(width * height * bytes_per_pixel);

            for y in 0..height {
                let row_start = y * stride;
                let row_end = row_start + width * bytes_per_pixel;
                output.extend_from_slice(&data[row_start..row_end]);
            }

            Ok(output)
        }

        PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let bytes_per_pixel = 3;
            let stride = frame.stride(0);
            let data = frame.data(0);

            let mut output = Vec::with_capacity(width * height * bytes_per_pixel);

            for y in 0..height {
                let row_start = y * stride;
                let row_end = row_start + width * bytes_per_pixel;
                output.extend_from_slice(&data[row_start..row_end]);
            }

            Ok(output)
        }

        // Planar formats - copy all planes
        PixelFormat::Yuv420p
        | PixelFormat::Yuv422p
        | PixelFormat::Yuv444p
        | PixelFormat::Yuv420p10 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;

            // Calculate plane sizes based on format
            let (y_size, uv_height) = match format {
                PixelFormat::Yuv420p | PixelFormat::Yuv420p10 => (width * height, height / 2),
                PixelFormat::Yuv422p => (width * height, height),
                PixelFormat::Yuv444p => (width * height, height),
                _ => unreachable!(),
            };

            let uv_width = match format {
                PixelFormat::Yuv420p | PixelFormat::Yuv422p | PixelFormat::Yuv420p10 => width / 2,
                PixelFormat::Yuv444p => width,
                _ => unreachable!(),
            };

            let bytes_per_sample = if format == PixelFormat::Yuv420p10 {
                2
            } else {
                1
            };
            let total_size = (y_size + 2 * uv_width * uv_height) * bytes_per_sample;

            let mut output = Vec::with_capacity(total_size);

            // Copy Y plane
            let y_stride = frame.stride(0);
            let y_data = frame.data(0);
            for y in 0..height {
                let row_start = y * y_stride;
                let row_end = row_start + width * bytes_per_sample;
                output.extend_from_slice(&y_data[row_start..row_end]);
            }

            // Copy U plane
            let u_stride = frame.stride(1);
            let u_data = frame.data(1);
            for y in 0..uv_height {
                let row_start = y * u_stride;
                let row_end = row_start + uv_width * bytes_per_sample;
                output.extend_from_slice(&u_data[row_start..row_end]);
            }

            // Copy V plane
            let v_stride = frame.stride(2);
            let v_data = frame.data(2);
            for y in 0..uv_height {
                let row_start = y * v_stride;
                let row_end = row_start + uv_width * bytes_per_sample;
                output.extend_from_slice(&v_data[row_start..row_end]);
            }

            Ok(output)
        }

        // NV12 - semi-planar 8-bit
        PixelFormat::Nv12 => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let y_size = width * height;
            let uv_size = width * (height / 2);

            let mut output = Vec::with_capacity(y_size + uv_size);

            // Copy Y plane
            let y_stride = frame.stride(0);
            let y_data = frame.data(0);
            for y in 0..height {
                let row_start = y * y_stride;
                let row_end = row_start + width;
                output.extend_from_slice(&y_data[row_start..row_end]);
            }

            // Copy UV plane (interleaved)
            let uv_stride = frame.stride(1);
            let uv_data = frame.data(1);
            for y in 0..(height / 2) {
                let row_start = y * uv_stride;
                let row_end = row_start + width;
                output.extend_from_slice(&uv_data[row_start..row_end]);
            }

            Ok(output)
        }

        // P010 - semi-planar 10-bit (stored in 16-bit, 2 bytes per sample)
        PixelFormat::P010le => {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let bytes_per_sample = 2; // 16-bit storage for 10-bit data
            let y_size = width * height * bytes_per_sample;
            let uv_size = width * (height / 2) * bytes_per_sample;

            let mut output = Vec::with_capacity(y_size + uv_size);

            // Copy Y plane (16-bit per sample)
            let y_stride = frame.stride(0);
            let y_data = frame.data(0);
            for y in 0..height {
                let row_start = y * y_stride;
                let row_end = row_start + width * bytes_per_sample;
                output.extend_from_slice(&y_data[row_start..row_end]);
            }

            // Copy UV plane (interleaved, 16-bit per component)
            let uv_stride = frame.stride(1);
            let uv_data = frame.data(1);
            for y in 0..(height / 2) {
                let row_start = y * uv_stride;
                let row_end = row_start + width * bytes_per_sample;
                output.extend_from_slice(&uv_data[row_start..row_end]);
            }

            Ok(output)
        }

        // Handle future pixel format variants
        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported for frame copy",
            format
        ))),
    }
}

/**
    Convert FFmpeg pixel format to our PixelFormat.
*/
fn pixel_format_from_ffmpeg(format: ffmpeg_next::format::Pixel) -> Option<PixelFormat> {
    use ffmpeg_next::format::Pixel;

    match format {
        Pixel::YUV420P => Some(PixelFormat::Yuv420p),
        Pixel::NV12 => Some(PixelFormat::Nv12),
        Pixel::BGRA => Some(PixelFormat::Bgra),
        Pixel::RGBA => Some(PixelFormat::Rgba),
        Pixel::RGB24 => Some(PixelFormat::Rgb24),
        Pixel::BGR24 => Some(PixelFormat::Bgr24),
        Pixel::YUV422P => Some(PixelFormat::Yuv422p),
        Pixel::YUV444P => Some(PixelFormat::Yuv444p),
        Pixel::YUV420P10LE | Pixel::YUV420P10BE => Some(PixelFormat::Yuv420p10),
        Pixel::P010LE | Pixel::P010BE => Some(PixelFormat::P010le),
        _ => None,
    }
}

impl std::fmt::Debug for VideoDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoDecoder")
            .field("time_base", &self.time_base)
            .field("is_hw_accelerated", &self.is_hw_accelerated)
            .finish_non_exhaustive()
    }
}
