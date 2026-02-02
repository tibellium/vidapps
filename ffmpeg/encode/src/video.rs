/*!
    Video encoder implementation.
*/

use ffmpeg_next::{
    Dictionary, Rational as FFmpegRational,
    codec::{self, Id as CodecIdFFmpeg, encoder::Video as VideoEncoderFFmpeg},
    ffi,
    util::frame::video::Video as VideoFrameFFmpeg,
};

use ffmpeg_types::{
    CodecId, Error, MediaDuration, Packet, PixelFormat, Pts, Rational, Result, StreamType,
    VideoFrame, VideoStreamInfo,
};

use crate::config::{RateControl, VideoEncoderConfig};

/**
    Video encoder.

    Encodes raw video frames into compressed packets.
*/
pub struct VideoEncoder {
    encoder: VideoEncoderFFmpeg,
    time_base: Rational,
    frame_count: i64,
}

impl VideoEncoder {
    /**
        Create a new video encoder with the given configuration.
    */
    pub fn new(config: VideoEncoderConfig) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        // Find the codec
        let codec_id = codec_id_to_ffmpeg(config.codec)?;
        let codec = ffmpeg_next::encoder::find(codec_id).ok_or_else(|| {
            Error::unsupported_format(format!("codec {:?} not found", config.codec))
        })?;

        // Create encoder context
        let encoder_ctx = codec::context::Context::new_with_codec(codec);
        let mut encoder = encoder_ctx
            .encoder()
            .video()
            .map_err(|e| Error::codec(e.to_string()))?;

        // Set dimensions
        encoder.set_width(config.width);
        encoder.set_height(config.height);

        // Set pixel format
        let pixel_format = pixel_format_to_ffmpeg(config.pixel_format)?;
        encoder.set_format(pixel_format);

        // Set frame rate and time base
        let frame_rate = FFmpegRational::new(config.frame_rate.num, config.frame_rate.den);
        encoder.set_frame_rate(Some(frame_rate));

        // Time base is inverse of frame rate for video
        let time_base = FFmpegRational::new(config.frame_rate.den, config.frame_rate.num);
        encoder.set_time_base(time_base);

        // Set GOP size (keyframe interval)
        if let Some(gop) = config.keyframe_interval {
            encoder.set_gop(gop);
        } else {
            // Default: keyframe every 2 seconds
            let fps = config.frame_rate.num as f64 / config.frame_rate.den as f64;
            encoder.set_gop((fps * 2.0) as u32);
        }

        // Build encoder options
        let mut opts = Dictionary::new();

        // Set preset
        opts.set("preset", config.preset.as_str());

        // Set rate control
        match config.rate_control {
            RateControl::Crf(crf) => {
                opts.set("crf", &crf.to_string());
            }
            RateControl::Cbr(bitrate) => {
                encoder.set_bit_rate(bitrate as usize);
                encoder.set_max_bit_rate(bitrate as usize);
                opts.set("rc", "cbr");
            }
            RateControl::Vbr(bitrate) => {
                encoder.set_bit_rate(bitrate as usize);
            }
        }

        // Open the encoder
        let encoder = encoder
            .open_with(opts)
            .map_err(|e| Error::codec(format!("failed to open encoder: {}", e)))?;

        let time_base = Rational {
            num: config.frame_rate.den,
            den: config.frame_rate.num,
        };

