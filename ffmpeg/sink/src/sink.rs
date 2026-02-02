/*!
    Media sink implementation.
*/

use std::path::Path;

use ffmpeg_next::{
    Dictionary, Rational as FFmpegRational, codec::Parameters, ffi,
    format::context::Output as OutputContext, packet::Mut as PacketMut,
};

use ffmpeg_types::{
    AudioStreamInfo, CodecId, Error, Packet, PixelFormat, Rational, Result, SampleFormat,
    StreamType, VideoStreamInfo,
};

use crate::config::{ContainerFormat, SinkConfig};

/**
    Media sink for writing to container files.

    Takes encoded packets and writes them into a container format (MP4, MKV, etc.).
*/
pub struct Sink {
    output: OutputContext,
    video_stream_index: Option<usize>,
    audio_stream_index: Option<usize>,
    video_time_base: Option<Rational>,
    audio_time_base: Option<Rational>,
    header_written: bool,
}

impl Sink {
    /**
        Create a new sink that writes to a file.
    */
    pub fn file<P: AsRef<Path>>(path: P, config: SinkConfig) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        let path = path.as_ref();

        // Create output context
        let mut output = ffmpeg_next::format::output(path)
            .map_err(|e| Error::codec(format!("failed to create output: {}", e)))?;

        let mut video_stream_index = None;
        let mut audio_stream_index = None;
        let mut video_time_base = None;
        let mut audio_time_base = None;

        // Add video stream if configured
        if let Some(ref video_info) = config.video {
            let codec_id = codec_id_to_ffmpeg_video(video_info.codec_id)?;
            let codec = ffmpeg_next::encoder::find(codec_id).ok_or_else(|| {
                Error::unsupported_format(format!(
                    "video codec {:?} not found",
                    video_info.codec_id
                ))
            })?;

            let mut stream = output
                .add_stream(codec)
                .map_err(|e| Error::codec(format!("failed to add video stream: {}", e)))?;

            // Set stream parameters
            let params = stream.parameters();
            set_video_parameters(&params, video_info)?;

            // Set time base
            let tb = FFmpegRational::new(video_info.time_base.num, video_info.time_base.den);
            stream.set_time_base(tb);

            video_stream_index = Some(stream.index());
            video_time_base = Some(video_info.time_base);
        }

        // Add audio stream if configured
        if let Some(ref audio_info) = config.audio {
            let codec_id = codec_id_to_ffmpeg_audio(audio_info.codec_id)?;
            let codec = ffmpeg_next::encoder::find(codec_id).ok_or_else(|| {
                Error::unsupported_format(format!(
                    "audio codec {:?} not found",
                    audio_info.codec_id
                ))
            })?;

            let mut stream = output
                .add_stream(codec)
                .map_err(|e| Error::codec(format!("failed to add audio stream: {}", e)))?;

            // Set stream parameters
            let params = stream.parameters();
            set_audio_parameters(&params, audio_info)?;

            // Set time base
            let tb = FFmpegRational::new(audio_info.time_base.num, audio_info.time_base.den);
            stream.set_time_base(tb);

            audio_stream_index = Some(stream.index());
            audio_time_base = Some(audio_info.time_base);
        }

        // Set format options
        let mut opts = Dictionary::new();

        // Enable fast start for MP4
        if matches!(config.format, ContainerFormat::Mp4) && config.fast_start {
            opts.set("movflags", "+faststart");
        }

        // HLS options
        if let ContainerFormat::Hls { segment_duration } = &config.format {
            opts.set("hls_time", &segment_duration.as_secs().to_string());
            opts.set("hls_list_size", "0"); // Keep all segments in playlist
            opts.set("hls_segment_type", "mpegts");
        }

        // Write header
        output
            .write_header_with(opts)
            .map_err(|e| Error::codec(format!("failed to write header: {}", e)))?;

