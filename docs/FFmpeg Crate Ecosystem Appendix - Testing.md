# FFmpeg Crate Ecosystem Appendix - Testing

## The Testing Challenge

Media pipelines are notoriously difficult to test. A "correct" video is subjective — does that frame look right? Is that audio glitch a bug or a codec artifact? Did the seek land in the right place?

Full end-to-end testing of "open file, decode, transform, encode, mux, verify output plays correctly" is:

- Slow (processing real video takes time)
- Flaky (timing-dependent, hardware-dependent)
- Expensive (requires test media files)
- Inconclusive (what does "correct" even mean?)

Instead, we focus on testing the **atoms** — small, deterministic units with verifiable behavior.

## Testing Philosophy

We use standard Rust unit tests (`#[test]` functions, `cargo test`). No CI infrastructure, no test files, no external dependencies beyond what the crate already requires.

**The rule:** If a test requires a media file, we probably shouldn't write that test. Test behavior, not media processing.

## What to Test

### 1. Timestamp Arithmetic (ffmpeg-types)

Timestamp conversion is pure math with no FFmpeg dependency. Test exhaustively:

**What to verify:**

- PTS to Duration conversion with various time bases
- Round-trip conversion (PTS → Duration → PTS)
- Edge cases: zero, negative, very large values
- Common time bases: 1/90000, 1/1000, 1/48000

**Why testable:**

- Pure functions, no I/O
- Deterministic
- Clear expected outputs

**Example test cases:**

- PTS 90000 with time_base 1/90000 = 1 second
- PTS 48000 with time_base 1/48000 = 1 second
- PTS 0 = 0 seconds (any time base)
- Negative PTS handling (should produce Duration::ZERO or error)

### 2. Clock Behavior (ffmpeg-types)

Clock implementations are stateful but deterministic:

**AudioClock tests:**

- Initial position is zero
- Adding samples advances position correctly
- Position calculation respects sample rate and channels
- `mark_finished()` freezes audio position
- After finish, position extrapolates with wall time
- `reset_to()` sets position correctly and clears finished state

**WallClock tests:**

- Initial position is zero
- Position advances with wall time
- Pause stops advancement
- Resume continues from paused position
- Multiple pause/resume cycles accumulate correctly
- `reset_to()` adjusts base time

**Testing approach:**

- For sample-based position (AudioClock without wall time): fully deterministic, test exact values
- For wall time extrapolation: use small sleeps and tolerances, or test only the non-time-dependent parts
- Test state transitions explicitly

### 3. Packet/Frame Data Structures (ffmpeg-types)

Simple struct tests:

**What to verify:**

- Construction with various field values
- Field access
- Clone behavior (for types that implement Clone)
- Send/Sync trait bounds (compile-time check)

**Why test:**

- Catches accidental breaking changes
- Documents expected behavior
- Validates trait implementations

### 4. Error Type Conversions (ffmpeg-types)

Error handling infrastructure:

**What to verify:**

- `From` implementations work correctly
- Error messages are meaningful
- `Display` output is human-readable
- `std::error::Error` implementation

### 5. Rational Number Operations (ffmpeg-types)

If we implement arithmetic on Rational:

**What to verify:**

- Construction from num/den
- Reduction to lowest terms (if implemented)
- Conversion to/from f64
- Common values: 24000/1001, 30000/1001, 1/90000

### 6. Enum Completeness

For enums like PixelFormat, SampleFormat, CodecId:

**What to verify:**

- All variants can be constructed
- Debug/Display output is sensible
- Equality works correctly

## What NOT to Test

### FFmpeg Correctness

We don't test whether FFmpeg decodes H.264 correctly. That's FFmpeg's job. We trust it.

### Anything Requiring Media Files

No test should require:

- A video file
- An audio file
- Network access
- Any external resource

If we need to test "does decode work at all," that's manual verification during development, not an automated test.

### Full Pipeline Behavior

We don't test:

- Source → Decode → Transform → Encode → Sink
- "Does this video play correctly"
- "Does seeking work"

These require media files and are inherently integration tests.

### Visual/Audio Quality

"Does this look/sound right?" is untestable. We test that APIs return expected types and dimensions, not that the content is correct.

### Hardware Acceleration

Hardware depends on the machine. We don't write tests that only pass on certain hardware. If VideoToolbox isn't available, code should gracefully fall back — but we don't automate testing this.

### Timing-Dependent Behavior

Tests like "audio and video stay in sync" depend on system load and thread scheduling. We test clock arithmetic, which is deterministic. We don't test real-time playback behavior.

## Testing Techniques

### Compile-Time Trait Verification

