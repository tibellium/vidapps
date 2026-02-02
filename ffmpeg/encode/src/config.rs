/*!
    Encoder configuration types.
*/

use ffmpeg_types::{ChannelLayout, CodecId, PixelFormat, Rational, SampleFormat};

/**
    Encoder speed preset.

    Slower presets produce better compression (smaller files at same quality)
    but take longer to encode.
*/
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncoderPreset {
    /// Fastest encoding, largest files.
    Ultrafast,
    /// Very fast encoding.
    Superfast,
    /// Fast encoding.
    Veryfast,
    /// Faster than default.
    Faster,
    /// Fast encoding, good for real-time.
    Fast,
    /// Default balance of speed and compression.
    #[default]
    Medium,
    /// Better compression, slower.
    Slow,
    /// Even better compression.
    Slower,
    /// Best compression, slowest.
    Veryslow,
}

impl EncoderPreset {
    /**
        Get the FFmpeg preset string.
    */
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ultrafast => "ultrafast",
            Self::Superfast => "superfast",
            Self::Veryfast => "veryfast",
            Self::Faster => "faster",
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::Slower => "slower",
            Self::Veryslow => "veryslow",
        }
    }
}

/**
    Rate control mode for video encoding.
*/
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateControl {
    /**
        Constant Rate Factor - target constant quality.
        Lower values = higher quality. Range 0-51, typical 18-28.
    */
    Crf(u8),
    /**
        Constant Bitrate in bits per second.
    */
    Cbr(u64),
    /**
        Variable Bitrate - target average bitrate in bits per second.
    */
    Vbr(u64),
}

impl Default for RateControl {
    fn default() -> Self {
        // CRF 23 is a reasonable default for H.264
        Self::Crf(23)
    }
}

/**
    Hardware encoder device.
*/
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwEncoder {
    /// VideoToolbox (macOS).
    VideoToolbox,
    /// NVIDIA NVENC.
    Nvenc,
    /// Intel Quick Sync Video.
    Qsv,
    /// VA-API (Linux).
    Vaapi,
}

/**
    Configuration for video encoding.
*/
#[derive(Clone, Debug)]
pub struct VideoEncoderConfig {
    /// Codec to use.
    pub codec: CodecId,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Frame rate.
    pub frame_rate: Rational,
    /// Expected input pixel format.
    pub pixel_format: PixelFormat,
    /// Rate control mode.
    pub rate_control: RateControl,
    /// Encoder speed preset.
    pub preset: EncoderPreset,
    /// Keyframe interval in frames (None = encoder default).
    pub keyframe_interval: Option<u32>,
    /// Prefer hardware encoding if available.
    pub prefer_hw: bool,
    /// Specific hardware encoder to use.
    pub hw_encoder: Option<HwEncoder>,
}

impl VideoEncoderConfig {
    /**
        Create a new video encoder configuration.
    */
    pub fn new(codec: CodecId, width: u32, height: u32, frame_rate: Rational) -> Self {
        Self {
            codec,
            width,
            height,
            frame_rate,
            pixel_format: PixelFormat::Yuv420p,
            rate_control: RateControl::default(),
            preset: EncoderPreset::default(),
            keyframe_interval: None,
            prefer_hw: false,
            hw_encoder: None,
        }
    }

    /**
        Create configuration for H.264 encoding.
    */
    pub fn h264(width: u32, height: u32, frame_rate: Rational) -> Self {
        Self::new(CodecId::H264, width, height, frame_rate)
    }

    /**
        Create configuration for H.265/HEVC encoding.
    */
    pub fn h265(width: u32, height: u32, frame_rate: Rational) -> Self {
        Self::new(CodecId::H265, width, height, frame_rate)
    }

    /**
        Set the rate control mode.
    */
    pub fn with_rate_control(mut self, rate_control: RateControl) -> Self {
        self.rate_control = rate_control;
        self
    }

    /**
        Set constant bitrate in bits per second.
    */
    pub fn with_bitrate(mut self, bitrate: u64) -> Self {
        self.rate_control = RateControl::Cbr(bitrate);
        self
    }

    /**
        Set CRF quality (0-51, lower is better, typical 18-28).
    */
    pub fn with_crf(mut self, crf: u8) -> Self {
        self.rate_control = RateControl::Crf(crf.min(51));
        self
    }

    /**
        Set the encoder preset.
    */
    pub fn with_preset(mut self, preset: EncoderPreset) -> Self {
        self.preset = preset;
        self
    }

    /**
        Set the keyframe interval in frames.
    */
    pub fn with_keyframe_interval(mut self, frames: u32) -> Self {
        self.keyframe_interval = Some(frames);
        self
    }

    /**
        Set the input pixel format.
    */
    pub fn with_pixel_format(mut self, format: PixelFormat) -> Self {
        self.pixel_format = format;
        self
    }

    /**
        Enable hardware encoding with auto-detection.
    */
    pub fn with_hw_accel(mut self) -> Self {
        self.prefer_hw = true;
        self
    }

    /**
        Enable specific hardware encoder.
    */
    pub fn with_hw_encoder(mut self, encoder: HwEncoder) -> Self {
        self.prefer_hw = true;
        self.hw_encoder = Some(encoder);
        self
    }
}

/**
    Configuration for audio encoding.
*/
#[derive(Clone, Debug)]
pub struct AudioEncoderConfig {
    /// Codec to use.
    pub codec: CodecId,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel layout.
    pub channels: ChannelLayout,
    /// Expected input sample format.
    pub sample_format: SampleFormat,
    /// Target bitrate in bits per second (None = codec default).
    pub bitrate: Option<u64>,
}

impl AudioEncoderConfig {
    /**
        Create a new audio encoder configuration.
    */
    pub fn new(codec: CodecId, sample_rate: u32, channels: ChannelLayout) -> Self {
        Self {
            codec,
            sample_rate,
            channels,
            sample_format: SampleFormat::F32,
            bitrate: None,
        }
    }

    /**
        Create configuration for AAC encoding.
    */
    pub fn aac(sample_rate: u32, channels: ChannelLayout) -> Self {
        Self::new(CodecId::Aac, sample_rate, channels)
    }

    /**
        Create configuration for Opus encoding.
    */
    pub fn opus(sample_rate: u32, channels: ChannelLayout) -> Self {
        Self::new(CodecId::Opus, sample_rate, channels)
    }

    /**
        Set the target bitrate in bits per second.
    */
    pub fn with_bitrate(mut self, bitrate: u64) -> Self {
        self.bitrate = Some(bitrate);
        self
    }

    /**
        Set the input sample format.
    */
    pub fn with_sample_format(mut self, format: SampleFormat) -> Self {
        self.sample_format = format;
        self
    }
}
