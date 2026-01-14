use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};

/// Atomic f32 wrapper for thread-safe volume control
pub struct AtomicF32 {
    inner: AtomicU32,
}

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self {
            inner: AtomicU32::new(value.to_bits()),
        }
    }

    pub fn load(&self, ordering: Ordering) -> f32 {
        f32::from_bits(self.inner.load(ordering))
    }

    pub fn store(&self, value: f32, ordering: Ordering) {
        self.inner.store(value.to_bits(), ordering);
    }
}

/// Default ring buffer size (~2 seconds of stereo audio at 48kHz)
const RING_BUFFER_SIZE: usize = 48000 * 2 * 2;

/// Default sample rate for audio position calculations
const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Default number of channels (stereo)
const DEFAULT_CHANNELS: u16 = 2;

/// Audio clock that tracks playback position based on samples consumed.
/// This is shared between the audio consumer and video player for A/V sync.
///
/// The clock is updated by the audio consumer as it consumes samples,
/// and can be read by any component that needs to know the current audio time.
///
/// When audio playback finishes (all samples consumed), the clock automatically
/// switches to wall-time-based extrapolation so video frames continue to advance.
pub struct AudioStreamClock {
    /// Total number of samples consumed (interleaved, so L+R = 2 samples)
    samples_consumed: AtomicU64,
    sample_rate: u32,
    channels: u16,
    /// When audio finishes, we record the position and wall time to extrapolate from
    finished_state: Mutex<Option<FinishedState>>,
}

/// State recorded when audio playback finishes
struct FinishedState {
    /// The audio position when playback finished
    position_at_finish: Duration,
    /// The wall time when playback finished
    wall_time_at_finish: Instant,
}

impl AudioStreamClock {
    /// Create a new audio clock with the given sample rate and channel count
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            samples_consumed: AtomicU64::new(0),
            sample_rate,
            channels,
            finished_state: Mutex::new(None),
        }
    }

    /// Get the current playback position as a Duration.
    /// This is the primary method for A/V sync - video should display
    /// frames whose PTS is <= this position.
    ///
    /// If audio has finished, this extrapolates using wall time from
    /// the point where audio ended, ensuring video continues to advance.
    pub fn position(&self) -> Duration {
        // Check if audio has finished - if so, extrapolate from wall time
        if let Some(ref finished) = *self.finished_state.lock() {
            let elapsed_since_finish = finished.wall_time_at_finish.elapsed();
            return finished.position_at_finish + elapsed_since_finish;
        }

        // Normal case: return position based on samples consumed
        let samples = self.samples_consumed.load(Ordering::Relaxed);
        // samples is interleaved (L,R,L,R...), so divide by channels to get audio frames
        let audio_frames = samples / self.channels as u64;
        // Convert audio frames to duration
        Duration::from_secs_f64(audio_frames as f64 / self.sample_rate as f64)
    }

    /// Mark the audio stream as finished.
    /// After this, position() will extrapolate using wall time.
    /// Called by AudioStreamConsumer when the ring buffer is empty and closed.
    pub(crate) fn mark_finished(&self) {
        let mut finished = self.finished_state.lock();
        if finished.is_none() {
            let current_position = {
                let samples = self.samples_consumed.load(Ordering::Relaxed);
                let audio_frames = samples / self.channels as u64;
                Duration::from_secs_f64(audio_frames as f64 / self.sample_rate as f64)
            };
            *finished = Some(FinishedState {
                position_at_finish: current_position,
                wall_time_at_finish: Instant::now(),
            });
        }
    }

    /// Get the total number of samples consumed (raw count)
    pub fn samples_consumed(&self) -> u64 {
        self.samples_consumed.load(Ordering::Relaxed)
    }

    /// Add to the consumed sample count. Called by AudioStreamConsumer.
    pub(crate) fn add_samples(&self, count: u64) {
        self.samples_consumed.fetch_add(count, Ordering::Relaxed);
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the channel count
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Reset the clock to a specific position (for seeking).
    /// This sets the samples_consumed to match the target position
    /// and clears any finished state.
    pub fn reset_to(&self, position: Duration) {
        // Calculate how many samples correspond to this position
        let audio_frames = (position.as_secs_f64() * self.sample_rate as f64) as u64;
        let samples = audio_frames * self.channels as u64;
        self.samples_consumed.store(samples, Ordering::Relaxed);
        // Clear finished state so clock resumes normal operation
        *self.finished_state.lock() = None;
    }
}

