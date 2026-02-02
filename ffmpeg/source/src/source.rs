/*!
    Media source implementation.
*/

use std::path::Path;
use std::time::Duration;

use ffmpeg_next::{Dictionary, format::context::Input as InputContext, media::Type};

use ffmpeg_types::{Error, MediaInfo, Packet, Rational, Result, StreamType};

use crate::codec_config::CodecConfig;
use crate::convert::{duration_from_ffmpeg, pts_from_ffmpeg, rational_from_ffmpeg};
use crate::probe::extract_media_info;

/**
    Network-specific options for HTTP/HLS sources.
*/
#[derive(Clone, Debug, Default)]
pub struct NetworkOptions {
    /// Connection/read timeout.
    pub timeout: Option<Duration>,
    /// Custom User-Agent header.
    pub user_agent: Option<String>,
    /// Custom HTTP headers (key-value pairs).
    pub headers: Option<Vec<(String, String)>>,
    /// Force HLS mode (for URLs that don't end in .m3u8).
    pub is_hls: bool,
    /// Reconnect on error (for live streams).
    pub reconnect: bool,
}

impl NetworkOptions {
    /**
        Create new network options with a timeout.
    */
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            ..Default::default()
        }
    }

    /**
        Set the User-Agent header.
    */
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /**
        Add a custom HTTP header.
    */
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(Vec::new)
            .push((key.into(), value.into()));
        self
    }

    /**
        Enable HLS mode.
    */
    pub fn hls(mut self) -> Self {
        self.is_hls = true;
        self
    }

    /**
        Enable reconnection on error.
    */
    pub fn reconnect(mut self) -> Self {
        self.reconnect = true;
        self
    }
}

/**
    Configuration for opening a media source.
*/
#[derive(Clone, Debug, Default)]
pub struct SourceConfig {
    /// Filter which streams to demux (None = all available).
    pub stream_filter: Option<StreamFilter>,
    /// Network options for HTTP/HLS sources.
    pub network_options: Option<NetworkOptions>,
}

impl SourceConfig {
    /**
        Set network options for HTTP/HLS sources.
    */
    pub fn with_network_options(mut self, opts: NetworkOptions) -> Self {
        self.network_options = Some(opts);
        self
    }
}

/**
    Filter for selecting which streams to demux.
*/
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamFilter {
    /// Only demux video streams.
    VideoOnly,
    /// Only demux audio streams.
    AudioOnly,
    /// Demux both video and audio streams.
    #[default]
    Both,
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
    /// Buffered packet from seek operation (returned on next next_packet call).
    buffered_packet: Option<Packet>,
    /// Whether this source is from a network URL (affects seeking behavior).
    is_network_source: bool,
}

/**
    Check if a string looks like a URL.
*/
fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("rtmp://")
}

impl Source {
    /**
        Open a media source from a file path or URL.

        Supports local files, HTTP/HTTPS URLs, and HLS streams.

        # Example

        ```ignore
        // Open a local file
        let source = Source::open("video.mp4", SourceConfig::default())?;
        println!("Duration: {:?}", source.media_info().duration);

        // Open from a PathBuf
        let path = PathBuf::from("video.mp4");
        let source = Source::open(&path, SourceConfig::default())?;

        // Open an HTTP URL
        let source = Source::open(
            "https://example.com/video.mp4",
            SourceConfig::default()
                .with_network_options(NetworkOptions::with_timeout(Duration::from_secs(30)))
        )?;

        // Open an HLS stream
        let source = Source::open(
            "https://example.com/stream/playlist.m3u8",
            SourceConfig::default()
        )?;
        ```
    */
    pub fn open<P: AsRef<Path>>(location: P, config: SourceConfig) -> Result<Self> {
        let path = location.as_ref();

        // Check if it's a URL by converting to string
        if let Some(path_str) = path.to_str() {
            if is_url(path_str) {
                return Self::open_url(path_str, config);
            }
        }

        Self::open_file(path, config)
    }

    /**
        Open a local media file.
    */
    fn open_file(path: &Path, config: SourceConfig) -> Result<Self> {
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

        Self::from_input_context(input, config, false)
    }

