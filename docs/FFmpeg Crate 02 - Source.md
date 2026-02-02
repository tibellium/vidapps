# FFmpeg Crate 02 - Source

## Purpose

`ffmpeg-source` handles the input side of the media pipeline. It opens media from various sources (files, HTTP, HLS streams), parses containers, and produces encoded packets that downstream crates can decode.

Think of it as the "demuxer" — it understands container formats (MP4, MKV, MPEG-TS, HLS) and extracts the encoded streams within them.

## Why Separate Source from Decode?

The split reflects a fundamental separation in media processing:

**Demuxing** (source) deals with:

- Container formats (MP4 atoms, MKV elements, TS packets)
- Stream discovery (what streams exist, their properties)
- Timestamp management (converting container timestamps to stream timestamps)
- I/O (reading from files, networks)

**Decoding** deals with:

- Codec algorithms (H.264 macroblocks, AAC frequency bands)
- Reference frame management
- Hardware acceleration
- Pixel/sample format handling

These concerns are independent. You can demux an MP4 with FFmpeg and decode with a hardware decoder. You can demux an MKV and re-mux to MP4 without decoding at all.

## Supported Sources

### Local Files

The simplest case. FFmpeg's `avformat_open_input` handles:

- Auto-detection of container format
- Seeking via file I/O
- Memory-mapped I/O for performance

### HTTP/HTTPS

Standard HTTP streaming. FFmpeg handles:

- Range requests for seeking
- Chunked transfer encoding
- Basic authentication

We use `reqwest` rather than FFmpeg's built-in HTTP because:

- Better async integration with Rust ecosystem
- More control over connection pooling, timeouts
- Easier to add custom headers (auth tokens, etc.)

This means we implement a custom AVIOContext that reads from reqwest's response body.

### HLS (HTTP Live Streaming)

HLS is a playlist-based format:

1. Master playlist points to variant playlists (different qualities)
2. Variant playlist lists segment URLs
3. Segments are typically MPEG-TS or fMP4

FFmpeg has built-in HLS support, but we may want custom handling for:

- Adaptive bitrate switching based on bandwidth
- Prefetching upcoming segments
- Custom segment caching

Initially, we'll use FFmpeg's HLS demuxer. Custom handling can come later if needed.

### MPEG-TS

Transport streams are common for broadcast and HLS segments. FFmpeg handles:

- PID filtering (selecting specific streams)
- PCR-based timing
- Handling packet loss gracefully

## The Probe vs Open Distinction

### Probing

`probe()` opens the source just enough to read metadata:

- Container format
- Stream count and types
- Codec information
- Duration (if available)
- Resolution, frame rate, sample rate

This is lightweight — it doesn't start demuxing packets. Use it for:

- Displaying file info in UI
- Checking if a file is valid before committing to process it
- Selecting which quality variant to use for HLS

### Opening

`open()` prepares for full demuxing:

- Validates all streams
- Prepares codec parameters for decoder
- Positions at the start (or specified position)
- Ready to produce packets

The distinction matters for performance — probing 100 files to display a list should be fast.

## Packet Production

### Single Stream vs Multiple Streams

A media file typically contains multiple streams (video, audio, maybe subtitles). The source must decide how to expose them.

**Option A: Single packet iterator**

```
while let Some(packet) = source.next_packet() {
    match packet.stream_type {
        Video => video_decoder.decode(packet),
        Audio => audio_decoder.decode(packet),
    }
}
```

**Option B: Separate iterators**

```
let (video_packets, audio_packets) = source.split();
// Run in separate threads
```

We choose **Option A** because:

- Simpler implementation
- Consumer controls routing logic
- Works naturally with async (single stream of packets)
- Consumer can filter (skip audio if only video needed)

The consumer can spawn separate decode threads and route packets as needed (this is what vidwall does).

### Packet Contents

Each packet contains:

- Compressed data (ready for decoder)
- PTS/DTS timestamps (in stream time base)
- Duration
- Keyframe flag (important for seeking)
- Stream type (video/audio)

The source doesn't interpret the compressed data — that's the decoder's job.

## Seeking

### How Container Seeking Works

Containers have index structures that map timestamps to byte positions:

- MP4: `stss` (sync sample) and `stco` (chunk offset) atoms
- MKV: Cues element
- TS: Often no index, must scan

When you seek:

1. Find the nearest keyframe at or before the target time
2. Jump to that position in the file
3. Start demuxing from there

You can only seek to keyframes because inter-frames (P/B frames) depend on previous frames.

### Seek Precision

Seeking to 5.0 seconds might land you at 4.8 seconds (the nearest keyframe). The consumer must:

1. Accept that the first packet is before the target
2. Decode frames but discard until reaching the target time
3. Or accept the imprecision and start from the keyframe

This is why `seek()` doesn't guarantee exact positioning — it's a container limitation.

### Seeking in Network Streams

HTTP with range requests: Works like files (if server supports ranges)
HLS: Seek to appropriate segment, then within segment
Live streams: Often can't seek backward

## Async Design

### Why Async for Network

Network I/O is inherently async — waiting for data shouldn't block a thread. For HTTP and HLS sources:

- Request is async
- Reading response body is async
- Segment fetches (HLS) can be concurrent

### Sync for Files

Local file I/O is fast enough that async overhead isn't worth it. We provide:

- `open_sync()` / `next_packet_sync()` for files
- `open()` / `next_packet()` (async) for everything

The sync API is simpler when you don't need async.

### Integration with Tokio

We use `tokio` for the async runtime because:

- `reqwest` is tokio-based
- Most Rust async ecosystems have settled on tokio
- Excellent performance and tooling

Consumers using a different runtime can use `tokio::runtime::Runtime` to run our async code in a blocking context.

## Codec Parameters

### The Problem

Decoders need codec-specific initialization data:

- H.264: SPS/PPS (sequence/picture parameter sets)
- AAC: AudioSpecificConfig
- VP9: Codec features

This data is stored in the container, not in packets. The source extracts it.

### The Solution

We return an opaque `CodecConfig` from the source that the decoder accepts:

```
let source = open("video.mp4").await?;
let config = source.video_codec_config()?;
let decoder = video_decoder(config)?;
```

This hides FFmpeg's `AVCodecParameters` from the public API while preserving all necessary information.

Internally, `CodecConfig` may serialize the parameters or hold a reference — the consumer doesn't care.

## Error Handling

### Common Errors

- **File not found**: `Io` error
- **Invalid container**: `InvalidData` — file exists but isn't valid media
- **Unsupported format**: `UnsupportedFormat` — valid media but format we don't handle
- **Network timeout**: `Io` error (from reqwest)
- **End of stream**: `Eof` — not really an error, signals completion

### Partial Reads

Network streams may have transient errors. The source should:

- Retry on temporary failures (configurable)
- Surface persistent failures to consumer
- For HLS: Skip bad segments if possible, continue with next

## What This Crate Does NOT Do

- **No decoding**: Packets come out compressed
- **No format conversion**: That's `ffmpeg-transform`
- **No thread management**: Consumer decides threading model
- **No playback timing**: Consumer's clock handles that
- **No buffering policy**: Consumer decides how much to buffer

## Relationship to vidwall

The current vidwall code has `video_demux()` and `audio_demux()` functions that each open their own file handle. This was done to prevent deadlocks (separate threads with separate I/O).

In the crate design:

- Single `Source` produces all packets
- Consumer routes packets to appropriate decoders
- Consumer can still use separate threads — just route packets between them

The separate-file-handle pattern was a workaround for vidwall's specific architecture. The crate provides a cleaner single-source model that's more flexible.