/// Producer half of the audio stream (used by decoder thread)
///
/// SAFETY: This is safe because ringbuf's HeapProd is designed to be used
/// from a single producer thread while a consumer operates on the other half.
/// The producer and consumer halves can operate independently without locking.
pub struct AudioStreamProducer {
    producer: UnsafeCell<ringbuf::HeapProd<f32>>,
    /// Shared with consumer to signal end of stream
    closed: Arc<AtomicBool>,
}

// SAFETY: HeapProd is safe to send between threads.
// Only one thread should use the producer at a time (the decoder thread).
unsafe impl Send for AudioStreamProducer {}
unsafe impl Sync for AudioStreamProducer {}

impl AudioStreamProducer {
    /// Push samples to the ring buffer, blocking if the buffer is full.
    /// Returns false if the producer was closed while waiting.
    pub fn push(&self, samples: &[f32]) -> bool {
        let mut offset = 0;
        while offset < samples.len() {
            if self.closed.load(Ordering::Acquire) {
                return false;
            }

            // SAFETY: Only one thread (decoder) calls push, and ringbuf's
            // producer is designed to work independently from consumer.
            let written = unsafe { (*self.producer.get()).push_slice(&samples[offset..]) };
            offset += written;

            if offset < samples.len() {
                // Buffer full, wait a bit for consumer to drain
                thread::sleep(Duration::from_micros(500));
            }
        }
        true
    }

    /// Check if there's space for more samples
    pub fn available(&self) -> usize {
        // SAFETY: vacant_len() only reads atomic state
        unsafe { (*self.producer.get()).vacant_len() }
    }

    /// Close the producer (signals end of stream)
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Check if closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// Consumer half of the audio stream (used by audio callback)
///
/// SAFETY: This is safe because ringbuf's HeapCons is designed to be used
/// from a single consumer thread while a producer operates on the other half.
pub struct AudioStreamConsumer {
    consumer: UnsafeCell<ringbuf::HeapCons<f32>>,
    volume: AtomicF32,
    /// Shared with producer - set when producer signals end of stream
    closed: Arc<AtomicBool>,
    /// When paused, fill_buffer outputs silence without consuming samples
    paused: AtomicBool,
    /// When muted, fill_buffer outputs silence but still consumes samples
    muted: AtomicBool,
    /// Shared clock for tracking playback position
    clock: Arc<AudioStreamClock>,
}

// SAFETY: HeapCons is safe to send between threads.
// Only one thread should use the consumer at a time (the audio callback thread).
unsafe impl Send for AudioStreamConsumer {}
unsafe impl Sync for AudioStreamConsumer {}

impl AudioStreamConsumer {
    /// Get a reference to the shared audio clock
    pub fn clock(&self) -> &Arc<AudioStreamClock> {
        &self.clock
    }

