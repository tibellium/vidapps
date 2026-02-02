/*!
    Audio frame transformation.
*/

use ffmpeg_next::{
    ChannelLayout as FFmpegChannelLayout,
    software::resampling::context::Context as ResamplerContext,
    util::frame::audio::Audio as AudioFrameFFmpeg,
};

use ffmpeg_types::{AudioFrame, ChannelLayout, Error, Rational, Result, SampleFormat};

/**
    Configuration for audio transformation.
*/
#[derive(Clone, Debug)]
pub struct AudioTransformConfig {
    /// Target sample rate in Hz.
    pub sample_rate: u32,
    /// Target channel layout.
    pub channels: ChannelLayout,
    /// Target sample format.
    pub format: SampleFormat,
}

impl AudioTransformConfig {
    /**
        Create a new audio transform configuration.
    */
    pub fn new(sample_rate: u32, channels: ChannelLayout, format: SampleFormat) -> Self {
        Self {
            sample_rate,
            channels,
            format,
        }
    }

    /**
        Create configuration for standard playback output.

        48kHz stereo F32 is the most common format for audio APIs.
    */
    pub fn playback() -> Self {
        Self::new(48000, ChannelLayout::Stereo, SampleFormat::F32)
    }

    /**
        Create configuration for CD-quality output.

        44.1kHz stereo S16.
    */
    pub fn cd_quality() -> Self {
        Self::new(44100, ChannelLayout::Stereo, SampleFormat::S16)
    }
}

/**
    Audio frame transformer.

    Converts audio frames between formats, handling:
    - Sample rate conversion
    - Channel layout conversion (mono ↔ stereo)
    - Sample format conversion (S16 ↔ F32, etc.)

    The resampler context is lazily initialized on first use and
    automatically reinitialized if the input format changes.

    Note: Audio resampling is stateful. Frames should be processed
    in order, and `flush()` should be called at end of stream.
*/
pub struct AudioTransform {
    config: AudioTransformConfig,
    /// Cached resampler context and the input format it was created for.
    resampler_state: Option<ResamplerState>,
}

struct ResamplerState {
    context: ResamplerContext,
    src_sample_rate: u32,
    src_channels: ChannelLayout,
    src_format: SampleFormat,
}

impl AudioTransform {
    /**
        Create a new audio transformer with the given configuration.
    */
    pub fn new(config: AudioTransformConfig) -> Self {
        Self {
            config,
            resampler_state: None,
        }
    }

    /**
        Get the target configuration.
    */
    pub fn config(&self) -> &AudioTransformConfig {
        &self.config
    }

    /**
        Transform an audio frame to the target format.

        The resampler is lazily initialized on first call and reused for
        subsequent frames with the same input format. If the input format
        changes, the resampler is automatically reinitialized.
    */
    pub fn transform(&mut self, frame: &AudioFrame) -> Result<AudioFrame> {
        // Validate input
        if frame.samples == 0 {
            return Err(Error::invalid_data("input frame has zero samples"));
        }

        if frame.data.is_empty() {
            return Err(Error::invalid_data("input frame has no data"));
        }

        // Check if we need to (re)initialize the resampler
        let needs_init = match &self.resampler_state {
            None => true,
            Some(state) => {
                state.src_sample_rate != frame.sample_rate
                    || state.src_channels != frame.channels
                    || state.src_format != frame.format
            }
        };

        if needs_init {
            self.init_resampler(frame.sample_rate, frame.channels, frame.format)?;
        }

        // Perform the transformation
        self.resample_frame(frame)
    }

    /**
        Flush any remaining samples from the resampler.

        Call this at end of stream to get any buffered samples.
        Returns None if no samples are buffered.
    */
    pub fn flush(&mut self) -> Result<Option<AudioFrame>> {
        let state = match &mut self.resampler_state {
            Some(s) => s,
            None => return Ok(None),
        };

        // Create an empty output frame to receive flushed samples
        let dst_sample = sample_format_to_ffmpeg(self.config.format)?;
        let dst_layout = channel_layout_to_ffmpeg(self.config.channels);

        // FFmpeg's delay() returns the number of samples buffered
        let delay = state.context.delay();
        let delay_samples = delay.map(|d| d.output as usize).unwrap_or(0);
        if delay_samples == 0 {
            return Ok(None);
        }

        let mut dst_frame = AudioFrameFFmpeg::new(dst_sample, delay_samples, dst_layout);
        dst_frame.set_rate(self.config.sample_rate);

        // Run with empty input to flush
        match state.context.flush(&mut dst_frame) {
            Ok(_) => {}
            Err(e) => {
                // Flush might fail if no data buffered
                if dst_frame.samples() == 0 {
                    return Ok(None);
                }
                return Err(Error::codec(format!("resampler flush failed: {}", e)));
            }
        }

        if dst_frame.samples() == 0 {
            return Ok(None);
        }

        // Copy output data
        let data =
            copy_audio_data_from_ffmpeg(&dst_frame, self.config.format, self.config.channels)?;
        let samples = dst_frame.samples();

        // Flushed frames don't have meaningful PTS
        Ok(Some(AudioFrame::new(
            data,
            samples,
            self.config.sample_rate,
            self.config.channels,
            self.config.format,
            None,
            Rational {
                num: 1,
                den: self.config.sample_rate as i32,
            },
        )))
    }