        Ok(Self {
            output,
            video_stream_index,
            audio_stream_index,
            video_time_base,
            audio_time_base,
            header_written: true,
        })
    }

    /**
        Write a packet to the sink.

        Packets are automatically routed to the correct stream based on their type.
    */
    pub fn write(&mut self, packet: &Packet) -> Result<()> {
        if !self.header_written {
            return Err(Error::invalid_data("header not written"));
        }

        // Determine stream index and time base
        let (stream_index, stream_time_base) = match packet.stream_type {
            StreamType::Video => {
                let idx = self
                    .video_stream_index
                    .ok_or_else(|| Error::invalid_data("no video stream configured"))?;
                let tb = self.video_time_base.unwrap();
                (idx, tb)
            }
            StreamType::Audio => {
                let idx = self
                    .audio_stream_index
                    .ok_or_else(|| Error::invalid_data("no audio stream configured"))?;
                let tb = self.audio_time_base.unwrap();
                (idx, tb)
            }
        };

        // Create FFmpeg packet
        let mut ffmpeg_pkt = if packet.data.is_empty() {
            ffmpeg_next::Packet::empty()
        } else {
            ffmpeg_next::Packet::copy(&packet.data)
        };

        // Set stream index
        ffmpeg_pkt.set_stream(stream_index);

        // Set timing
        unsafe {
            let pkt_ptr = ffmpeg_pkt.as_mut_ptr();
            if let Some(pts) = packet.pts {
                (*pkt_ptr).pts = rescale_ts(pts.0, packet.time_base, stream_time_base);
            }
            if let Some(dts) = packet.dts {
                (*pkt_ptr).dts = rescale_ts(dts.0, packet.time_base, stream_time_base);
            }
            (*pkt_ptr).duration = rescale_ts(packet.duration.0, packet.time_base, stream_time_base);
        }

        // Set keyframe flag
        if packet.is_keyframe {
            ffmpeg_pkt.set_flags(ffmpeg_next::packet::Flags::KEY);
        }

        // Write packet
        ffmpeg_pkt
            .write_interleaved(&mut self.output)
            .map_err(|e| Error::codec(format!("failed to write packet: {}", e)))?;

        Ok(())
    }

    /**
        Finish writing and close the sink.

        This writes any trailing metadata (duration, seeking index) and
        finalizes the container. The file may be corrupt if this is not called.
    */
    pub fn finish(mut self) -> Result<()> {
        self.output
            .write_trailer()
            .map_err(|e| Error::codec(format!("failed to write trailer: {}", e)))?;

        Ok(())
    }
}

/**
    Rescale a timestamp from one time base to another.
*/
fn rescale_ts(ts: i64, from: Rational, to: Rational) -> i64 {
    if from.num == to.num && from.den == to.den {
        return ts;
    }

    // ts * from.num / from.den * to.den / to.num
    // = ts * from.num * to.den / (from.den * to.num)
    let num = ts as i128 * from.num as i128 * to.den as i128;
    let den = from.den as i128 * to.num as i128;
    (num / den) as i64
}

/**
    Convert our video CodecId to FFmpeg's codec ID.
*/
fn codec_id_to_ffmpeg_video(codec: CodecId) -> Result<ffmpeg_next::codec::Id> {
    use ffmpeg_next::codec::Id;

    match codec {
        CodecId::H264 => Ok(Id::H264),
        CodecId::H265 => Ok(Id::HEVC),
        CodecId::Vp9 => Ok(Id::VP9),
        CodecId::Av1 => Ok(Id::AV1),
        _ => Err(Error::unsupported_format(format!(
            "video codec {:?} not supported for muxing",
            codec
        ))),
    }
}

/**
    Convert our audio CodecId to FFmpeg's codec ID.
*/
fn codec_id_to_ffmpeg_audio(codec: CodecId) -> Result<ffmpeg_next::codec::Id> {
    use ffmpeg_next::codec::Id;

    match codec {
        CodecId::Aac => Ok(Id::AAC),
        CodecId::Opus => Ok(Id::OPUS),
        CodecId::Mp3 => Ok(Id::MP3),
        _ => Err(Error::unsupported_format(format!(
            "audio codec {:?} not supported for muxing",
            codec
        ))),
    }
}

