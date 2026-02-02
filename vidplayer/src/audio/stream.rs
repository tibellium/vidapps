use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use ffmpeg_types::AudioClock;
use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};

struct AtomicF32 {
    inner: AtomicU32,
}

impl AtomicF32 {
    fn new(value: f32) -> Self {
        Self {
            inner: AtomicU32::new(value.to_bits()),
        }
    }

    fn load(&self, ordering: Ordering) -> f32 {
        f32::from_bits(self.inner.load(ordering))
    }

    fn store(&self, value: f32, ordering: Ordering) {
        self.inner.store(value.to_bits(), ordering);
    }
}

const RING_BUFFER_SIZE: usize = 48000 * 2 * 2; // ~2 seconds stereo at 48kHz

pub struct AudioStreamProducer {
    producer: UnsafeCell<ringbuf::HeapProd<f32>>,
    closed: Arc<AtomicBool>,
}

unsafe impl Send for AudioStreamProducer {}
unsafe impl Sync for AudioStreamProducer {}

impl AudioStreamProducer {
    pub fn push(&self, samples: &[f32]) -> bool {
        let mut offset = 0;
        while offset < samples.len() {
            if self.closed.load(Ordering::Acquire) {
                return false;
            }

            let written = unsafe { (*self.producer.get()).push_slice(&samples[offset..]) };
            offset += written;

            if offset < samples.len() {
                thread::sleep(Duration::from_micros(500));
            }
        }
        true
    }

    pub fn available(&self) -> usize {
        unsafe { (*self.producer.get()).vacant_len() }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

pub struct AudioStreamConsumer {
    consumer: UnsafeCell<ringbuf::HeapCons<f32>>,
    volume: AtomicF32,
    closed: Arc<AtomicBool>,
    paused: AtomicBool,
    muted: AtomicBool,
    clock: Arc<AudioClock>,
}

unsafe impl Send for AudioStreamConsumer {}
unsafe impl Sync for AudioStreamConsumer {}

impl AudioStreamConsumer {
    pub fn clock(&self) -> &Arc<AudioClock> {
        &self.clock
    }

    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.clamp(0.0, 1.0), Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn mute(&self) {
        self.muted.store(true, Ordering::Relaxed);
    }

    pub fn unmute(&self) {
        self.muted.store(false, Ordering::Relaxed);
    }

    pub fn toggle_mute(&self) -> bool {
        let was_muted = self.muted.load(Ordering::Relaxed);
        self.muted.store(!was_muted, Ordering::Relaxed);
        !was_muted
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn is_ended(&self) -> bool {
        unsafe { self.closed.load(Ordering::Acquire) && (*self.consumer.get()).is_empty() }
    }

    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub fn available(&self) -> usize {
        unsafe { (*self.consumer.get()).occupied_len() }
    }

    pub fn fill_buffer(&self, output: &mut [f32]) -> usize {
        if self.paused.load(Ordering::Relaxed) {
            for sample in output.iter_mut() {
                *sample = 0.0;
            }
            return 0;
        }

        let is_muted = self.muted.load(Ordering::Relaxed);
        let volume = self.volume();

        let available = unsafe { (*self.consumer.get()).occupied_len() };
        let to_read = output.len().min(available);

        if to_read > 0 {
            let read = unsafe { (*self.consumer.get()).pop_slice(&mut output[..to_read]) };

            self.clock.add_samples(read as u64);

            if is_muted {
                for sample in &mut output[..read] {
                    *sample = 0.0;
                }
            } else {
                for sample in &mut output[..read] {
                    *sample *= volume;
                }
            }

            for sample in &mut output[read..] {
                *sample = 0.0;
            }

            read
        } else {
            for sample in output.iter_mut() {
                *sample = 0.0;
            }

            if self.closed.load(Ordering::Acquire) {
                self.clock.mark_finished();
            }

            0
        }
    }

    pub fn clear(&self) {
        unsafe {
            let consumer = &mut *self.consumer.get();
            // Clear by popping all available samples
            let available = consumer.occupied_len();
            consumer.skip(available);
        }
    }
}

pub struct AudioStream {
    pub producer: AudioStreamProducer,
    pub consumer: Arc<AudioStreamConsumer>,
    pub clock: Arc<AudioClock>,
}

impl AudioStream {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(AudioClock::new(48000, 2)))
    }

    pub fn with_clock(clock: Arc<AudioClock>) -> Self {
        let rb = HeapRb::<f32>::new(RING_BUFFER_SIZE);
        let (producer, consumer) = rb.split();

        let closed = Arc::new(AtomicBool::new(false));

        let producer = AudioStreamProducer {
            producer: UnsafeCell::new(producer),
            closed: Arc::clone(&closed),
        };

        let consumer = Arc::new(AudioStreamConsumer {
            consumer: UnsafeCell::new(consumer),
            volume: AtomicF32::new(1.0),
            closed,
            paused: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            clock: Arc::clone(&clock),
        });

        Self {
            producer,
            consumer,
            clock,
        }
    }
}

impl Default for AudioStream {
    fn default() -> Self {
        Self::new()
    }
}
