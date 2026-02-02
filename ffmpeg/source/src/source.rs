/*!
    Media source implementation.
*/

use std::path::Path;
use std::time::Duration;

use ffmpeg_next::{format::context::Input as InputContext, media::Type};

use ffmpeg_types::{Error, MediaInfo, Packet, Rational, Result, StreamType};

use crate::codec_config::CodecConfig;
use crate::convert::{duration_from_ffmpeg, pts_from_ffmpeg, rational_from_ffmpeg};
use crate::probe::extract_media_info;

/**
    Configuration for opening a media source.
*/
#[derive(Clone, Debug, Default)]
pub struct SourceConfig {
    /// Filter which streams to demux (None = all available).
    pub stream_filter: Option<StreamFilter>,
}

/**
    Filter for selecting which streams to demux.
*/
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFilter {
    /// Only demux video streams.
    VideoOnly,
    /// Only demux audio streams.
    AudioOnly,
    /// Demux both video and audio streams.
    Both,
}

impl Default for StreamFilter {
    fn default() -> Self {
        Self::Both
    }
}

/**
    A media source that produces encoded packets.

    Created by [`open`] or [`Source::open`]. Provides access to stream
    information and produces packets via iteration.
*/
pub struct Source {
    /// The FFmpeg input context.
    input: InputContext,
    /// Cached media info.
    media_info: MediaInfo,
    /// Video stream index (if present and wanted).
    video_stream_index: Option<usize>,
    /// Audio stream index (if present and wanted).
    audio_stream_index: Option<usize>,
    /// Video stream time base.
    video_time_base: Option<Rational>,
    /// Audio stream time base.
    audio_time_base: Option<Rational>,
    /// Video codec config (if present).
    video_codec_config: Option<CodecConfig>,
    /// Audio codec config (if present).
    audio_codec_config: Option<CodecConfig>,
}

