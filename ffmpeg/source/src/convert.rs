/*!
    Conversion utilities between ffmpeg-next types and ffmpeg-types.
*/

use ffmpeg_types::{
    ChannelLayout, CodecId, MediaDuration, PixelFormat, Pts, Rational, SampleFormat,
};

/**
    Convert ffmpeg_next::Rational to our Rational.
*/
pub fn rational_from_ffmpeg(r: ffmpeg_next::Rational) -> Rational {
    Rational::new(r.numerator(), r.denominator())
}

/**
    Convert our Rational to ffmpeg_next::Rational.
*/
#[allow(dead_code)]
pub fn rational_to_ffmpeg(r: Rational) -> ffmpeg_next::Rational {
    ffmpeg_next::Rational::new(r.num, r.den)
}

/**
    Convert ffmpeg_next pixel format to our PixelFormat.
*/
pub fn pixel_format_from_ffmpeg(format: ffmpeg_next::format::Pixel) -> Option<PixelFormat> {
    use ffmpeg_next::format::Pixel;

    match format {
        Pixel::YUV420P => Some(PixelFormat::Yuv420p),
        Pixel::NV12 => Some(PixelFormat::Nv12),
        Pixel::BGRA => Some(PixelFormat::Bgra),
        Pixel::RGBA => Some(PixelFormat::Rgba),
        Pixel::RGB24 => Some(PixelFormat::Rgb24),
        Pixel::BGR24 => Some(PixelFormat::Bgr24),
        Pixel::YUV422P => Some(PixelFormat::Yuv422p),
        Pixel::YUV444P => Some(PixelFormat::Yuv444p),
        Pixel::YUV420P10LE | Pixel::YUV420P10BE => Some(PixelFormat::Yuv420p10),
        _ => None,
    }
}

/**
    Convert ffmpeg_next sample format to our SampleFormat.
*/
pub fn sample_format_from_ffmpeg(format: ffmpeg_next::format::Sample) -> Option<SampleFormat> {
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

/**
    Convert ffmpeg_next channel layout to our ChannelLayout.
*/
#[allow(dead_code)]
pub fn channel_layout_from_ffmpeg(layout: ffmpeg_next::ChannelLayout) -> Option<ChannelLayout> {
    if layout == ffmpeg_next::ChannelLayout::MONO {
        Some(ChannelLayout::Mono)
    } else if layout == ffmpeg_next::ChannelLayout::STEREO {
        Some(ChannelLayout::Stereo)
    } else {
        // For now, treat anything else as stereo if it has 2 channels
        // or mono if it has 1 channel
        None
    }
}

/**
    Convert channel count to our ChannelLayout.
*/
pub fn channel_layout_from_count(channels: u16) -> ChannelLayout {
    match channels {
        1 => ChannelLayout::Mono,
        _ => ChannelLayout::Stereo, // Default to stereo for 2+ channels
    }
}

/**
    Convert ffmpeg_next codec ID to our CodecId.
*/
pub fn codec_id_from_ffmpeg(id: ffmpeg_next::codec::Id) -> Option<CodecId> {
    use ffmpeg_next::codec::Id;

    match id {
        // Video
        Id::H264 => Some(CodecId::H264),
        Id::HEVC => Some(CodecId::H265),
        Id::VP8 => Some(CodecId::Vp8),
        Id::VP9 => Some(CodecId::Vp9),
        Id::AV1 => Some(CodecId::Av1),
        Id::MPEG4 => Some(CodecId::Mpeg4),
        Id::MPEG2VIDEO => Some(CodecId::Mpeg2Video),
        // Audio
        Id::AAC => Some(CodecId::Aac),
        Id::OPUS => Some(CodecId::Opus),
        Id::MP3 => Some(CodecId::Mp3),
        Id::VORBIS => Some(CodecId::Vorbis),
        Id::FLAC => Some(CodecId::Flac),
        Id::PCM_S16LE => Some(CodecId::PcmS16Le),
        Id::PCM_S16BE => Some(CodecId::PcmS16Be),
        Id::PCM_F32LE => Some(CodecId::PcmF32Le),
        Id::AC3 => Some(CodecId::Ac3),
        _ => None,
    }
}

/**
    Create a Pts from an optional i64 timestamp.
*/
pub fn pts_from_ffmpeg(pts: Option<i64>) -> Option<Pts> {
    pts.map(Pts)
}

/**
    Create a MediaDuration from an i64 duration.
*/
pub fn duration_from_ffmpeg(duration: i64) -> MediaDuration {
    MediaDuration(duration)
}
