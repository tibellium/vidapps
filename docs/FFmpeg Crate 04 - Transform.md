# FFmpeg Crate 04 - Transform

## Purpose

`ffmpeg-transform` converts frames between formats. For video: scaling, pixel format conversion. For audio: resampling, channel layout conversion, sample format conversion.

This is the "adapter" layer that makes decoded frames usable by downstream consumers.

## Why Transform is Necessary

### Decoder Output is Unpredictable

Decoders output frames in whatever format the codec specifies:

- H.264 might output YUV420P, NV12, or YUV420P10
- VP9 might output 8-bit or 10-bit YUV
- AAC outputs samples in the codec's native format

Consumers need predictable formats:

- Display rendering typically wants BGRA or RGBA
- Audio output APIs want F32 or S16 at specific sample rates
- Encoders have specific input format requirements

Transform bridges this gap.

### Dimension Matching

Video frames may need resizing:

- Display at different resolution than source
- Encoding at different resolution (transcoding)
- Thumbnail generation

Resizing is computationally significant and affects quality. The algorithm matters.

## Video Transformation

### Scaling Algorithms

We support several algorithms with different quality/speed trade-offs:

**Nearest Neighbor**

- Fastest, lowest quality
- Just picks nearest source pixel
- Use for: pixel art, debugging, maximum speed

**Bilinear**

- Fast, acceptable quality
- Interpolates between 4 nearest pixels
- Use for: real-time playback, preview

**Bicubic**

- Moderate speed, good quality
- Uses 16 pixels with cubic interpolation
- Use for: final output, when quality matters

**Lanczos**

- Slowest, highest quality
- Sophisticated windowed sinc filter
- Use for: archival, professional work

The default is bilinear — good balance for real-time playback.

### Pixel Format Conversion

Common conversions:

**YUV → RGB**
The fundamental conversion for display. YUV separates luminance (Y) from chrominance (U, V), which is how video is typically encoded. RGB is what displays show.

This involves:

- Matrix multiplication (BT.601, BT.709, or BT.2020 depending on source)
- Chroma upsampling (4:2:0 → 4:4:4)
- Range expansion (limited range to full range)

**NV12 → BGRA**
NV12 is a semi-planar format common from hardware decoders. BGRA is what macOS/Windows display APIs prefer.

**10-bit → 8-bit**
HDR content uses 10-bit depth. SDR displays need 8-bit. This involves tone mapping and bit depth reduction.

### Stride Handling

Video frames have stride (bytes per row) that may differ from width × bytes_per_pixel due to alignment requirements. Transform handles this:

- Input: respects source stride
- Output: can produce tightly-packed or aligned data

## Audio Transformation

### Sample Rate Conversion

Changing sample rate (e.g., 44100 Hz → 48000 Hz) is mathematically complex:

- Can't just drop/duplicate samples (causes aliasing/artifacts)
- Need proper resampling filter
- FFmpeg's swresample handles this well

Common conversions:

- 44100 → 48000 (CD to standard audio API rate)
- 48000 → 44100 (video audio to CD)
- Arbitrary rates (some codecs use unusual rates)

### Channel Layout Conversion

Converting between channel layouts:

- Mono → Stereo: duplicate channel
- Stereo → Mono: mix channels
- 5.1 → Stereo: downmix with proper coefficients

We support mono and stereo initially. Surround sound can be added later.

### Sample Format Conversion

Different representations of the same audio:

- **S16** (signed 16-bit integer): Compact, common for output
- **S32** (signed 32-bit integer): More headroom
- **F32** (32-bit float): Most flexible, good for processing

F32 is the best intermediate format — no clipping during processing.

## Lazy Initialization

### Why Lazy

Transform contexts (swscale, swresample) are expensive to create but cheap to use. We don't want to create them until we know the input format.

The pattern:

1. Create `VideoTransform` with target format
2. First call to `transform()` creates the context based on input format
3. Subsequent calls reuse the context
4. If input format changes, recreate context

### Format Change Detection

Input format might change mid-stream (rare but possible):

- Adaptive streaming switches quality levels
- Some containers allow format changes

We detect changes and reinitialize:

- Compare input dimensions, format to cached values
- If different, create new context
- Log the change (useful for debugging)

## Stateless vs Stateful

### Video Transform is Stateless

Each frame transforms independently:

- No buffering (one in, one out)
- No temporal filtering
- Can process frames in any order

This simplifies usage — just call `transform()` for each frame.

### Audio Transform Has State

Audio resampling maintains state:

- Filter history from previous samples
- Fractional sample position

This means:

- Frames should be processed in order
- `flush()` needed at end to get remaining samples
- `reset()` needed after seek to clear state

## Performance Considerations

### SIMD Optimization

FFmpeg's swscale and swresample use SIMD (SSE, AVX, NEON) for performance. We get this for free by using FFmpeg.

### Allocation Strategy

Each transform allocates output buffer. Options:

1. **Allocate new Vec each time**: Simple, current approach
2. **Reuse output buffer**: Pass buffer to transform, avoid allocation
3. **Pool buffers**: Maintain pool of reusable buffers

We start with option 1 for simplicity. Optimize if profiling shows allocation is a bottleneck.

### In-Place Transform

Some transforms could theoretically work in-place (same buffer for input and output). FFmpeg doesn't support this, and it's error-prone. We always use separate buffers.

## What We Don't Do

### No Filter Graphs

FFmpeg's libavfilter supports complex filter graphs:

- Multiple inputs/outputs
- Chained effects
- Conditional processing

We explicitly don't support this:

- Complexity explosion
- Different programming model (graph construction vs function calls)
- Most use cases only need simple transforms

If someone needs filter graphs, they can use libavfilter directly.

### No Effects

We don't implement:

- Color correction
- Sharpening/blurring
- Deinterlacing
- Noise reduction

These are valid needs, but out of scope. They could be a separate crate built on libavfilter.

### No Mixing

Combining multiple audio streams into one is mixing, not transform. That's application-level logic or a separate crate.

## Error Handling

### Invalid Dimensions

Zero width/height, or dimensions the scaler can't handle, produce errors. We validate before creating contexts.

### Unsupported Format Combinations

Not all format combinations are supported by swscale/swresample. We surface these as `UnsupportedFormat` errors.

### Resource Exhaustion

Extremely large dimensions could exhaust memory. We don't impose arbitrary limits, but very large transforms will fail if memory is insufficient.

## Relationship to vidwall

The vidwall code creates scalers/resamplers inline in the decode functions:

```rust
// In decode_video_packets()
let scaler = ScalerContext::get(
    src_format, src_width, src_height,
    Pixel::BGRA, dst_width, dst_height,
    ScalerFlags::BILINEAR,
)?;
scaler.run(&sw_frame, &mut bgra_frame)?;
```

This works but conflates decode and transform. The crate design separates them:

```rust
let frame = decoder.decode(&packet)?;
let bgra_frame = transform.transform(&frame)?;
```

The separation makes each component testable and reusable independently.