        Ok(Self {
            encoder,
            time_base,
            frame_count: 0,
        })
    }

    /**
        Get the time base for encoded packets.
    */
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /**
        Get stream info for the muxer.
    */
    pub fn stream_info(&self) -> VideoStreamInfo {
        let frame_rate = Rational {
            num: self.time_base.den,
            den: self.time_base.num,
        };

        VideoStreamInfo {
            width: self.encoder.width(),
            height: self.encoder.height(),
            pixel_format: ffmpeg_pixel_format_to_ours(self.encoder.format()),
            frame_rate: Some(frame_rate),
            time_base: self.time_base,
            duration: None,
            codec_id: CodecId::H264, // TODO: track actual codec
        }
    }

    /**
        Encode a video frame, returning encoded packets.

        May return zero, one, or multiple packets depending on encoder buffering.
    */
    pub fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<Packet>> {
        // Validate input dimensions
        if frame.width != self.encoder.width() || frame.height != self.encoder.height() {
            return Err(Error::invalid_data(format!(
                "frame dimensions {}x{} don't match encoder {}x{}",
                frame.width,
                frame.height,
                self.encoder.width(),
                self.encoder.height()
            )));
        }

        // Create FFmpeg frame
        let pixel_format = pixel_format_to_ffmpeg(frame.format)?;
        let mut ffmpeg_frame = VideoFrameFFmpeg::new(pixel_format, frame.width, frame.height);

        // Copy data into FFmpeg frame
        copy_data_to_ffmpeg_frame(&mut ffmpeg_frame, frame)?;

        // Set PTS
        let pts = if let Some(p) = frame.pts {
            p.0
        } else {
            self.frame_count
        };
        ffmpeg_frame.set_pts(Some(pts));
        self.frame_count += 1;

        // Send frame to encoder
        self.encoder
            .send_frame(&ffmpeg_frame)
            .map_err(|e| Error::codec(e.to_string()))?;

        // Receive all available packets
        self.receive_packets()
    }

    /**
        Flush the encoder to get any remaining buffered packets.

        Call this at end of stream.
    */
    pub fn flush(&mut self) -> Result<Vec<Packet>> {
        self.encoder
            .send_eof()
            .map_err(|e| Error::codec(e.to_string()))?;

        self.receive_packets()
    }

    /**
        Receive all available packets from the encoder.
    */
    fn receive_packets(&mut self) -> Result<Vec<Packet>> {
        let mut packets = Vec::new();
        let mut encoded_pkt = ffmpeg_next::Packet::empty();

        loop {
            match self.encoder.receive_packet(&mut encoded_pkt) {
                Ok(()) => {
                    let packet = self.convert_packet(&encoded_pkt)?;
                    packets.push(packet);
                }
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::AVERROR(ffi::EAGAIN) => {
                    break;
                }
                Err(ffmpeg_next::Error::Eof) => {
                    break;
                }
                Err(e) => {
                    return Err(Error::codec(e.to_string()));
                }
            }
        }

        Ok(packets)
    }

    /**
        Convert an FFmpeg packet to our Packet type.
    */
    fn convert_packet(&self, pkt: &ffmpeg_next::Packet) -> Result<Packet> {
        let data = pkt.data().map(|d| d.to_vec()).unwrap_or_default();
        let pts = pkt.pts().map(Pts);
        let dts = pkt.dts().map(Pts);
        let duration = MediaDuration(pkt.duration());
        let is_keyframe = pkt.is_key();

        Ok(Packet {
            data,
            pts,
            dts,
            duration,
            time_base: self.time_base,
            is_keyframe,
            stream_type: StreamType::Video,
        })
    }
}

/**
    Convert our CodecId to FFmpeg's codec ID.
*/
fn codec_id_to_ffmpeg(codec: CodecId) -> Result<CodecIdFFmpeg> {
    match codec {
        CodecId::H264 => Ok(CodecIdFFmpeg::H264),
        CodecId::H265 => Ok(CodecIdFFmpeg::HEVC),
        CodecId::Vp9 => Ok(CodecIdFFmpeg::VP9),
        CodecId::Av1 => Ok(CodecIdFFmpeg::AV1),
        _ => Err(Error::unsupported_format(format!(
            "video codec {:?} not supported for encoding",
            codec
        ))),
    }
}

/**
    Convert our PixelFormat to FFmpeg's Pixel format.
*/
fn pixel_format_to_ffmpeg(format: PixelFormat) -> Result<ffmpeg_next::format::Pixel> {
    use ffmpeg_next::format::Pixel;

    match format {
        PixelFormat::Yuv420p => Ok(Pixel::YUV420P),
        PixelFormat::Nv12 => Ok(Pixel::NV12),
        PixelFormat::Bgra => Ok(Pixel::BGRA),
        PixelFormat::Rgba => Ok(Pixel::RGBA),
        PixelFormat::Rgb24 => Ok(Pixel::RGB24),
        PixelFormat::Bgr24 => Ok(Pixel::BGR24),
        PixelFormat::Yuv422p => Ok(Pixel::YUV422P),
        PixelFormat::Yuv444p => Ok(Pixel::YUV444P),
        PixelFormat::Yuv420p10 => Ok(Pixel::YUV420P10LE),
        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported",
            format
        ))),
    }
}

/**
    Convert FFmpeg's Pixel format to our PixelFormat.
*/
fn ffmpeg_pixel_format_to_ours(format: ffmpeg_next::format::Pixel) -> PixelFormat {
    use ffmpeg_next::format::Pixel;

    match format {
        Pixel::YUV420P => PixelFormat::Yuv420p,
        Pixel::NV12 => PixelFormat::Nv12,
        Pixel::BGRA => PixelFormat::Bgra,
        Pixel::RGBA => PixelFormat::Rgba,
        Pixel::RGB24 => PixelFormat::Rgb24,
        Pixel::BGR24 => PixelFormat::Bgr24,
        Pixel::YUV422P => PixelFormat::Yuv422p,
        Pixel::YUV444P => PixelFormat::Yuv444p,
        Pixel::YUV420P10LE | Pixel::YUV420P10BE => PixelFormat::Yuv420p10,
        _ => PixelFormat::Yuv420p, // fallback
    }
}