    /// Get current volume (0.0 to 1.0)
    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed)
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 1.0), Ordering::Relaxed);
    }

    /// Pause audio playback. When paused, fill_buffer outputs silence
    /// without consuming samples from the ring buffer.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    /// Resume audio playback after being paused.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    /// Check if audio is paused
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Mute audio output. When muted, fill_buffer outputs silence
    /// but still consumes samples (playback continues silently).
    pub fn mute(&self) {
        self.muted.store(true, Ordering::Relaxed);
    }

    /// Unmute audio output.
    pub fn unmute(&self) {
        self.muted.store(false, Ordering::Relaxed);
    }

    /// Toggle mute state. Returns the new muted state.
    pub fn toggle_mute(&self) -> bool {
        // Note: This isn't perfectly atomic but is fine for UI toggling
        let was_muted = self.muted.load(Ordering::Relaxed);
        self.muted.store(!was_muted, Ordering::Relaxed);
        !was_muted
    }

    /// Check if audio is muted
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    /// Check if the stream has ended
    pub fn is_ended(&self) -> bool {
        // SAFETY: is_empty() only reads atomic state
        unsafe { self.closed.load(Ordering::Acquire) && (*self.consumer.get()).is_empty() }
    }

    /// Mark as closed (called when producer signals end)
    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Check how many samples are available in the buffer
    pub fn available(&self) -> usize {
        // SAFETY: occupied_len() only reads atomic state
        unsafe { (*self.consumer.get()).occupied_len() }
    }

    /// Fill the output buffer with samples, applying volume.
    /// This is completely lock-free and safe for real-time audio.
    /// Updates the shared clock with the number of samples actually consumed.
    ///
    /// When paused, outputs silence without consuming samples.
    /// When muted, outputs silence but still consumes samples (playback continues).
    ///
    /// When the audio stream ends (closed and empty), marks the clock as finished
    /// so it can switch to wall-time extrapolation for remaining video frames.
    ///
    /// Returns: Number of actual audio samples written (not silence)
    pub fn fill_buffer(&self, output: &mut [f32]) -> usize {
        // If paused, output silence without consuming samples
        if self.paused.load(Ordering::Relaxed) {
            for sample in output.iter_mut() {
                *sample = 0.0;
            }
            return 0;
        }

        let is_muted = self.muted.load(Ordering::Relaxed);
        let volume = self.volume();

        // SAFETY: Only one thread (audio callback) calls fill_buffer, and ringbuf's
        // consumer is designed to work independently from producer.
        let available = unsafe { (*self.consumer.get()).occupied_len() };
        let to_read = output.len().min(available);

        if to_read > 0 {
            // Read samples from ring buffer
            let read = unsafe { (*self.consumer.get()).pop_slice(&mut output[..to_read]) };

            // Update the shared clock with samples consumed
            self.clock.add_samples(read as u64);

            // Apply volume (or silence if muted) to the samples we read
            if is_muted {
                for sample in &mut output[..read] {
                    *sample = 0.0;
                }
            } else {
                for sample in &mut output[..read] {
                    *sample *= volume;
                }
            }

            // Fill remaining with silence
            for sample in &mut output[read..] {
                *sample = 0.0;
            }

            read
        } else {
            // No samples available, output silence
            for sample in output.iter_mut() {
                *sample = 0.0;
            }

            // If stream is closed and empty, mark clock as finished
            // so video can continue using wall time
            if self.closed.load(Ordering::Acquire) {
                self.clock.mark_finished();
            }

            0
        }
    }
}

/// Create a new audio stream with producer, consumer, and shared clock
pub fn create_audio_stream() -> (
    AudioStreamProducer,
    AudioStreamConsumer,
    Arc<AudioStreamClock>,
) {
    let clock = Arc::new(AudioStreamClock::new(DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS));
    let (producer, consumer) = create_audio_stream_with_clock(Arc::clone(&clock));
    (producer, consumer, clock)
}

/// Create a new audio stream using an existing clock.
/// Used for seeking - we create fresh producer/consumer but keep the same clock
/// so the VideoPlayer's PlaybackClock reference remains valid.
pub fn create_audio_stream_with_clock(
    clock: Arc<AudioStreamClock>,
) -> (AudioStreamProducer, AudioStreamConsumer) {
    let rb = HeapRb::<f32>::new(RING_BUFFER_SIZE);
    let (producer, consumer) = rb.split();

    // Shared closed flag so consumer knows when producer is done
    let closed = Arc::new(AtomicBool::new(false));

    (
        AudioStreamProducer {
            producer: UnsafeCell::new(producer),
            closed: Arc::clone(&closed),
        },
        AudioStreamConsumer {
            consumer: UnsafeCell::new(consumer),
            volume: AtomicF32::new(1.0),
            closed,
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            clock,
        },
    )
}
