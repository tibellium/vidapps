/*!
    Media encoding for the ffmpeg crate ecosystem.

    This crate transforms raw frames into compressed packets. It's the inverse
    of decode — taking PCM audio and raw video and producing H.264, AAC, or
    other codec bitstreams.

    # Video Encoding

    ```ignore
    use ffmpeg_encode::{VideoEncoder, VideoEncoderConfig, RateControl};
    use ffmpeg_types::Rational;

    // Create H.264 encoder at 1080p 30fps
    let config = VideoEncoderConfig::h264(1920, 1080, Rational { num: 30, den: 1 })
        .with_crf(23)  // Good quality
        .with_preset(EncoderPreset::Fast);

    let mut encoder = VideoEncoder::new(config)?;

    // Encode frames (must be YUV420P by default)
    for frame in video_frames {
        let packets = encoder.encode(&frame)?;
        for packet in packets {
            // Write to muxer
        }
    }

    // Flush remaining packets
    let final_packets = encoder.flush()?;
    ```

    # Audio Encoding

    ```ignore
    use ffmpeg_encode::{AudioEncoder, AudioEncoderConfig};
    use ffmpeg_types::ChannelLayout;

    // Create AAC encoder at 48kHz stereo
    let config = AudioEncoderConfig::aac(48000, ChannelLayout::Stereo)
        .with_bitrate(192_000);  // 192 kbps

    let mut encoder = AudioEncoder::new(config)?;

    // Encode frames
    for frame in audio_frames {
        let packets = encoder.encode(&frame)?;
        for packet in packets {
            // Write to muxer
        }
    }

    // Flush remaining packets
    let final_packets = encoder.flush()?;
    ```

    # Rate Control

    Video encoding supports several rate control modes:

    - **CRF (Constant Rate Factor)**: Target constant quality, variable bitrate.
      Lower values = higher quality. 18-28 is typical range.
    - **CBR (Constant Bitrate)**: Fixed bitrate throughout. Required for some
      streaming protocols.
    - **VBR (Variable Bitrate)**: Target average bitrate with quality variation.

    ```ignore
    // CRF mode (quality-based)
    config.with_crf(23)

    // CBR mode (5 Mbps)
    config.with_bitrate(5_000_000)

    // VBR mode
    config.with_rate_control(RateControl::Vbr(5_000_000))
    ```

    # Presets

    Encoder presets trade speed for compression efficiency:

    - `Ultrafast`: Fastest, largest files
    - `Fast`: Good for real-time
    - `Medium`: Default balance
    - `Slow`/`Veryslow`: Best compression, slowest

    ```ignore
    config.with_preset(EncoderPreset::Fast)  // For real-time
    config.with_preset(EncoderPreset::Slow)  // For archival
    ```

    # Frame Requirements

    Encoders expect specific input formats:

    - **H.264**: Typically YUV420P
    - **AAC**: Typically F32 or S16

    Use `ffmpeg-transform` to convert frames to the required format before encoding.

    # Features

    - `videotoolbox`: VideoToolbox hardware encoding (macOS)
    - `nvenc`: NVIDIA NVENC hardware encoding
    - `qsv`: Intel Quick Sync Video
    - `vaapi`: VA-API hardware encoding (Linux)
*/

pub use ffmpeg_types::{
    AudioFrame, AudioStreamInfo, ChannelLayout, CodecId, Error, Packet, PixelFormat, Rational,
    Result, SampleFormat, VideoFrame, VideoStreamInfo,
};

mod audio;
mod config;
mod video;

pub use audio::AudioEncoder;
pub use config::{AudioEncoderConfig, EncoderPreset, HwEncoder, RateControl, VideoEncoderConfig};
pub use video::VideoEncoder;