    /**
        Open a media source from a URL (HTTP, HTTPS, HLS).
    */
    fn open_url(url: &str, config: SourceConfig) -> Result<Self> {
        ffmpeg_next::init().map_err(|e| Error::codec(e.to_string()))?;

        let mut opts = Dictionary::new();

        // Set network options
        if let Some(ref net_opts) = config.network_options {
            if let Some(timeout) = net_opts.timeout {
                // FFmpeg uses timeout in microseconds
                opts.set("timeout", &(timeout.as_micros() as i64).to_string());
            }
            if let Some(ref user_agent) = net_opts.user_agent {
                opts.set("user_agent", user_agent);
            }
            // Add custom headers
            if let Some(ref headers) = net_opts.headers {
                let headers_str = headers
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<_>>()
                    .join("\r\n");
                if !headers_str.is_empty() {
                    opts.set("headers", &headers_str);
                }
            }
            // Reconnect on error for live streams
            if net_opts.reconnect {
                opts.set("reconnect", "1");
                opts.set("reconnect_streamed", "1");
                opts.set("reconnect_delay_max", "5");
            }
        }

        // HLS-specific options
        let is_hls = url.contains(".m3u8")
            || config
                .network_options
                .as_ref()
                .map(|o| o.is_hls)
                .unwrap_or(false);
        if is_hls {
            opts.set("allowed_extensions", "ALL");
        }

        let input = ffmpeg_next::format::input_with_dictionary(&url, opts)
            .map_err(|e| Error::codec(format!("failed to open URL: {}", e)))?;

        Self::from_input_context(input, config, true)
    }

    /**
        Create a Source from an FFmpeg input context.
    */
    fn from_input_context(
        input: InputContext,
        config: SourceConfig,
        is_network_source: bool,
    ) -> Result<Self> {
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
            buffered_packet: None,
            is_network_source,
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
        Check if this source is from a network URL.

        Network sources may have different behavior for seeking and buffering.
    */
    pub fn is_network_source(&self) -> bool {
        self.is_network_source
    }

    /**
        Check if this source supports seeking.

        Network streams (especially live streams) may not support seeking.

        Returns false for live streams or sources without a seekable I/O context.
    */
    pub fn is_seekable(&self) -> bool {
        // Check FFmpeg's I/O context for seekability
        unsafe {
            let ctx = self.input.as_ptr();
            // If no I/O context, assume not seekable
            if (*ctx).pb.is_null() {
                return false;
            }
            // Check the seekable flag on the I/O context
            (*(*ctx).pb).seekable != 0
        }
    }

    /**
        Read the next packet from the source.

        Returns `Ok(Some(packet))` for each packet, `Ok(None)` at end of stream,
        or an error if something goes wrong.

        Packets are returned in file order, interleaved between streams.
        Use `packet.stream_type` to determine if it's video or audio.
    */
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        // Return buffered packet first (from seek operation)
        if let Some(packet) = self.buffered_packet.take() {
            return Ok(Some(packet));
        }

        self.read_next_packet_internal()
    }

    /**
        Internal method to read the next packet from the demuxer.
    */
    fn read_next_packet_internal(&mut self) -> Result<Option<Packet>> {
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

        Returns the actual position that was seeked to (the keyframe position),
        which may be before the requested target due to keyframe alignment.
        Callers should use this returned position to reset their playback clock.

        Returns an error if the source does not support seeking (e.g., live streams).

        Use [`is_seekable`](Self::is_seekable) to check before calling.
    */
    pub fn seek(&mut self, position: Duration) -> Result<Duration> {
        // Ensure seeking is supported
        if !self.is_seekable() {
            return Err(Error::unsupported_format(
                "source does not support seeking (may be a live stream)",
            ));
        }

        // Convert position to FFmpeg's time base (microseconds)
        let timestamp = (position.as_secs_f64() * ffmpeg_next::ffi::AV_TIME_BASE as f64) as i64;

        self.input
            .seek(timestamp, ..timestamp)
            .map_err(|e| Error::codec(format!("seek failed: {}", e)))?;

        // Clear any previously buffered packet
        self.buffered_packet = None;

        // Read the first packet to determine actual position.
        // This packet is buffered and will be returned by next_packet().
        if let Some(packet) = self.read_next_packet_internal()? {
            let actual_position = packet.presentation_time().unwrap_or(position);
            self.buffered_packet = Some(packet);
            Ok(actual_position)
        } else {
            // No packets available (e.g., seeked past end)
            Ok(position)
        }
    }
}

/**
    Open a media source with default configuration.

    This is a convenience function equivalent to `Source::open(location, SourceConfig::default())`.

    Accepts both file paths and URLs.
*/
pub fn open<P: AsRef<Path>>(location: P) -> Result<Source> {
    Source::open(location, SourceConfig::default())
}

/**
    Open a media source with the given configuration.

    Accepts both file paths and URLs.
*/
pub fn open_with_config<P: AsRef<Path>>(location: P, config: SourceConfig) -> Result<Source> {
    Source::open(location, config)
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
