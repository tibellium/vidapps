# FFmpeg Crate 05 - Encode

## Purpose

`ffmpeg-encode` transforms raw frames into compressed packets. It's the inverse of decode — taking PCM audio and raw video and producing H.264, AAC, or other codec bitstreams.

This enables transcoding, recording, and streaming workflows.

## Why Encoding is Different from Decoding

While decode and encode are conceptually inverse operations, they have different characteristics:

### Encoding is Configurable

Decoders have one job: reconstruct what the encoder produced. There's one correct output for a given input.

Encoders have many choices:

- Bitrate (quality vs file size)
- Preset (speed vs compression efficiency)
- Profile/level (compatibility vs features)
- GOP structure (keyframe interval)
- Rate control mode (CBR, VBR, CRF)

We must expose these options while keeping the API manageable.

### Encoding is Slower

Modern codecs are designed for "encode once, decode many times":

- Decoding: ~10ms per 1080p frame
- Encoding: ~50-500ms per 1080p frame (depends on preset)

This asymmetry is intentional — viewers outnumber producers.

### Encoding Quality is Subjective

Decode quality is objective (either correct or corrupted). Encode quality is subjective:

- Blocking artifacts vs blur
- Preservation of detail vs file size
- Motion smoothness vs bitrate

We provide presets that embody sensible quality trade-offs.

## Video Encoding

### Supported Codecs

**H.264 (AVC)**

- Most compatible codec
- Supported by virtually all devices
- Good quality at reasonable bitrates
- Primary codec for most use cases

**H.265 (HEVC)**

- 30-50% better compression than H.264
- Higher CPU cost
- Patent licensing concerns
- Use for: archival, bandwidth-constrained

**VP9**

- Royalty-free
- Similar compression to H.265
- Good for web (YouTube uses it)

**AV1**

- Next-generation, royalty-free
- Best compression
- Very slow to encode (for now)
- Use for: future-proofing, bandwidth-critical

We start with H.264 as it covers most use cases.

### Rate Control

**CRF (Constant Rate Factor)**

- Target constant quality
- Bitrate varies per scene
- Best for archival
- CRF 18-23 is "visually lossless" for H.264

**CBR (Constant Bitrate)**

- Fixed bitrate throughout
- Predictable file size
- Required for some streaming protocols

**VBR (Variable Bitrate)**

- Target average bitrate
- Quality varies less than CBR
- Good balance for streaming

We expose these via the config:

```
VideoEncoderConfig {
    bitrate: Some(5_000_000),  // 5 Mbps CBR
    // or
    quality: Some(23),  // CRF mode
}
```

### Presets

Speed vs compression trade-off:

- **ultrafast**: Fastest, largest files
- **superfast/veryfast/faster/fast**: Progressively better compression
- **medium**: Default, good balance
- **slow/slower/veryslow**: Best compression, slowest

Slower presets find better ways to compress the same content. The quality for a given bitrate improves, but encoding takes longer.

For real-time streaming, use fast presets. For archival, use slower presets.

### GOP Structure

GOP (Group of Pictures) defines keyframe placement:

- Keyframe interval: frames between keyframes (e.g., 60 = 2 seconds at 30fps)
- B-frames: bidirectional frames between P-frames (improve compression)

Shorter GOP = more keyframes = easier seeking = larger files
Longer GOP = fewer keyframes = harder seeking = smaller files

Default: keyframe every 2 seconds, which balances seeking and compression.

## Audio Encoding

### Supported Codecs

**AAC**

- Most compatible lossy codec
- Good quality at 128-256 kbps
- Required for MP4 containers

**Opus**

- Best quality at low bitrates
- Excellent for voice and music
- Used in WebM, WebRTC

**MP3**

- Legacy compatibility
- Lower quality than AAC/Opus
- Use only when required for compatibility

We start with AAC for broad compatibility.

### Bitrate

Audio bitrate recommendations:

- 96 kbps: Acceptable for voice
- 128 kbps: Good for music (stereo)
- 192 kbps: Very good quality
- 256 kbps: Transparent for most listeners
- 320 kbps: Maximum for MP3/AAC (diminishing returns)

### Sample Rate Considerations

Encoders expect specific sample rates:

- AAC: 44100 or 48000 Hz typical
- Opus: 48000 Hz native

If input doesn't match, consumer must transform first (ffmpeg-transform).

## Frame Input Requirements

### Format Matching

Encoders expect specific input formats:

- H.264: Typically YUV420P
- AAC: Float or S16

Consumer must transform frames to the expected format before encoding. We document expected formats per codec.

### Timestamp Handling

Frames must have valid PTS for encoder to produce correct DTS/PTS in output packets:

- Frames should have monotonically increasing PTS
- Missing PTS results in incorrect timing
- Incorrect timing breaks playback

If transforming changes frame count (rare with our simple transforms), timestamps must be adjusted.

## Buffering Behavior

### Why Encoders Buffer

Like decoders, encoders buffer frames internally:

- B-frame encoding needs future frames
- Rate control looks at upcoming frames
- Lookahead improves quality

This means:

- `encode(frame)` might return empty Vec (frame buffered)
- Later `encode()` calls might return multiple packets
- `flush()` returns remaining buffered packets

### Latency Implications

Buffering adds latency:

- ultrafast preset: minimal buffering
- veryslow preset: significant buffering

For low-latency streaming, use fast presets and consider disabling B-frames.

## Stream Info for Muxer

After encoder initialization, we can provide stream info that the muxer needs:

- Codec parameters (for container header)
- Time base
- Expected dimensions/sample rate

This info comes from `encoder.stream_info()` and is passed to `ffmpeg-sink`.

## Hardware Encoding

### Available Hardware Encoders

Like decode, encoding can be hardware-accelerated:

- **VideoToolbox** (macOS): H.264, H.265
- **NVENC** (NVIDIA): H.264, H.265
- **QSV** (Intel): H.264, H.265
- **VAAPI** (Linux AMD/Intel): H.264, H.265

Hardware encoding is:

- Much faster (10-100x)
- Lower quality per bitrate (typically)
- Lower power consumption

### Trade-offs

Hardware encoders optimize for speed, not compression efficiency:

- Same bitrate: software looks better
- Same quality: software uses less bitrate

Use hardware when:

- Real-time encoding required
- Power consumption matters
- Quality can be compensated with higher bitrate

Use software when:

- Quality matters most
- Time isn't critical (offline encoding)
- Bitrate is constrained

### Configuration

```
VideoEncoderConfig {
    codec: CodecId::H264,
    prefer_hw: true,
    hw_device: Some(HwDevice::VideoToolbox),
    // ...
}
```

We fall back to software if hardware isn't available.

## Error Handling

### Invalid Configuration

Some configurations are invalid:

- Unsupported codec
- Impossible dimension/bitrate combination
- Incompatible profile/level settings

These produce errors at encoder creation time.

### Encoding Errors

During encoding:

- Invalid input format: error
- Frame size mismatch: error
- Resource exhaustion: error

We surface these immediately; consumer decides whether to skip frame or abort.

## What This Crate Does NOT Do

- **No muxing**: Outputs packets, not container files
- **No format conversion**: Expects frames in encoder-compatible format
- **No thread management**: Consumer handles threading
- **No rate pacing**: Consumer decides when to encode frames
- **No two-pass encoding**: Out of scope (significantly complicates API)

## Relationship to vidwall

vidwall currently doesn't encode — it's a playback application. The encode crate enables new use cases:

- Screen recording
- Transcoding for different devices
- Live streaming

When vidwall or a related app needs these features, the encode crate provides them.