/**
    Copy data from our VideoFrame into an FFmpeg frame.
*/
fn copy_data_to_ffmpeg_frame(dst: &mut VideoFrameFFmpeg, src: &VideoFrame) -> Result<()> {
    match src.format {
        // Packed formats - single plane
        PixelFormat::Bgra | PixelFormat::Rgba => {
            let bytes_per_pixel = 4;
            let dst_stride = dst.stride(0);
            let dst_data = dst.data_mut(0);

            for y in 0..src.height as usize {
                let src_row_start = y * src.width as usize * bytes_per_pixel;
                let src_row_end = src_row_start + src.width as usize * bytes_per_pixel;
                let dst_row_start = y * dst_stride;

                dst_data[dst_row_start..dst_row_start + src.width as usize * bytes_per_pixel]
                    .copy_from_slice(&src.data[src_row_start..src_row_end]);
            }
            Ok(())
        }

        PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
            let bytes_per_pixel = 3;
            let dst_stride = dst.stride(0);
            let dst_data = dst.data_mut(0);

            for y in 0..src.height as usize {
                let src_row_start = y * src.width as usize * bytes_per_pixel;
                let src_row_end = src_row_start + src.width as usize * bytes_per_pixel;
                let dst_row_start = y * dst_stride;

                dst_data[dst_row_start..dst_row_start + src.width as usize * bytes_per_pixel]
                    .copy_from_slice(&src.data[src_row_start..src_row_end]);
            }
            Ok(())
        }

        // Planar YUV formats
        PixelFormat::Yuv420p
        | PixelFormat::Yuv422p
        | PixelFormat::Yuv444p
        | PixelFormat::Yuv420p10 => {
            let width = src.width as usize;
            let height = src.height as usize;

            let (uv_width, uv_height) = match src.format {
                PixelFormat::Yuv420p | PixelFormat::Yuv420p10 => (width / 2, height / 2),
                PixelFormat::Yuv422p => (width / 2, height),
                PixelFormat::Yuv444p => (width, height),
                _ => unreachable!(),
            };

            let bytes_per_sample = if src.format == PixelFormat::Yuv420p10 {
                2
            } else {
                1
            };

            let y_size = width * height * bytes_per_sample;
            let uv_size = uv_width * uv_height * bytes_per_sample;

            // Copy Y plane
            let y_stride = dst.stride(0);
            let y_data = dst.data_mut(0);
            for y in 0..height {
                let src_start = y * width * bytes_per_sample;
                let dst_start = y * y_stride;
                y_data[dst_start..dst_start + width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + width * bytes_per_sample]);
            }

            // Copy U plane
            let u_stride = dst.stride(1);
            let u_data = dst.data_mut(1);
            for y in 0..uv_height {
                let src_start = y_size + y * uv_width * bytes_per_sample;
                let dst_start = y * u_stride;
                u_data[dst_start..dst_start + uv_width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + uv_width * bytes_per_sample]);
            }

            // Copy V plane
            let v_stride = dst.stride(2);
            let v_data = dst.data_mut(2);
            for y in 0..uv_height {
                let src_start = y_size + uv_size + y * uv_width * bytes_per_sample;
                let dst_start = y * v_stride;
                v_data[dst_start..dst_start + uv_width * bytes_per_sample]
                    .copy_from_slice(&src.data[src_start..src_start + uv_width * bytes_per_sample]);
            }

            Ok(())
        }

        // NV12 - semi-planar
        PixelFormat::Nv12 => {
            let width = src.width as usize;
            let height = src.height as usize;
            let y_size = width * height;

            // Copy Y plane
            let y_stride = dst.stride(0);
            let y_data = dst.data_mut(0);
            for y in 0..height {
                let src_start = y * width;
                let dst_start = y * y_stride;
                y_data[dst_start..dst_start + width]
                    .copy_from_slice(&src.data[src_start..src_start + width]);
            }

            // Copy UV plane
            let uv_stride = dst.stride(1);
            let uv_data = dst.data_mut(1);
            let uv_height = height / 2;
            for y in 0..uv_height {
                let src_start = y_size + y * width;
                let dst_start = y * uv_stride;
                uv_data[dst_start..dst_start + width]
                    .copy_from_slice(&src.data[src_start..src_start + width]);
            }

            Ok(())
        }

        _ => Err(Error::unsupported_format(format!(
            "pixel format {:?} not supported for encoding input",
            src.format
        ))),
    }
}

impl std::fmt::Debug for VideoEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoEncoder")
            .field("width", &self.encoder.width())
            .field("height", &self.encoder.height())
            .field("time_base", &self.time_base)
            .finish_non_exhaustive()
    }
}
