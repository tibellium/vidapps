/*!
    Audio encoder implementation.
*/

use ffmpeg_next::{
    ChannelLayout as FFmpegChannelLayout,
    codec::{self, Id as CodecIdFFmpeg, encoder::Audio as AudioEncoderFFmpeg},
    ffi,
    util::frame::audio::Audio as AudioFrameFFmpeg,
};

use ffmpeg_types::{
    AudioFrame, AudioStreamInfo, ChannelLayout, CodecId, Error, MediaDuration, Packet, Pts,
    Rational, Result, SampleFormat, StreamType,
};

use crate::config::AudioEncoderConfig;

/**
    Audio encoder.

    Encodes raw audio frames into compressed packets.
*/
pub struct AudioEncoder {
    encoder: AudioEncoderFFmpeg,
    time_base: Rational,
    sample_count: i64,
}

impl AudioEncoder {
    /**
        Create a new audio encoder with the given configuration.
    */
    pub fn new(config: AudioEncoderConfig) -> Result<Self> {
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
            .audio()
            .map_err(|e| Error::codec(e.to_string()))?;

        // Set sample format
        let sample_format = sample_format_to_ffmpeg(config.sample_format)?;
        encoder.set_format(sample_format);

        // Set sample rate
        encoder.set_rate(config.sample_rate as i32);

        // Set channel layout
        let channel_layout = channel_layout_to_ffmpeg(config.channels);
        encoder.set_channel_layout(channel_layout);

        // Set time base (1/sample_rate is standard for audio)
        let time_base = ffmpeg_next::Rational::new(1, config.sample_rate as i32);
        encoder.set_time_base(time_base);

        // Set bitrate if specified
        if let Some(bitrate) = config.bitrate {
            encoder.set_bit_rate(bitrate as usize);
        }

        // Open the encoder
        let encoder = encoder
            .open()
            .map_err(|e| Error::codec(format!("failed to open encoder: {}", e)))?;

        let time_base = Rational {
            num: 1,
            den: config.sample_rate as i32,
        };

        Ok(Self {
            encoder,
            time_base,
            sample_count: 0,
        })
    }

    /**
        Get the time base for encoded packets.
    */
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /**
        Get the frame size expected by the encoder.

        Some codecs require a specific number of samples per frame.
        Returns None if the codec accepts variable frame sizes.
    */
    pub fn frame_size(&self) -> Option<usize> {
        let size = self.encoder.frame_size() as usize;
        if size == 0 { None } else { Some(size) }
    }

    /**
        Get stream info for the muxer.
    */
    pub fn stream_info(&self) -> AudioStreamInfo {
        // Extract extradata, bitrate, and profile from encoder context
        let (extradata, bitrate, profile) = unsafe {
            let ctx_ptr = self.encoder.as_ptr();
            let extradata = if (*ctx_ptr).extradata_size > 0 && !(*ctx_ptr).extradata.is_null() {
                let slice = std::slice::from_raw_parts(
                    (*ctx_ptr).extradata,
                    (*ctx_ptr).extradata_size as usize,
                );
                Some(slice.to_vec())
            } else {
                None
            };
            let bitrate = if (*ctx_ptr).bit_rate > 0 {
                Some((*ctx_ptr).bit_rate as u64)
            } else {
                None
            };
            let profile = if (*ctx_ptr).profile != ffi::FF_PROFILE_UNKNOWN {
                Some((*ctx_ptr).profile)
            } else {
                None
            };
            (extradata, bitrate, profile)
        };

        AudioStreamInfo {
            sample_rate: self.encoder.rate(),
            channels: ffmpeg_channel_layout_to_ours(self.encoder.channel_layout()),
            sample_format: ffmpeg_sample_format_to_ours(self.encoder.format()),
            time_base: self.time_base,
            duration: None,
            codec_id: CodecId::Aac, // TODO: track actual codec
            extradata,
            bitrate,
            profile,
        }
    }

