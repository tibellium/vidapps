/*!
    Stream information types.
*/

use std::time::Duration;

use crate::{ChannelLayout, CodecId, PixelFormat, Rational, SampleFormat};

/**
    Information about a video stream.
*/
#[derive(Clone, Debug)]
pub struct VideoStreamInfo {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel format.
    pub pixel_format: PixelFormat,
    /// Frame rate (may be approximate or unavailable).
    pub frame_rate: Option<Rational>,
    /// Time base for timestamps.
    pub time_base: Rational,
    /// Total duration (may be unavailable for some streams).
    pub duration: Option<Duration>,
    /// Codec used.
    pub codec_id: CodecId,
    /// Codec extradata (SPS/PPS for H.264, VPS/SPS/PPS for H.265, etc.).
    pub extradata: Option<Vec<u8>>,
    /// Bitrate in bits per second (if known).
    pub bitrate: Option<u64>,
    /// Codec profile (codec-specific value, e.g., H.264 High Profile).
    pub profile: Option<i32>,
    /// Codec level (codec-specific value, e.g., H.264 Level 4.1).
    pub level: Option<i32>,
}

impl VideoStreamInfo {
    /**
        Returns the aspect ratio as a float.
    */
    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }

    /**
        Returns the frame rate as fps, if available.
    */
    pub fn fps(&self) -> Option<f64> {
        self.frame_rate.map(|r| r.to_f64())
    }
}

/**
    Information about an audio stream.
*/
#[derive(Clone, Debug)]
pub struct AudioStreamInfo {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel layout.
    pub channels: ChannelLayout,
    /// Sample format.
    pub sample_format: SampleFormat,
    /// Time base for timestamps.
    pub time_base: Rational,
    /// Total duration (may be unavailable for some streams).
    pub duration: Option<Duration>,
    /// Codec used.
    pub codec_id: CodecId,
    /// Codec extradata (AudioSpecificConfig for AAC, etc.).
    pub extradata: Option<Vec<u8>>,
    /// Bitrate in bits per second (if known).
    pub bitrate: Option<u64>,
    /// Codec profile (codec-specific value, e.g., AAC LC, HE-AAC).
    pub profile: Option<i32>,
}

impl AudioStreamInfo {
    /**
        Returns the number of channels.
    */
    pub fn channel_count(&self) -> u16 {
        self.channels.channels()
    }

    /**
        Returns the number of bytes per sample per channel.
    */
    pub fn bytes_per_sample(&self) -> usize {
        self.sample_format.bytes_per_sample()
    }
}

/**
    Combined information about a media source.
*/
#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    /// Total duration of the media (may be unavailable).
    pub duration: Option<Duration>,
    /// Video stream information (if video is present).
    pub video: Option<VideoStreamInfo>,
    /// Audio stream information (if audio is present).
    pub audio: Option<AudioStreamInfo>,
}

impl MediaInfo {
    /**
        Returns true if this media has video.
    */
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }

    /**
        Returns true if this media has audio.
    */
    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_stream_info_aspect_ratio() {
        let info = VideoStreamInfo {
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: Some(Rational::new(24000, 1001)),
            time_base: Rational::new(1, 90000),
            duration: Some(Duration::from_secs(120)),
            codec_id: CodecId::H264,
            extradata: None,
            bitrate: None,
            profile: None,
            level: None,
        };

        let aspect = info.aspect_ratio();
        assert!((aspect - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn video_stream_info_fps() {
        let info = VideoStreamInfo {
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: Some(Rational::new(30, 1)),
            time_base: Rational::new(1, 90000),
            duration: None,
            codec_id: CodecId::H264,
            extradata: None,
            bitrate: None,
            profile: None,
            level: None,
        };

        assert_eq!(info.fps(), Some(30.0));
    }

    #[test]
    fn audio_stream_info_channel_count() {
        let info = AudioStreamInfo {
            sample_rate: 48000,
            channels: ChannelLayout::Stereo,
            sample_format: SampleFormat::F32,
            time_base: Rational::new(1, 48000),
            duration: None,
            codec_id: CodecId::Aac,
            extradata: None,
            bitrate: None,
            profile: None,
        };

        assert_eq!(info.channel_count(), 2);
    }

    #[test]
    fn media_info_has_video_audio() {
        let mut info = MediaInfo::default();
        assert!(!info.has_video());
        assert!(!info.has_audio());

        info.video = Some(VideoStreamInfo {
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::Yuv420p,
            frame_rate: None,
            time_base: Rational::new(1, 1000),
            duration: None,
            codec_id: CodecId::H264,
            extradata: None,
            bitrate: None,
            profile: None,
            level: None,
        });

        assert!(info.has_video());
        assert!(!info.has_audio());
    }
}
