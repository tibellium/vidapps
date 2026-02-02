/*!
    Media source and demuxing for the ffmpeg crate ecosystem.

    This crate handles the input side of the media pipeline. It opens media from
    various sources (files, HTTP, HLS streams), parses containers, and produces
    encoded packets that downstream crates can decode.

    # Features

    - `file` (default): Support for local file sources
    - `http` (future): Support for HTTP/HTTPS sources
    - `hls` (future): Support for HLS streaming sources

    # Example

    ```ignore
    use ffmpeg_source::{open, probe};

    // Probe a file for metadata
    let info = probe("video.mp4")?;
    println!("Duration: {:?}", info.duration);
    println!("Has video: {}", info.has_video());
    println!("Has audio: {}", info.has_audio());

    // Open and read packets
    let mut source = open("video.mp4")?;
    while let Some(packet) = source.next_packet()? {
        match packet.stream_type {
            StreamType::Video => { /* decode video */ }
            StreamType::Audio => { /* decode audio */ }
        }
    }
    ```

    # Architecture

    The source crate is designed to be extensible for different input types:

    - **File sources**: Direct filesystem access (current implementation)
    - **HTTP sources**: Network streaming via reqwest (planned)
    - **HLS sources**: Adaptive streaming with segment management (planned)

    All source types produce the same `Packet` type, allowing downstream
    decoders to work uniformly regardless of input source.
*/

pub use ffmpeg_types::{
    AudioStreamInfo, Error, MediaInfo, Packet, Result, StreamType, VideoStreamInfo,
};

mod codec_config;
mod convert;
mod probe;
mod source;

pub use codec_config::CodecConfig;
pub use probe::probe;
pub use source::{Source, SourceConfig, StreamFilter, open, open_with_config};