    /**
        Encode an audio frame, returning encoded packets.

        May return zero, one, or multiple packets depending on encoder buffering.
    */
    pub fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<Packet>> {
        // Create FFmpeg frame
        let sample_format = sample_format_to_ffmpeg(frame.format)?;
        let channel_layout = channel_layout_to_ffmpeg(frame.channels);
        let mut ffmpeg_frame = AudioFrameFFmpeg::new(sample_format, frame.samples, channel_layout);
        ffmpeg_frame.set_rate(frame.sample_rate);

        // Copy data into FFmpeg frame
        copy_data_to_ffmpeg_frame(&mut ffmpeg_frame, frame)?;

        // Set PTS
        let pts = if let Some(p) = frame.pts {
            p.0
        } else {
            self.sample_count
        };
        ffmpeg_frame.set_pts(Some(pts));
        self.sample_count += frame.samples as i64;

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
            stream_type: StreamType::Audio,
        })
    }
}

/**
    Convert our CodecId to FFmpeg's codec ID.
*/
fn codec_id_to_ffmpeg(codec: CodecId) -> Result<CodecIdFFmpeg> {
    match codec {
        CodecId::Aac => Ok(CodecIdFFmpeg::AAC),
        CodecId::Opus => Ok(CodecIdFFmpeg::OPUS),
        CodecId::Mp3 => Ok(CodecIdFFmpeg::MP3),
        _ => Err(Error::unsupported_format(format!(
            "audio codec {:?} not supported for encoding",
            codec
        ))),
    }
}

/**
    Convert our SampleFormat to FFmpeg's Sample format.
*/
fn sample_format_to_ffmpeg(format: SampleFormat) -> Result<ffmpeg_next::format::Sample> {
    use ffmpeg_next::format::Sample;
    use ffmpeg_next::format::sample::Type;

    match format {
        SampleFormat::F32 => Ok(Sample::F32(Type::Packed)),
        SampleFormat::F64 => Ok(Sample::F64(Type::Packed)),
        SampleFormat::S16 => Ok(Sample::I16(Type::Packed)),
        SampleFormat::S32 => Ok(Sample::I32(Type::Packed)),
        SampleFormat::U8 => Ok(Sample::U8(Type::Packed)),
        _ => Err(Error::unsupported_format(format!(
            "sample format {:?} not supported",
            format
        ))),
    }
}

/**
    Convert FFmpeg's Sample format to our SampleFormat.
*/
fn ffmpeg_sample_format_to_ours(format: ffmpeg_next::format::Sample) -> SampleFormat {
    use ffmpeg_next::format::Sample;

    match format {
        Sample::F32(_) => SampleFormat::F32,
        Sample::F64(_) => SampleFormat::F64,
        Sample::I16(_) => SampleFormat::S16,
        Sample::I32(_) => SampleFormat::S32,
        Sample::U8(_) => SampleFormat::U8,
        _ => SampleFormat::F32, // fallback
    }
}

/**
    Convert our ChannelLayout to FFmpeg's ChannelLayout.
*/
fn channel_layout_to_ffmpeg(layout: ChannelLayout) -> FFmpegChannelLayout {
    match layout {
        ChannelLayout::Mono => FFmpegChannelLayout::MONO,
        ChannelLayout::Stereo => FFmpegChannelLayout::STEREO,
        _ => FFmpegChannelLayout::STEREO, // fallback
    }
}

/**
    Convert FFmpeg's ChannelLayout to our ChannelLayout.
*/
fn ffmpeg_channel_layout_to_ours(layout: FFmpegChannelLayout) -> ChannelLayout {
    if layout == FFmpegChannelLayout::MONO {
        ChannelLayout::Mono
    } else {
        ChannelLayout::Stereo
    }
}

/**
    Copy data from our AudioFrame into an FFmpeg frame.
*/
fn copy_data_to_ffmpeg_frame(dst: &mut AudioFrameFFmpeg, src: &AudioFrame) -> Result<()> {
    let bytes_per_sample = src.format.bytes_per_sample();
    let channel_count = src.channels.channels() as usize;
    let total_bytes = src.samples * channel_count * bytes_per_sample;

    // For packed format, copy to plane 0
    let dst_data = dst.data_mut(0);
    if dst_data.len() < total_bytes {
        return Err(Error::invalid_data(format!(
            "destination buffer too small: {} < {}",
            dst_data.len(),
            total_bytes
        )));
    }

    dst_data[..total_bytes].copy_from_slice(&src.data[..total_bytes]);
    Ok(())
}

impl std::fmt::Debug for AudioEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEncoder")
            .field("sample_rate", &self.encoder.rate())
            .field("time_base", &self.time_base)
            .finish_non_exhaustive()
    }
}