impl Source {
    /**
        Open a media file.

        # Example

        ```ignore
        let source = Source::open("video.mp4", SourceConfig::default())?;
        println!("Duration: {:?}", source.media_info().duration);
        ```
    */
    pub fn open<P: AsRef<Path>>(path: P, config: SourceConfig) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        let input = ffmpeg_next::format::input(&path).map_err(|e| {
            if e.to_string().contains("No such file") {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    e.to_string(),
                ))
            } else {
                Error::codec(e.to_string())
            }
        })?;

        let media_info = extract_media_info(&input)?;

        // Determine which streams to use based on filter
        let want_video = match config.stream_filter {
            None | Some(StreamFilter::Both) | Some(StreamFilter::VideoOnly) => true,
            Some(StreamFilter::AudioOnly) => false,
        };
        let want_audio = match config.stream_filter {
            None | Some(StreamFilter::Both) | Some(StreamFilter::AudioOnly) => true,
            Some(StreamFilter::VideoOnly) => false,
        };

        // Find video stream
        let (video_stream_index, video_time_base, video_codec_config) = if want_video {
            if let Some(stream) = input.streams().best(Type::Video) {
                let index = stream.index();
                let time_base = rational_from_ffmpeg(stream.time_base());
                let codec_config = CodecConfig::new(stream.parameters());
                (Some(index), Some(time_base), Some(codec_config))
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        // Find audio stream
        let (audio_stream_index, audio_time_base, audio_codec_config) = if want_audio {
            if let Some(stream) = input.streams().best(Type::Audio) {
                let index = stream.index();
                let time_base = rational_from_ffmpeg(stream.time_base());
                let codec_config = CodecConfig::new(stream.parameters());
                (Some(index), Some(time_base), Some(codec_config))
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        Ok(Self {
            input,
            media_info,
            video_stream_index,
            audio_stream_index,
            video_time_base,
            audio_time_base,
            video_codec_config,
            audio_codec_config,
        })
    }

    /**
        Get the media info for this source.
    */
    pub fn media_info(&self) -> &MediaInfo {
        &self.media_info
    }

    /**
        Get the video codec configuration, if video is present.

        Pass this to `ffmpeg-decode` to create a video decoder.
    */
    pub fn video_codec_config(&self) -> Option<&CodecConfig> {
        self.video_codec_config.as_ref()
    }

    /**
        Take the video codec configuration, if video is present.

        This consumes the codec config from the source.
    */
    pub fn take_video_codec_config(&mut self) -> Option<CodecConfig> {
        self.video_codec_config.take()
    }

    /**
        Get the audio codec configuration, if audio is present.

        Pass this to `ffmpeg-decode` to create an audio decoder.
    */
    pub fn audio_codec_config(&self) -> Option<&CodecConfig> {
        self.audio_codec_config.as_ref()
    }

    /**
        Take the audio codec configuration, if audio is present.

        This consumes the codec config from the source.
    */
    pub fn take_audio_codec_config(&mut self) -> Option<CodecConfig> {
        self.audio_codec_config.take()
    }

    /**
        Get the video stream time base, if video is present.
    */
    pub fn video_time_base(&self) -> Option<Rational> {
        self.video_time_base
    }

    /**
        Get the audio stream time base, if audio is present.
    */
    pub fn audio_time_base(&self) -> Option<Rational> {
        self.audio_time_base
    }

    /**
        Check if this source has video.
    */
    pub fn has_video(&self) -> bool {
        self.video_stream_index.is_some()
    }

    /**
        Check if this source has audio.
    */
    pub fn has_audio(&self) -> bool {
        self.audio_stream_index.is_some()
    }

    /**
        Read the next packet from the source.

        Returns `Ok(Some(packet))` for each packet, `Ok(None)` at end of stream,
        or an error if something goes wrong.

        Packets are returned in file order, interleaved between streams.
        Use `packet.stream_type` to determine if it's video or audio.
    */
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        loop {
            // Get next packet from demuxer
            let (stream, ffmpeg_packet) = match self.input.packets().next() {
                Some(result) => result,
                None => return Ok(None), // End of stream
            };

            let stream_index = stream.index();

            // Check if this is a stream we want
            let (stream_type, time_base) = if Some(stream_index) == self.video_stream_index {
                (StreamType::Video, self.video_time_base.unwrap())
            } else if Some(stream_index) == self.audio_stream_index {
                (StreamType::Audio, self.audio_time_base.unwrap())
            } else {
                // Skip streams we don't want
                continue;
            };

            // Check for keyframe
            let is_keyframe = ffmpeg_packet.is_key();

            // Extract packet data
            let data = ffmpeg_packet.data().map(|d| d.to_vec()).unwrap_or_default();

            let packet = Packet::new(
                data,
                pts_from_ffmpeg(ffmpeg_packet.pts()),
                pts_from_ffmpeg(ffmpeg_packet.dts()),
                duration_from_ffmpeg(ffmpeg_packet.duration()),
                time_base,
                is_keyframe,
                stream_type,
            );

            return Ok(Some(packet));
        }
    }

    /**
        Seek to a position in the media.

        Seeks to the nearest keyframe at or before the target position.
        After seeking, you should flush any decoder buffers.

        Note: The actual position after seeking may be before the target
        due to keyframe alignment.
    */
    pub fn seek(&mut self, position: Duration) -> Result<()> {
        // Convert position to FFmpeg's time base (microseconds)
        let timestamp = (position.as_secs_f64() * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;

        self.input
            .seek(timestamp, ..timestamp)
            .map_err(|e| Error::codec(format!("seek failed: {}", e)))?;

        Ok(())
    }
}

/**
    Open a media file with default configuration.

    This is a convenience function equivalent to `Source::open(path, SourceConfig::default())`.
*/
pub fn open<P: AsRef<Path>>(path: P) -> Result<Source> {
    Source::open(path, SourceConfig::default())
}

/**
    Open a media file with the given configuration.
*/
pub fn open_with_config<P: AsRef<Path>>(path: P, config: SourceConfig) -> Result<Source> {
    Source::open(path, config)
}

/**
    Iterator adapter for Source that yields packets.
*/
impl Iterator for Source {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_packet() {
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
