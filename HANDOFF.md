# Handoff Document: Audio/Video Pipeline Deadlock

## The Application
`vidwall` - A 2x2 video grid player for macOS. Plays 4 random videos simultaneously with audio mixing. Built with Rust, GPUI, FFmpeg, and cpal.

## The Problem
The application freezes after approximately 2100 total video frames are decoded (across all 4 videos). This happens consistently regardless of which videos are playing.

## Root Cause Analysis

The current architecture is **NOT a sound pipeline**. A proper pipeline looks like:

```
Producer -> Consumer/Producer -> Consumer/Producer -> Final Consumer
```

Each stage should naturally throttle through backpressure. If a downstream consumer is slow, upstream producers wait. This is self-regulating.

### Current Broken Architecture

```
Demux Thread ─┬─> Video Packet Queue ─> Video Decode Thread ─> Frame Queue ─> UI (via audio clock)
              │
              └─> Audio Packet Queue ─> Audio Decode Thread ─> Ring Buffer ─> cpal callback
```

The problem: **The final consumer (cpal audio callback) runs at a fixed rate (48kHz)**. But the pipeline upstream produces as fast as possible. There's no backpressure mechanism that properly connects the entire pipeline to this fixed-rate consumer.

### What Happens

1. Demux reads packets as fast as I/O allows
2. Video/Audio decode run as fast as CPU allows
3. Queues fill up (frame queue: 60, packet queues: 120/240, ring buffer: ~192k samples)
4. Eventually something blocks waiting for queue space
5. But the thing that should be draining (cpal callback at 48kHz) can't keep up with 4 simultaneous decode streams producing at 10x+ realtime
6. Deadlock

### The Fundamental Issue

The audio decode thread pushes to the ring buffer with a **blocking push** (added to fix A/V sync). When the ring buffer is full, it waits for space. But:

- Ring buffer drains at 48kHz (realtime)
- Audio decode produces at 10x+ realtime
- With 4 videos, the pressure is 4x worse

The blocking push was added because non-blocking push (dropping samples) caused A/V desync. But blocking push causes deadlock.

## What Needs to Be Fixed

The pipeline needs to be **rate-limited by the final consumer**. Options:

### Option 1: Clock-Driven Decode
Don't decode ahead freely. Only decode when the playback clock says it's time. The audio clock (which advances at realtime as samples are consumed) should gate when new packets are demuxed/decoded.

### Option 2: Proper Backpressure
Make the ring buffer the throttle point, but ensure it can actually apply backpressure without deadlock. This might mean:
- Larger buffers to absorb bursts
- The blocking should be interruptible/timeout properly
- Or use a different sync mechanism

### Option 3: PTS-Based Timing
Instead of tracking "samples consumed" for the audio clock, use PTS timestamps from packets. Then dropping samples doesn't break sync, and non-blocking push works.

## Key Files

- `src/video/decoder.rs` - demux(), decode_video_packets(), decode_audio_packets()
- `src/video/player.rs` - VideoPlayer, get_render_image(), audio clock usage
- `src/video/packet_queue.rs` - Bounded queue for packets
- `src/video/queue.rs` - Frame queue
- `src/audio/stream.rs` - Ring buffer producer/consumer, AudioStreamClock
- `src/audio/mixer.rs` - Mixes 4 audio streams
- `src/audio/output.rs` - cpal integration

## Debug Output Currently in Code

There's extensive debug output (eprintln!) scattered through the code:
- `[demux]` messages in decoder.rs
- `[video_decode]` messages in decoder.rs  
- `[audio_decode]` messages in decoder.rs
- `[audio_clock]` messages in stream.rs
- `[cpal]` messages in output.rs

## The User's Core Point

**"If we somehow get stuck in later pipeline steps because we demux or decode too fast, then we simply do not have a sound pipeline."**

The architecture needs to ensure that producing faster than consuming is impossible by design, not by hoping buffers are big enough.