    /**
        Reset the resampler state.

        Call this after a seek to clear any buffered samples.
    */
    pub fn reset(&mut self) {
        self.resampler_state = None;
    }

    /**
        Initialize or reinitialize the resampler for the given input format.
    */
    fn init_resampler(
        &mut self,
        src_sample_rate: u32,
        src_channels: ChannelLayout,
        src_format: SampleFormat,
    ) -> Result<()> {
        let src_sample = sample_format_to_ffmpeg(src_format)?;
        let src_layout = channel_layout_to_ffmpeg(src_channels);

        let dst_sample = sample_format_to_ffmpeg(self.config.format)?;
        let dst_layout = channel_layout_to_ffmpeg(self.config.channels);

        let context = ResamplerContext::get(
            src_sample,
            src_layout,
            src_sample_rate,
            dst_sample,
            dst_layout,
            self.config.sample_rate,
        )
        .map_err(|e| Error::codec(format!("failed to create resampler: {}", e)))?;

        self.resampler_state = Some(ResamplerState {
            context,
            src_sample_rate,
            src_channels,
            src_format,
        });

        Ok(())
    }

    /**
        Resample a frame using the initialized resampler.
    */
    fn resample_frame(&mut self, frame: &AudioFrame) -> Result<AudioFrame> {
        let state = self
            .resampler_state
            .as_mut()
            .expect("resampler not initialized");

        // Create FFmpeg source frame
        let src_sample = sample_format_to_ffmpeg(frame.format)?;
        let src_layout = channel_layout_to_ffmpeg(frame.channels);
        let mut src_frame = AudioFrameFFmpeg::new(src_sample, frame.samples, src_layout);
        src_frame.set_rate(frame.sample_rate);

        // Copy input data into FFmpeg frame
        copy_audio_data_to_ffmpeg(&mut src_frame, frame)?;

        // Estimate output samples (might be different due to rate conversion)
        let output_samples = if frame.sample_rate == self.config.sample_rate {
            frame.samples
        } else {
            // Add some extra for resampling overhead
            ((frame.samples as u64 * self.config.sample_rate as u64) / frame.sample_rate as u64
                + 64) as usize
        };

        // Create destination frame
        let dst_sample = sample_format_to_ffmpeg(self.config.format)?;
        let dst_layout = channel_layout_to_ffmpeg(self.config.channels);
        let mut dst_frame = AudioFrameFFmpeg::new(dst_sample, output_samples, dst_layout);
        dst_frame.set_rate(self.config.sample_rate);

        // Run the resampler
        state
            .context
            .run(&src_frame, &mut dst_frame)
            .map_err(|e| Error::codec(format!("resampling failed: {}", e)))?;

        // Copy output data
        let actual_samples = dst_frame.samples();
        let data =
            copy_audio_data_from_ffmpeg(&dst_frame, self.config.format, self.config.channels)?;

        Ok(AudioFrame::new(
            data,
            actual_samples,
            self.config.sample_rate,
            self.config.channels,
            self.config.format,
            frame.pts,
            frame.time_base,
        ))
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
    Convert our ChannelLayout to FFmpeg's ChannelLayout.
*/
fn channel_layout_to_ffmpeg(layout: ChannelLayout) -> FFmpegChannelLayout {
    match layout {
        ChannelLayout::Mono => FFmpegChannelLayout::MONO,
        ChannelLayout::Stereo => FFmpegChannelLayout::STEREO,
        ChannelLayout::Surround5_1 => FFmpegChannelLayout::_5POINT1,
        ChannelLayout::Surround7_1 => FFmpegChannelLayout::_7POINT1,
        _ => FFmpegChannelLayout::STEREO, // Default fallback
    }
}

/**
    Copy data from our AudioFrame into an FFmpeg frame.
*/
fn copy_audio_data_to_ffmpeg(dst: &mut AudioFrameFFmpeg, src: &AudioFrame) -> Result<()> {
    let bytes_per_sample = src.format.bytes_per_sample();
    let total_bytes = src.samples * src.channels.channels() as usize * bytes_per_sample;

    // For packed format, just copy to plane 0
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

/**
    Copy data from an FFmpeg frame to a contiguous buffer.
*/
fn copy_audio_data_from_ffmpeg(
    frame: &AudioFrameFFmpeg,
    format: SampleFormat,
    channels: ChannelLayout,
) -> Result<Vec<u8>> {
    let samples = frame.samples();
    let bytes_per_sample = format.bytes_per_sample();
    let channel_count = channels.channels() as usize;
    let total_bytes = samples * channel_count * bytes_per_sample;

    // For packed format, just copy from plane 0
    let src_data = frame.data(0);
    Ok(src_data[..total_bytes].to_vec())
}

impl std::fmt::Debug for AudioTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioTransform")
            .field("config", &self.config)
            .field("initialized", &self.resampler_state.is_some())
            .finish()
    }
}