Verify Send/Sync bounds at compile time:

```rust
fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn types_are_send_sync() {
    assert_send::<VideoFrame>();
    assert_send::<AudioFrame>();
    assert_send::<Packet>();
    assert_send::<AudioClock>();
    assert_sync::<AudioClock>();
}
```

These tests pass if they compile — no runtime logic needed.

### Synthetic Data

When we need frame/packet data for testing, construct it in the test:

```rust
#[test]
fn video_frame_dimensions() {
    let frame = VideoFrame {
        data: vec![0u8; 100 * 100 * 4], // 100x100 BGRA
        width: 100,
        height: 100,
        format: PixelFormat::Bgra,
        pts: Some(Pts(0)),
        time_base: Rational { num: 1, den: 1000 },
    };

    assert_eq!(frame.width, 100);
    assert_eq!(frame.height, 100);
    assert_eq!(frame.data.len(), 100 * 100 * 4);
}
```

No files needed — just construct the struct.

### Edge Case Enumeration

For numeric operations, explicitly test edge cases:

```rust
#[test]
fn pts_conversion_edge_cases() {
    // Zero
    assert_eq!(Pts(0).to_duration(tb_1_1000), Duration::ZERO);

    // Large values (near i64::MAX)
    let large = Pts(i64::MAX / 2);
    let _ = large.to_duration(tb_1_1000); // Should not panic

    // Negative (if we allow it)
    let neg = Pts(-100);
    assert_eq!(neg.to_duration(tb_1_1000), Duration::ZERO); // Or whatever we define
}
```

### State Machine Testing

For stateful types like clocks, test state transitions:

```rust
#[test]
fn audio_clock_state_transitions() {
    let clock = AudioClock::new(48000, 2);

    // Initial state
    assert_eq!(clock.position(), Duration::ZERO);

    // After adding samples
    clock.add_samples(48000 * 2); // 1 second of stereo
    assert_eq!(clock.position(), Duration::from_secs(1));

    // After reset
    clock.reset_to(Duration::from_secs(5));
    assert_eq!(clock.position(), Duration::from_secs(5));
}
```

### Tolerance for Time-Based Tests

When wall time is involved, use tolerances:

```rust
#[test]
fn wall_clock_advances() {
    let clock = WallClock::new();
    std::thread::sleep(Duration::from_millis(100));

    let pos = clock.position();
    // Allow 50ms tolerance for scheduling variance
    assert!(pos >= Duration::from_millis(50));
    assert!(pos <= Duration::from_millis(200));
}
```

Keep such tests minimal — they can be flaky on loaded systems.

## Test Organization

All tests live alongside the code in `#[cfg(test)]` modules:

```rust
// In ffmpeg-types/src/timestamp.rs

pub struct Pts(pub i64);

impl Pts {
    pub fn to_duration(&self, time_base: Rational) -> Duration {
        // implementation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pts_to_duration_basic() {
        // ...
    }

    #[test]
    fn pts_to_duration_edge_cases() {
        // ...
    }
}
```

Run with `cargo test`. No separate test directories, no test fixtures, no external files.

## What Each Crate Tests

### ffmpeg-types

Heavy testing here — it's all pure Rust with no FFmpeg dependency:

- Timestamp conversions (exhaustive)
- Clock behavior (AudioClock, WallClock)
- Rational arithmetic
- Data structure construction
- Error type conversions
- Trait implementations (Send, Sync, Clone, Debug)

### ffmpeg-source, ffmpeg-decode, ffmpeg-transform, ffmpeg-encode, ffmpeg-sink

Minimal testing — these wrap FFmpeg:

- Configuration struct construction
- Error type conversions
- Any pure-Rust helper functions

We don't test "does FFmpeg work" — we test our Rust code around FFmpeg.

## Summary

| Area                 | Test?   | How                             |
| -------------------- | ------- | ------------------------------- |
| Timestamp math       | Yes     | Unit tests with explicit values |
| Clock behavior       | Yes     | State transition tests          |
| Data structures      | Yes     | Construction, field access      |
| Trait bounds         | Yes     | Compile-time assertions         |
| Error types          | Yes     | From/Display implementations    |
| FFmpeg operations    | No      | Trust FFmpeg                    |
| Media files          | No      | No test files                   |
| Full pipeline        | No      | Not testable without files      |
| Visual/audio quality | No      | Subjective, untestable          |
| Hardware accel       | No      | Machine-dependent               |
| Timing behavior      | Minimal | Use tolerances, keep few        |

The goal: confidence that our pure-Rust code is correct. FFmpeg correctness is FFmpeg's responsibility.