/**
    Set video stream parameters.
*/
fn set_video_parameters(params: &Parameters, info: &VideoStreamInfo) -> Result<()> {
    unsafe {
        let ptr = params.as_ptr() as *mut ffi::AVCodecParameters;

        (*ptr).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*ptr).codec_id = match info.codec_id {
            CodecId::H264 => ffi::AVCodecID::AV_CODEC_ID_H264,
            CodecId::H265 => ffi::AVCodecID::AV_CODEC_ID_HEVC,
            CodecId::Vp9 => ffi::AVCodecID::AV_CODEC_ID_VP9,
            CodecId::Av1 => ffi::AVCodecID::AV_CODEC_ID_AV1,
            _ => {
                return Err(Error::unsupported_format(format!(
                    "video codec {:?} not supported",
                    info.codec_id
                )));
            }
        };

        (*ptr).width = info.width as i32;
        (*ptr).height = info.height as i32;

        (*ptr).format = match info.pixel_format {
            PixelFormat::Yuv420p => ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32,
            PixelFormat::Nv12 => ffi::AVPixelFormat::AV_PIX_FMT_NV12 as i32,
            PixelFormat::Bgra => ffi::AVPixelFormat::AV_PIX_FMT_BGRA as i32,
            PixelFormat::Rgba => ffi::AVPixelFormat::AV_PIX_FMT_RGBA as i32,
            _ => ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as i32,
        };

        // Set extradata if present (critical for remuxing - contains SPS/PPS for H.264, etc.)
        if let Some(ref extradata) = info.extradata {
            if !extradata.is_empty() {
                // Allocate buffer with padding (FFmpeg requires AV_INPUT_BUFFER_PADDING_SIZE)
                let alloc_size = extradata.len() + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
                let buf = ffi::av_mallocz(alloc_size) as *mut u8;
                if !buf.is_null() {
                    std::ptr::copy_nonoverlapping(extradata.as_ptr(), buf, extradata.len());
                    (*ptr).extradata = buf;
                    (*ptr).extradata_size = extradata.len() as i32;
                }
            }
        }

        // Set bitrate if present
        if let Some(bitrate) = info.bitrate {
            (*ptr).bit_rate = bitrate as i64;
        }

        // Set profile if present
        if let Some(profile) = info.profile {
            (*ptr).profile = profile;
        }

        // Set level if present
        if let Some(level) = info.level {
            (*ptr).level = level;
        }
    }

    Ok(())
}

/**
    Set audio stream parameters.
*/
fn set_audio_parameters(params: &Parameters, info: &AudioStreamInfo) -> Result<()> {
    unsafe {
        let ptr = params.as_ptr() as *mut ffi::AVCodecParameters;

        (*ptr).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_AUDIO;
        (*ptr).codec_id = match info.codec_id {
            CodecId::Aac => ffi::AVCodecID::AV_CODEC_ID_AAC,
            CodecId::Opus => ffi::AVCodecID::AV_CODEC_ID_OPUS,
            CodecId::Mp3 => ffi::AVCodecID::AV_CODEC_ID_MP3,
            _ => {
                return Err(Error::unsupported_format(format!(
                    "audio codec {:?} not supported",
                    info.codec_id
                )));
            }
        };

        (*ptr).sample_rate = info.sample_rate as i32;

        // Set channel layout
        let channels = info.channels.channels();
        (*ptr).ch_layout.nb_channels = channels as i32;

        (*ptr).format = match info.sample_format {
            SampleFormat::F32 => ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT as i32,
            SampleFormat::F64 => ffi::AVSampleFormat::AV_SAMPLE_FMT_DBL as i32,
            SampleFormat::S16 => ffi::AVSampleFormat::AV_SAMPLE_FMT_S16 as i32,
            SampleFormat::S32 => ffi::AVSampleFormat::AV_SAMPLE_FMT_S32 as i32,
            SampleFormat::U8 => ffi::AVSampleFormat::AV_SAMPLE_FMT_U8 as i32,
            _ => ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT as i32,
        };

        // Set extradata if present (critical for remuxing - contains AudioSpecificConfig for AAC, etc.)
        if let Some(ref extradata) = info.extradata {
            if !extradata.is_empty() {
                // Allocate buffer with padding (FFmpeg requires AV_INPUT_BUFFER_PADDING_SIZE)
                let alloc_size = extradata.len() + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
                let buf = ffi::av_mallocz(alloc_size) as *mut u8;
                if !buf.is_null() {
                    std::ptr::copy_nonoverlapping(extradata.as_ptr(), buf, extradata.len());
                    (*ptr).extradata = buf;
                    (*ptr).extradata_size = extradata.len() as i32;
                }
            }
        }

        // Set bitrate if present
        if let Some(bitrate) = info.bitrate {
            (*ptr).bit_rate = bitrate as i64;
        }

        // Set profile if present
        if let Some(profile) = info.profile {
            (*ptr).profile = profile;
        }
    }

    Ok(())
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sink")
            .field("video_stream", &self.video_stream_index)
            .field("audio_stream", &self.audio_stream_index)
            .field("header_written", &self.header_written)
            .finish_non_exhaustive()
    }
}
