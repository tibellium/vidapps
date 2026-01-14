# Handoff Document: Complete Pipeline Rearchitecture Required

## The Application

`vidwall` - A 2x2 video grid player for macOS. Plays 4 random videos simultaneously with audio mixing. Built with Rust, GPUI, FFmpeg, and cpal.

## The Problem

The application freezes after approximately 2100 frames are decoded by ANY single video. Multiple attempted fixes have failed because they addressed symptoms, not the root cause.

## Why All Fixes Have Failed

The current architecture is fundamentally broken. Every attempted fix has failed because **audio and video pipelines are coupled through shared threads and queues**.

### Current Architecture (BROKEN)

```
                    ┌─────────────────┐
                    │  Single Demux   │
                    │     Thread      │
                    └────────┬────────┘
                             │
              ┌──────────────┴──────────────┐
              │                             │
              ▼                             ▼
     ┌────────────────┐            ┌────────────────┐
     │ Video Packet   │            │ Audio Packet   │
     │    Queue       │            │    Queue       │
     └───────┬────────┘            └───────┬────────┘
             │                             │
             ▼                             ▼
     ┌────────────────┐            ┌────────────────┐
     │ Video Decode   │            │ Audio Decode   │
     │    Thread      │            │    Thread      │
     └───────┬────────┘            └───────┬────────┘
             │                             │
             ▼                             ▼
     ┌────────────────┐            ┌────────────────┐
     │  Frame Queue   │            │  Ring Buffer   │
     └───────┬────────┘            └───────┬────────┘
             │                             │
             ▼                             ▼
     ┌────────────────┐            ┌────────────────┐
     │ Video Player   │◄───clock───│ Audio Callback │
     │   (UI thread)  │            │ (cpal thread)  │
     └────────────────┘            └────────────────┘
```

### Why This Deadlocks

The single demux thread is the fatal flaw. Here's the deadlock chain:

1. Frame queue fills up (60 frames)
2. Video decode thread blocks on `frames.push()`
3. Video packet queue fills up (120 packets)
4. Demux thread blocks on `video_packets.push()`
5. **Demux cannot push audio packets while blocked on video**
6. Audio packet queue drains
7. Audio decode thread has no packets → ring buffer empties
8. Audio callback outputs silence → **clock stops advancing**
9. Video player waits for `clock.position() >= frame.pts` → frames never consumed
10. Frame queue stays full forever
11. **DEADLOCK**

### Why "Fixes" Failed

1. **Non-blocking video push with packet dropping**: Dropped packets cause decoder errors and visual corruption.

2. **Non-blocking video push with local buffering**: The local buffer grows unbounded, and audio still blocks eventually because audio decode blocks on ring buffer push.

3. **Larger queues**: Just delays the deadlock; doesn't fix it.

The fundamental issue: **ANY coupling between audio and video paths will eventually cause deadlock** because:

- Audio consumes at fixed realtime rate (48kHz)
- Video/audio decode produce at 10x+ realtime
- When any shared resource blocks, it can starve the other path

## Required Architecture: Complete Separation

The ONLY way to fix this is **complete pipeline separation**. Audio and video must be entirely independent except for ONE thing: video reads the audio clock to know when to present frames.

### New Architecture (CORRECT)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           AUDIO PIPELINE (independent)                       │
│                                                                             │
│  ┌──────────┐    ┌─────────┐    ┌──────────┐    ┌──────────┐    ┌────────┐ │
│  │  File    │───▶│  Audio  │───▶│  Audio   │───▶│   Ring   │───▶│  cpal  │ │
│  │ (handle) │    │  Demux  │    │  Decode  │    │  Buffer  │    │callback│ │
│  └──────────┘    └─────────┘    └──────────┘    └──────────┘    └───┬────┘ │
│                                                                      │      │
│                                                              updates clock  │
└──────────────────────────────────────────────────────────────────────┼──────┘
                                                                       │
                                                                       ▼
                                                              ┌────────────────┐
                                                              │  AudioClock    │
                                                              │ (AtomicU64)    │
                                                              └────────┬───────┘
                                                                       │
                                                                 reads clock
                                                                       │
┌──────────────────────────────────────────────────────────────────────┼──────┐
│                           VIDEO PIPELINE (independent)               │      │
│                                                                      ▼      │
│  ┌──────────┐    ┌─────────┐    ┌──────────┐    ┌──────────┐    ┌────────┐ │
│  │  File    │───▶│  Video  │───▶│  Video   │───▶│  Frame   │───▶│ Video  │ │
│  │ (handle) │    │  Demux  │    │  Decode  │    │  Queue   │    │ Player │ │
│  └──────────┘    └─────────┘    └──────────┘    └──────────┘    └────────┘ │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Principles

1. **Separate file handles**: Each pipeline opens the video file independently. FFmpeg can handle this.

2. **Separate demux threads**: Audio demux reads ONLY audio packets (ignores video). Video demux reads ONLY video packets (ignores audio).

3. **NO shared queues**: Audio pipeline has its own queues. Video pipeline has its own queues. They never interact.

4. **Clock is the ONLY shared state**:
    - Audio callback WRITES to clock (advances it as samples are consumed)
    - Video player READS from clock (to know when to show next frame)
    - Clock is a simple `AtomicU64` - lock-free, no blocking possible

