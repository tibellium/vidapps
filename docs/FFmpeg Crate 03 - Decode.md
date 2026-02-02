# FFmpeg Crate 03 - Decode

## Purpose

`ffmpeg-decode` transforms encoded packets into raw frames. It handles the computationally intensive work of codec decoding, including hardware acceleration when available.

This is where H.264 bitstreams become YUV pixel buffers, and where AAC frames become PCM samples.

## Why Decode is Complex

Video decoding is one of the most complex operations in a media pipeline:

### Temporal Dependencies

Video codecs use three frame types:

- **I-frames** (keyframes): Complete images, decode independently
- **P-frames**: Predicted from previous frames
- **B-frames**: Bi-directionally predicted from both past and future frames

This means:

- Decoding a P-frame requires having decoded previous frames
- Decoding a B-frame requires decoding frames that come _after_ it in the stream
- The decoder must buffer frames internally

### Decode Order vs Display Order

Because B-frames reference future frames, packets arrive in _decode order_ (DTS), but frames come out in _display order_ (PTS). The decoder handles this reordering internally.

This is why `decode()` returns `Vec<VideoFrame>` — you might get zero, one, or multiple frames from a single packet.

### Hardware Acceleration

Modern GPUs have dedicated video decode hardware:

- **VideoToolbox** (macOS): Decodes H.264, H.265, VP9
- **VAAPI** (Linux): AMD, Intel integrated graphics
- **NVDEC** (NVIDIA): CUDA-based decoding
- **QSV** (Intel): Quick Sync Video

Hardware decode is 10-100x more power-efficient than software. For battery-powered devices, it's essential.

## Hardware Acceleration Design

### Why Opt-In

Hardware acceleration isn't always better:

- Adds latency (frames buffered on GPU)
- Limited format support (not all codecs)
- Quality differences in some edge cases
- Debugging is harder (can't inspect GPU state)

We make it opt-in with automatic fallback:

```
let config = DecoderConfig {
    prefer_hw: true,
    hw_device: None,  // auto-detect
};
```

If hardware setup fails, we silently fall back to software. The consumer can check `decoder.is_hw_accelerated()` if they care.

### GPU Frame Transfer

Hardware decoders produce frames in GPU memory. These must be transferred to CPU memory before the rest of the pipeline can use them.

The transfer happens inside `decode()`:

1. Decoder outputs frame in GPU memory
2. We detect it's a hardware frame (`is_hw_frame()`)
3. We transfer to CPU memory (`av_hwframe_transfer_data`)
4. Return CPU-based `VideoFrame`

This hides the complexity from consumers — they always get CPU frames.

### Future: Zero-Copy GPU Pipeline

For GPU rendering (OpenGL, Vulkan, Metal), transferring to CPU is wasteful. A future optimization could provide:

- `VideoFrameGpu` type that stays on GPU
- Direct texture import for rendering
- Only transfer to CPU if consumer needs it

This is out of scope for the initial implementation.

## Decoder State

### Codec Context

FFmpeg decoders are stateful. The codec context contains:

- Codec parameters (from container)
- Reference frames (for inter-prediction)
- Internal buffers
- Hardware device context (if using HW accel)

The decoder owns its context exclusively — no sharing between decoder instances.

### Why Decoders Aren't Clone

Decoder state is too complex to clone:

- Hardware contexts have GPU resources
- Reference frames would need copying
- Buffered packets would need duplicating

If you need multiple decoders, create multiple instances. They can decode the same stream in parallel for seeking (decode ahead to a keyframe).

## Frame Output

### Pixel Formats

Decoders output frames in the codec's native format:

- H.264: Usually YUV420P or NV12
- VP9: YUV420P, sometimes 10-bit
- ProRes: YUV422P or YUV444P

We don't convert formats in the decoder — that's `ffmpeg-transform`'s job. The decoder outputs whatever the codec produces.

The `VideoFrame.format` field tells you what you got.

### Timestamps

Frames carry PTS (presentation timestamp) from the decoded data. The decoder:

- Extracts PTS from packet (or derives from frame)
- Handles B-frame reordering
- Outputs frames in display order with correct PTS

Frames always have a time_base matching the source stream.

## Audio Decoding

Audio is simpler than video:

- No inter-frame dependencies (usually)
- No reordering
- No hardware acceleration (CPU is fast enough)

Audio decoders:

1. Receive compressed packet (AAC, MP3, Opus, etc.)
2. Output PCM samples in codec's native format
3. Format might be planar or interleaved, various bit depths

Like video, we don't convert formats in the decoder.

## Handling Discontinuities

### The Seek Problem

When seeking:

1. Source jumps to new position
2. Decoder has frames from old position buffered
3. These must be discarded

The `reset()` method:

- Flushes internal buffers
- Clears reference frames
- Prepares for fresh decode from keyframe

Consumer must call `reset()` after seeking before decoding new packets.

### Flush at End of Stream

At end of stream, decoders may have buffered frames not yet output (B-frame delay). The `flush()` method:

- Signals no more packets coming
- Drains all buffered frames
- Returns remaining frames

Always call `flush()` when you know the stream is ending to get all frames.

## Error Handling

### Recoverable vs Fatal Errors

Some decode errors are recoverable:

- Corrupted packet: Skip it, try next
- Missing reference frame: Might recover at next keyframe

Some are fatal:

- Invalid codec parameters: Can't continue
- Hardware device lost: Need to recreate decoder

We surface all errors to the consumer, who decides whether to retry or abort.

### Partial Decode

A corrupted stream might decode some frames successfully before hitting errors. We don't throw away good frames — `decode()` returns what it can before returning an error.

## Thread Safety

### Send but not Sync

Decoders are `Send` (can move to another thread) but not `Sync` (can't share between threads). This is because:

- FFmpeg codec contexts aren't thread-safe
- Hardware contexts are typically single-threaded

The pattern is: create decoder, move to dedicated thread, use exclusively from that thread.

### Internal Threading

FFmpeg decoders can use multiple threads internally (frame-level or slice-level threading). This is configured via codec options and is transparent to our API.

We expose this via feature flags if needed, but default to FFmpeg's auto-detection.

## What This Crate Does NOT Do

- **No demuxing**: Expects packets, not files/URLs
- **No format conversion**: Outputs native codec format
- **No thread management**: Consumer's responsibility
- **No buffering policy**: Consumer decides how many frames to buffer
- **No timing/sync**: Just decodes as fast as packets arrive

## Relationship to vidwall

The vidwall `decode_video_packets()` and `decode_audio_packets()` functions combine:

- Packet reception (from queue)
- Decoding
- Scaling/resampling (transform)
- Frame output (to queue)
- Stop flag handling

In the crate design:

- Decoding is isolated to this crate
- Transform is separate
- Queueing and threading are consumer concerns

The vidwall functions will be refactored to use these crates, with the queueing logic staying in vidwall.
