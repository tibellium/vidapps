use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

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
pub struct AudioStreamClock {
    /// Total number of samples consumed (interleaved, so L+R = 2 samples)
    samples_consumed: AtomicU64,
    sample_rate: u32,
    channels: u16,
}

impl AudioStreamClock {
    /// Create a new audio clock with the given sample rate and channel count
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            samples_consumed: AtomicU64::new(0),
            sample_rate,
            channels,
        }
    }

    /// Get the current playback position as a Duration.
    /// This is the primary method for A/V sync - video should display
    /// frames whose PTS is <= this position.
    pub fn position(&self) -> Duration {
        let samples = self.samples_consumed.load(Ordering::Relaxed);
        // samples is interleaved (L,R,L,R...), so divide by channels to get audio frames
        let audio_frames = samples / self.channels as u64;
        // Convert audio frames to duration
        Duration::from_secs_f64(audio_frames as f64 / self.sample_rate as f64)
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
}

/// Producer half of the audio stream (used by decoder thread)
///
/// SAFETY: This is safe because ringbuf's HeapProd is designed to be used
/// from a single producer thread while a consumer operates on the other half.
/// The producer and consumer halves can operate independently without locking.
pub struct AudioStreamProducer {
    producer: UnsafeCell<ringbuf::HeapProd<f32>>,
    closed: AtomicBool,
}

// SAFETY: HeapProd is safe to send between threads.
// Only one thread should use the producer at a time (the decoder thread).
unsafe impl Send for AudioStreamProducer {}
unsafe impl Sync for AudioStreamProducer {}

impl AudioStreamProducer {
    /// Push samples to the ring buffer. Returns number of samples written.
    /// This is lock-free and will not block.
    pub fn push(&self, samples: &[f32]) -> usize {
        // SAFETY: Only one thread (decoder) calls push, and ringbuf's
        // producer is designed to work independently from consumer.
        unsafe { (*self.producer.get()).push_slice(samples) }
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
    closed: AtomicBool,
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
    ///
    /// The clock always advances by the full buffer size (representing real time),
    /// regardless of whether we had enough samples. This prevents deadlock when
    /// the audio buffer temporarily runs dry - video will keep advancing and
    /// consuming frames, allowing the decoder to produce more audio.
    ///
    /// Returns: Number of actual audio samples written (not silence)
    pub fn fill_buffer(&self, output: &mut [f32]) -> usize {
        let volume = self.volume();

        // SAFETY: Only one thread (audio callback) calls fill_buffer, and ringbuf's
        // consumer is designed to work independently from producer.
        let available = unsafe { (*self.consumer.get()).occupied_len() };
        let to_read = output.len().min(available);

        let samples_read = if to_read > 0 {
            // Read samples from ring buffer
            let read = unsafe { (*self.consumer.get()).pop_slice(&mut output[..to_read]) };

            // Apply volume to the samples we read
            for sample in &mut output[..read] {
                *sample *= volume;
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
            0
        };

        // Always advance clock by full buffer size - this represents real time passing.
        // Even if we output silence, time moves forward. This prevents deadlock:
        // if we only advance when we have samples, an empty buffer would stall
        // the video, which would stall the decoder, which couldn't produce more audio.
        self.clock.add_samples(output.len() as u64);

        samples_read
    }
}

/// Create a new audio stream with producer, consumer, and shared clock
pub fn create_audio_stream() -> (
    AudioStreamProducer,
    AudioStreamConsumer,
    Arc<AudioStreamClock>,
) {
    let rb = HeapRb::<f32>::new(RING_BUFFER_SIZE);
    let (producer, consumer) = rb.split();

    let clock = Arc::new(AudioStreamClock::new(DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS));

    (
        AudioStreamProducer {
            producer: UnsafeCell::new(producer),
            closed: AtomicBool::new(false),
        },
        AudioStreamConsumer {
            consumer: UnsafeCell::new(consumer),
            volume: AtomicF32::new(1.0),
            closed: AtomicBool::new(false),
            clock: Arc::clone(&clock),
        },
        clock,
    )
}
