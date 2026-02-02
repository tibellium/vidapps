# FFmpeg Crate 01 - Types

## Purpose

`ffmpeg-types` is the foundation crate that all other crates depend on. It defines the vocabulary of the ecosystem — the types that cross crate boundaries. Crucially, it has **no dependency on FFmpeg**, making it lightweight and enabling consumers to depend on it without pulling in FFmpeg bindings.

## Why a Separate Types Crate?

Without a shared types crate, you get one of two problems:

1. **Circular dependencies**: If `ffmpeg-decode` defines `VideoFrame` and `ffmpeg-transform` needs to accept it, transform must depend on decode. But what if decode wants to use a transform utility? Circular.

2. **Duplicate definitions**: Each crate defines its own `VideoFrame`, requiring conversion at every boundary. Tedious and error-prone.

The types crate solves both by being the single source of truth that everyone depends on.

## Design Principles

### No FFmpeg Dependency

This is non-negotiable. The types crate must compile without FFmpeg installed. This means:

- No `ffmpeg-next` in dependencies
- No FFI types leaking into public API
- All types are pure Rust

The practical benefit: applications can use `ffmpeg-types` in modules that don't touch FFmpeg directly (UI code, configuration, testing utilities).

### Owned Data Over Borrowed

Frames and packets own their data via `Vec<u8>` rather than borrowing. This is a deliberate trade-off:

**Why owned:**

- Frames often cross thread boundaries (decode thread → render thread)
- Borrowed data requires lifetime tracking that complicates pipeline architectures
- Most media pipelines aren't bottlenecked by memory copies — decode and encode are the expensive operations

**When this might change:**
If profiling shows frame copying is a bottleneck (unlikely for most use cases), we could add `VideoFrameRef<'a>` variants or use `bytes::Bytes` for reference-counted sharing.

### Timestamps as Separate Types

We define `Pts` and `MediaDuration` as distinct types rather than using `i64` directly:

- Prevents mixing up PTS, DTS, and duration values
- Makes time base explicit in conversions
- Type system catches "forgot to convert timestamp" bugs

The conversion to `std::time::Duration` requires an explicit time base, forcing correct usage.

## Clock System

### Why Clocks Are Complex

A/V sync seems simple until you try to implement it. The challenges:

1. **Audio is the master clock**: Human perception is more sensitive to audio glitches than video frame drops. Audio playback rate is fixed by the sound card. Video must adapt.

2. **Audio runs on a callback thread**: The audio subsystem calls you when it needs samples. You don't control the timing.

3. **What happens when audio ends?**: If a video is 10 seconds but audio is 8 seconds, the video must keep playing for 2 more seconds. The clock can't just stop.

4. **Seeking**: When the user seeks, the clock must jump to the new position instantly.

### AudioClock Design

The `AudioClock` tracks position based on samples consumed by the audio output:

1. Audio callback consumes N samples → calls `add_samples(N)`
2. Video renderer calls `position()` → gets current playback time
3. Video displays frame whose PTS ≤ current position

When audio ends:

1. Audio callback finds buffer empty and closed → calls `mark_finished()`
2. Clock records current position and wall time
3. Future `position()` calls extrapolate: `position_at_finish + time_since_finish`

This "switch to wall time" pattern is borrowed from the vidwall `AudioStreamClock` implementation, which handles this correctly.

### WallClock for Silent Videos

Videos without audio tracks need a different clock. `WallClock` uses wall time with pause support:

- `new()` records start time
- `position()` returns elapsed time since start (minus paused duration)
- `pause()` / `resume()` track paused intervals
- `reset_to()` adjusts the base time for seeking

### Why Clock is a Trait

Applications may have different sync requirements:

- A transcoder doesn't need real-time sync — it processes as fast as possible
- A video editor might sync to a timeline cursor, not playback time
- Some applications might want to artificially slow down or speed up playback

The `Clock` trait allows these variations while sharing the frame selection logic.

## Error Types

### Why a Common Error Type

Each crate could define its own error enum, but:

- Consumers often compose multiple crates and want uniform error handling
- FFmpeg errors are common across all crates — no need to wrap them multiple times
- `?` works smoothly when all crates use the same error type

### Error Variants

- `Io`: File/network errors — common across source and sink
- `Codec`: FFmpeg codec errors — decode and encode
- `InvalidData`: Malformed input — source and decode
- `UnsupportedFormat`: Format we don't handle — all crates
- `Eof`: End of stream — not really an error, but part of the control flow

Each crate re-exports `Error` from `ffmpeg-types` so consumers don't need to import from multiple places.

## Stream Info Types

### VideoStreamInfo / AudioStreamInfo

These describe a stream without owning decoder state. They're returned by `ffmpeg-source::probe()` and used to configure decoders.

Key fields:

- Dimensions/sample rate — needed to allocate output buffers
- Pixel/sample format — needed to configure transformers
- Time base — needed for timestamp conversion
- Duration — useful for UI (progress bars) but often approximate
- Codec ID — informational, decoder uses codec parameters internally

### CodecId Enum

We define a subset of codecs we actually support rather than exposing all of FFmpeg's codec IDs. This:

- Documents what the ecosystem handles
- Prevents consumers from expecting unsupported codecs to work
- Keeps the enum manageable

New codecs are added as we implement support for them.

## Pipeline Signals

### Why Explicit Signals

When seeking, the pipeline needs to coordinate:

1. Source seeks to new position
2. Decoder flushes buffered frames (they're from the old position)
3. Transformer resets any internal state
4. Consumer discards queued frames

Rather than having each component guess when a discontinuity occurred, we make it explicit with `PipelineSignal::Flush`.

### Signal Types

- `Flush`: Clear buffers, discontinuity ahead (seek)
- `Eos`: End of stream, no more data coming

These flow through the pipeline alongside packets/frames. The consumer's pipeline code handles them appropriately.

## Frame Types

### VideoFrame

Fields:

- `data: Vec<u8>` — pixel data, layout depends on format
- `width`, `height` — dimensions in pixels
- `format: PixelFormat` — how to interpret the data
- `pts: Option<Pts>` — presentation timestamp (None for generated frames)
- `time_base: Rational` — for converting PTS to Duration

The format field is important because frames may be in different formats at different pipeline stages:

- After decode: Whatever the codec outputs (often YUV420P)
- After transform: Whatever the consumer requested (often BGRA for display)

### AudioFrame

Similar structure but for audio:

- `data: Vec<u8>` — sample data, layout depends on format
- `samples: usize` — number of samples (not bytes)
- `sample_rate`, `channels`, `format` — how to interpret the data
- `pts`, `time_base` — timing info

The data is raw bytes rather than `Vec<f32>` because:

- Different formats have different sample sizes (S16 = 2 bytes, F32 = 4 bytes)
- Keeps the type uniform regardless of format
- Consumer interprets based on `format` field

### Packet

Encoded data between source and decoder, or encoder and sink:

- `data: Vec<u8>` — compressed data
- `pts`, `dts` — presentation and decode timestamps (different for B-frames)
- `duration` — packet duration
- `time_base` — for timestamp conversion
- `is_keyframe` — important for seeking (can only seek to keyframes)
- `stream_type` — video or audio (for routing)

## Rational Numbers

FFmpeg uses rationals extensively for time bases and frame rates. We expose this as:

```
Rational { num: i32, den: i32 }
```

Common time bases:

- 1/90000 — MPEG transport streams
- 1/1000 — millisecond precision
- 1/48000 — audio at 48kHz (one tick per sample)

Common frame rates:

- 24000/1001 — 23.976 fps (film)
- 30000/1001 — 29.97 fps (NTSC)
- 25/1 — 25 fps (PAL)

The rational representation preserves precision that would be lost with floating point.

## What This Crate Does NOT Do

- No I/O operations
- No FFmpeg calls
- No threading or async
- No codec-specific logic
- No format conversion (that's transform)

It's purely types and simple computations (timestamp conversion, clock position tracking).
