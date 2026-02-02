/*!
    Audio decoder implementation.
*/

use ffmpeg_next::{
    codec::{self, decoder::Audio as AudioDecoderFFmpeg},
    ffi,
    packet::Mut as PacketMut,
    util::frame::audio::Audio as AudioFrameFFmpeg,
};

use ffmpeg_source::CodecConfig;
use ffmpeg_types::{AudioFrame, ChannelLayout, Error, Packet, Pts, Rational, Result, SampleFormat};

use crate::config::AudioDecoderConfig;

/**
    Audio decoder.

    Decodes audio packets into frames.
*/
pub struct AudioDecoder {
    decoder: AudioDecoderFFmpeg,
    time_base: Rational,
}

impl AudioDecoder {
    /**
        Create a new audio decoder from codec configuration.

        # Arguments

        * `codec_config` - Codec configuration from the source
        * `time_base` - Time base for the audio stream
        * `_config` - Decoder configuration (reserved for future use)
    */
    pub fn new(
        codec_config: CodecConfig,
        time_base: Rational,
        _config: AudioDecoderConfig,
    ) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        let parameters = codec_config.into_parameters();

        let decoder_ctx = codec::context::Context::from_parameters(parameters)
            .map_err(|e| Error::codec(e.to_string()))?;

        let decoder = decoder_ctx
            .decoder()
            .audio()
            .map_err(|e| Error::codec(e.to_string()))?;

        Ok(Self { decoder, time_base })
    }

    /**
        Get the time base for this decoder.
    */
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /**
        Get the sample rate of the decoded audio.
    */
    pub fn sample_rate(&self) -> u32 {
        self.decoder.rate()
    }

    /**
        Get the number of channels.
    */
    pub fn channels(&self) -> u16 {
        self.decoder.channels() as u16
    }

    /**
        Decode a packet, returning decoded frames.

        May return zero, one, or multiple frames depending on codec.
    */
    pub fn decode(&mut self, packet: &Packet) -> Result<Vec<AudioFrame>> {
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

        Call this at end of stream to retrieve any buffered frames.
    */
    pub fn flush(&mut self) -> Result<Vec<AudioFrame>> {
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

        Clears internal buffers. Call this after seeking.
    */
    pub fn reset(&mut self) {
        self.decoder.flush();
    }

    /**
        Receive all available frames from the decoder.
    */
    fn receive_frames(&mut self) -> Result<Vec<AudioFrame>> {
        let mut frames = Vec::new();
        let mut decoded_frame = AudioFrameFFmpeg::empty();

        loop {
            match self.decoder.receive_frame(&mut decoded_frame) {
                Ok(()) => match self.convert_frame(&decoded_frame) {
                    Ok(frame) => frames.push(frame),
                    Err(e) => {
                        eprintln!("[audio_decode] frame conversion error: {}", e);
                    }
                },
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => {
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

        Ok(frames)
    }

    /**
        Convert an FFmpeg audio frame to our AudioFrame type.
    */
    fn convert_frame(&self, frame: &AudioFrameFFmpeg) -> Result<AudioFrame> {
        let samples = frame.samples();
        let sample_rate = frame.rate();
        let channel_count = frame.channels() as u16;

        if samples == 0 {
            return Err(Error::invalid_data("audio frame has zero samples"));
        }

        // Check that the frame actually has data planes allocated
        if frame.planes() == 0 {
            return Err(Error::invalid_data(
                "audio frame has no data planes (linesize is 0)",
            ));
        }

        // Get format
        let ffmpeg_format = frame.format();
        let format = sample_format_from_ffmpeg(ffmpeg_format).ok_or_else(|| {
            Error::unsupported_format(format!("unsupported sample format: {:?}", ffmpeg_format))
        })?;

        // Determine channel layout from actual channel count
        let channels = ChannelLayout::from_count(channel_count);

        // Get PTS
        let pts = frame.pts().map(Pts);

        // Copy frame data
        let data = copy_audio_data(frame, format, samples, channel_count)?;

        Ok(AudioFrame::new(
            data,
            samples,
            sample_rate,
            channels,
            format,
            pts,
            self.time_base,
        ))
    }
}

/**
    Copy audio data from FFmpeg frame.

    Handles both planar and packed formats. For planar audio, FFmpeg stores
    each channel in a separate plane, and we need to interleave them.

    Note: In FFmpeg planar audio, linesize[0] contains the size of EACH plane
    (they're all the same), while linesize[1..] may be 0. We access plane data
    directly via the data pointers rather than relying on linesize for each plane.
*/
fn copy_audio_data(
    frame: &AudioFrameFFmpeg,
    format: SampleFormat,
    samples: usize,
    channels: u16,
) -> Result<Vec<u8>> {
    let bytes_per_sample = format.bytes_per_sample();
    let total_bytes = samples * channels as usize * bytes_per_sample;
    let expected_plane_bytes = samples * bytes_per_sample;

    // For planar audio, check if we have the right number of planes
    let is_planar = frame.is_planar();
    let planes = frame.planes();

    if is_planar && planes >= channels as usize {
        // Planar format - interleave the channels
        // In planar format, each channel is in its own plane
        let mut output = vec![0u8; total_bytes];

        // Get linesize[0] which applies to all planes in planar audio
        let plane0_data = frame.data(0);
        let plane_size = plane0_data.len();

        if plane_size < expected_plane_bytes {
            return Err(Error::invalid_data(format!(
                "audio plane size {} is less than expected {} bytes for {} samples",
                plane_size, expected_plane_bytes, samples
            )));
        }

        for ch in 0..channels as usize {
            // Access each plane's data directly
            // Note: We use unsafe here because frame.data(ch) for ch > 0 may
            // return based on linesize[ch] which could be 0 for planar audio.
            // Instead, we know all planes have the same size as plane 0.
            let plane_data = unsafe {
                let ptr = (*frame.as_ptr()).data[ch];
                std::slice::from_raw_parts(ptr, plane_size)
            };

            for s in 0..samples {
                let src_offset = s * bytes_per_sample;
                let dst_offset = (s * channels as usize + ch) * bytes_per_sample;
                output[dst_offset..dst_offset + bytes_per_sample]
                    .copy_from_slice(&plane_data[src_offset..src_offset + bytes_per_sample]);
            }
        }

        Ok(output)
    } else {
        // Packed/interleaved format - all data in plane 0
        let plane0_data = frame.data(0);
        if plane0_data.len() < total_bytes {
            return Err(Error::invalid_data(format!(
                "packed audio data has {} bytes, expected at least {}",
                plane0_data.len(),
                total_bytes
            )));
        }
        Ok(plane0_data[..total_bytes].to_vec())
    }
}

/**
    Convert FFmpeg sample format to our SampleFormat.
*/
fn sample_format_from_ffmpeg(format: ffmpeg_next::format::Sample) -> Option<SampleFormat> {
    use ffmpeg_next::format::Sample;

    match format {
        Sample::F32(_) => Some(SampleFormat::F32),
        Sample::F64(_) => Some(SampleFormat::F64),
        Sample::I16(_) => Some(SampleFormat::S16),
        Sample::I32(_) => Some(SampleFormat::S32),
        Sample::U8(_) => Some(SampleFormat::U8),
        _ => None,
    }
}

impl std::fmt::Debug for AudioDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDecoder")
            .field("time_base", &self.time_base)
            .field("sample_rate", &self.decoder.rate())
            .field("channels", &self.decoder.channels())
            .finish_non_exhaustive()
    }
}
