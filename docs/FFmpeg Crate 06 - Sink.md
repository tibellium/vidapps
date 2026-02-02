# FFmpeg Crate 06 - Sink

## Purpose

`ffmpeg-sink` handles the output side of the media pipeline. It takes encoded packets from the encoder and writes them into container formats — MP4 files, MKV files, HLS segments, etc.

This is the "muxer" — the inverse of demuxing. It combines encoded streams into a playable container.

## Why Muxing Matters

Encoded packets aren't playable on their own. They need:

- Container structure (atoms, boxes, elements)
- Stream headers (codec parameters)
- Timing information (timestamps, duration)
- Index structures (for seeking)

A muxer provides all of this, producing files that players can open and play.

## Container Formats

### MP4

The most widely compatible container:

- Works on virtually all devices and browsers
- Supports H.264, H.265, AAC, and more
- Good seeking support via moov atom index

Structure:

- `ftyp`: File type identifier
- `moov`: Metadata, codec params, sample tables
- `mdat`: Actual audio/video data

Quirk: `moov` atom traditionally comes after `mdat`, requiring a second pass to write. "Fast start" moves `moov` to the beginning for streaming.

### MKV (Matroska)

Open, flexible container:

- Supports virtually any codec
- Good for archival (no licensing concerns)
- Excellent metadata support
- Variable frame rate support

Less compatible than MP4 with consumer devices, but more capable.

### HLS (HTTP Live Streaming)

Not a single file but a collection:

- Playlist files (`.m3u8`): Index of segments
- Segment files (`.ts` or `.m4s`): Actual media

Structure:

```
playlist.m3u8
segment_0.ts
segment_1.ts
segment_2.ts
...
```

The playlist describes:

- Segment URLs and durations
- Total duration
- Whether stream is live or VOD
- Variant streams (different qualities)

HLS is the standard for video streaming on the web.

### MPEG-TS

Transport stream format:

- Designed for broadcast (error resilience)
- Self-contained packets (can start playback mid-stream)
- Used as HLS segment format

Good for streaming, less good for archival (no seeking index without external metadata).

## Multi-Stream Muxing

### Interleaving

Containers must interleave audio and video packets properly:

- Packets ordered by DTS (decode timestamp)
- Audio and video interleaved for smooth playback
- Buffer requirements minimized

The muxer handles interleaving automatically — consumer just calls `write()` for each packet.

### Stream Registration

Before writing packets, tell the muxer what streams to expect:

```
SinkConfig {
    format: ContainerFormat::Mp4,
    video: Some(video_stream_info),
    audio: Some(audio_stream_info),
}
```

Stream info comes from the encoder's `stream_info()` method.

### Timestamp Requirements

Packets must have valid timestamps:

- PTS (presentation timestamp): When to display
- DTS (decode timestamp): When to decode (≤ PTS)
- Duration: How long this packet's content lasts

The muxer uses these to:

- Order packets correctly
- Build seeking index
- Calculate total duration

## File Output

### Basic File Writing

```
let sink = file_sink("output.mp4", config)?;
for packet in packets {
    sink.write(&packet)?;
}
sink.finish()?;
```

The `finish()` call is critical — it:

- Flushes buffers
- Writes trailing metadata (duration, index)
- Closes the file properly

Without `finish()`, the file may be corrupt or incomplete.

### Seeking Index

MP4 and MKV build seeking indexes:

- Map timestamps to byte positions
- Enable random access playback
- Written at `finish()` time (needs complete stream info)

For MP4 "fast start" (moov at beginning), we either:

- Buffer entire file in memory (small files)
- Write to temp file, then reorder (large files)
- Use fragmented MP4 (no reordering needed)

## HLS Output

### Segment Generation

HLS sink produces multiple files:

- Master playlist
- Media segments (fixed duration, typically 2-10 seconds)

Configuration:

```
ContainerFormat::Hls {
    segment_duration: Duration::from_secs(4),
}
```

Each segment:

- Starts with a keyframe (essential for seeking)
- Contains ~4 seconds of media
- Is independently playable

### Playlist Management

The sink maintains the playlist:

- Adds segment entries as they're created
- Updates duration totals
- Writes playlist atomically (temp file → rename)

For live streaming, the playlist uses a sliding window (older segments removed).

### VOD vs Live

**VOD (Video on Demand)**

- Complete playlist with all segments listed
- `#EXT-X-ENDLIST` tag marks completion
- Seeking to any position possible

**Live**

- Playlist updated as new segments created
- Only recent segments listed (sliding window)
- No `#EXT-X-ENDLIST` until stream ends

We support VOD initially; live streaming adds complexity (continuous playlist updates).

## Finalization

### Why finish() Matters

Container formats have trailing structures:

- MP4: moov atom (or must be relocated)
- MKV: Cues element for seeking
- HLS: `#EXT-X-ENDLIST` in playlist

Without proper finalization:

- Duration may be unknown to players
- Seeking may not work
- Some players won't open the file at all

### Consuming Self

`finish(self)` consumes the sink because:

- Ensures finish is called exactly once
- Prevents writing after finalization
- Makes the ownership model clear

If you need to abort without finishing, drop the sink. Some formats handle this gracefully (segments already written); others may produce corrupt output.

## Error Handling

### Write Errors

Common write errors:

- Disk full: `Io` error
- Permission denied: `Io` error
- Invalid packet (wrong stream, bad timestamps): `InvalidData`

Consumer decides whether to retry or abort.

### Partial Writes

If errors occur mid-write:

- Some data may be written
- File may be partially valid
- Finalization won't succeed

For critical applications, consider writing to temp files and renaming on success.

## Thread Safety

### Single Writer

Sinks are `Send` but not `Sync`:

- Can move to another thread
- Cannot share between threads
- Single-threaded access only

This matches the typical pattern: one thread produces packets, one thread writes.

### Async Considerations

File I/O is typically fast enough to be synchronous. For network destinations (future feature), async would make sense.

We provide sync API initially. Async can be added for network sinks.

## What This Crate Does NOT Do

- **No encoding**: Expects encoded packets
- **No transcoding**: Doesn't change codec, just containers
- **No streaming server**: Writes files, doesn't serve them
- **No DRM**: No encryption or content protection
- **No complex authoring**: No chapters, multiple audio tracks, etc.

## Relationship to vidwall

vidwall is a playback application — it doesn't produce output files. The sink crate enables:

- Recording playback to file
- Transcoding to different formats
- Generating HLS for streaming

These are complementary features for a broader media toolkit.

## Future Considerations

### Fragmented MP4

fMP4 enables streaming without rewriting:

- Each fragment is independently playable
- No moov relocation needed
- Works for live streaming

This is important for HLS with fMP4 segments (more efficient than TS).

### Network Destinations

Writing directly to HTTP (PUT/POST) or cloud storage:

- Would need async I/O
- Upload progress reporting
- Retry logic for failures

Out of scope initially, but the architecture allows adding network sinks later.

### Adaptive Bitrate

Generating multiple quality levels:

- Requires parallel encode pipelines
- Sink would handle multi-variant playlist
- Significant complexity

This is application-level orchestration, not sink responsibility. The sink just writes what it's given.