5. **Each pipeline has independent backpressure**:
    - Audio: ring buffer full → audio decode waits → packet queue fills → audio demux waits. Audio callback drains at 48kHz, so this self-regulates.
    - Video: frame queue full → video decode waits → packet queue fills → video demux waits. Video player drains based on clock, so this self-regulates.

### The Clock Interface

```rust
/// The ONLY shared state between audio and video pipelines.
/// Audio writes, video reads. That's it.
pub struct AudioClock {
    samples_consumed: AtomicU64,
    sample_rate: u32,
    channels: u16,
}

impl AudioClock {
    /// Called ONLY by audio callback after consuming samples
    pub fn advance(&self, samples: u64) {
        self.samples_consumed.fetch_add(samples, Ordering::Release);
    }

    /// Called by video player to get current playback position
    pub fn position(&self) -> Duration {
        let samples = self.samples_consumed.load(Ordering::Acquire);
        let frames = samples / self.channels as u64;
        Duration::from_secs_f64(frames as f64 / self.sample_rate as f64)
    }
}
```

### Video Frame Presentation

The video player should work like this:

```rust
fn present_frame(&mut self) {
    let clock_pos = self.clock.position();

    // Skip frames if we're behind
    while let Some(frame) = self.peek_next_frame() {
        if frame.pts <= clock_pos {
            self.current_frame = self.pop_frame();
        } else {
            break; // Next frame is in the future, wait
        }
    }

    // Render current_frame (may be same as last time if clock hasn't advanced)
}
```

This is a pure "pull" model:

- Video player pulls frames when clock says it's time
- If video is behind, it skips frames to catch up
- If video is ahead, it just keeps showing current frame
- **Video production is completely decoupled from video presentation**

### Handling Videos Without Audio

For videos without an audio track, use wall clock:

```rust
pub enum PlaybackClock {
    Audio(Arc<AudioClock>),
    WallTime { start: Instant },
}

impl PlaybackClock {
    pub fn position(&self) -> Duration {
        match self {
            PlaybackClock::Audio(clock) => clock.position(),
            PlaybackClock::WallTime { start } => start.elapsed(),
        }
    }
}
```

## Implementation Plan

### Phase 1: Create Independent Audio Pipeline

Create a new `AudioPipeline` struct that owns:

- Its own file handle (via FFmpeg)
- Audio demux thread (reads only audio packets)
- Audio decode thread
- Ring buffer
- Reference to shared `AudioClock`

The audio callback updates the clock.

### Phase 2: Create Independent Video Pipeline

Create a new `VideoPipeline` struct that owns:

- Its own file handle (via FFmpeg)
- Video demux thread (reads only video packets)
- Video decode thread
- Frame queue

### Phase 3: Create VideoPlayer with Clock Reference

The `VideoPlayer` takes:

- A `VideoPipeline`
- A `PlaybackClock` (either `Audio(Arc<AudioClock>)` or `WallTime`)

It pulls frames from the pipeline based on clock position.

### Phase 4: Wire Up in Main

For each video:

1. Create `AudioClock` (if video has audio)
2. Create `AudioPipeline` with reference to clock
3. Create `VideoPipeline`
4. Create `VideoPlayer` with pipeline and clock
5. Register audio consumers with mixer

### Phase 5: Remove Old Code

Delete the old coupled architecture:

- Old `demux()` function
- Old `VideoPlayer` implementation
- Any shared queue logic

## Key Files to Modify/Create

### New Files

- `src/audio/pipeline.rs` - AudioPipeline struct
- `src/audio/clock.rs` - AudioClock struct
- `src/video/pipeline.rs` - VideoPipeline struct

### Files to Heavily Modify

- `src/video/player.rs` - Rewrite to use clock-based presentation
- `src/video/decoder.rs` - Split into audio_demux() and video_demux()
- `src/main.rs` - Wire up new architecture

### Files That Stay Similar

- `src/audio/mixer.rs` - Still mixes 4 audio streams
- `src/audio/output.rs` - Still manages cpal
- `src/video/queue.rs` - Frame queue stays the same
- `src/video/packet_queue.rs` - Packet queue stays the same

## Critical Constraints

1. **NO shared threads between audio and video** - Ever. Period.

2. **NO shared queues between audio and video** - Each pipeline owns its queues.

3. **Clock is READ-ONLY for video** - Video never writes to clock.

4. **Blocking is OK within a pipeline** - Audio decode can block on ring buffer. Video decode can block on frame queue. This is fine because each pipeline self-regulates.

5. **Blocking must NEVER cross pipelines** - If anything in audio pipeline can block something in video pipeline (or vice versa), you've failed.

## Testing the Fix

1. Run with 4 videos simultaneously
2. Verify playback continues past 2100 frames (the previous freeze point)
3. Verify A/V sync is maintained
4. Verify no audio dropouts or video stuttering under normal conditions
5. Test with videos of varying bitrates and resolutions

## Summary

The current architecture is unfixable because audio and video are coupled through a shared demux thread. The solution is complete separation: independent pipelines that share ONLY a clock. The clock is the synchronization mechanism - audio produces it, video consumes it. Nothing else is shared.

**DO NOT attempt incremental fixes. Rearchitect from scratch.**
