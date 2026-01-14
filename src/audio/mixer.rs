use std::sync::Arc;
use std::sync::atomic::Ordering;

use parking_lot::RwLock;

use super::stream::{AtomicF32, AudioStreamConsumer};

/// Number of audio streams the mixer supports (one per video in the grid)
pub const MIXER_STREAM_COUNT: usize = 4;

/// Pre-allocated buffer size for mixing
const MIX_BUFFER_SIZE: usize = 4096;

/**
    Audio mixer that combines multiple audio streams into a single output.
    Supports per-stream volume (via AudioStreamConsumer) and master volume.

    Designed for real-time audio: uses RwLock with try_read to avoid blocking.
*/
pub struct AudioMixer {
    streams: RwLock<[Option<Arc<AudioStreamConsumer>>; MIXER_STREAM_COUNT]>,
    master_volume: AtomicF32,
    sample_rate: u32,
    channels: u16,
}

impl AudioMixer {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            streams: RwLock::new([None, None, None, None]),
            master_volume: AtomicF32::new(1.0),
            sample_rate,
            channels,
        }
    }

    /// Get the output sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the number of output channels
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Get the master volume (0.0 to 1.0)
    pub fn master_volume(&self) -> f32 {
        self.master_volume.load(Ordering::Relaxed)
    }

    /// Set the master volume (0.0 to 1.0)
    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume
            .store(volume.clamp(0.0, 1.0), Ordering::Relaxed);
    }

    /// Set a stream at the given index (0-3). Uses write lock.
    pub fn set_stream(&self, index: usize, stream: Option<Arc<AudioStreamConsumer>>) {
        if index < MIXER_STREAM_COUNT {
            let mut streams = self.streams.write();
            streams[index] = stream;
        }
    }

    /// Get a clone of a stream at the given index
    pub fn stream(&self, index: usize) -> Option<Arc<AudioStreamConsumer>> {
        if index < MIXER_STREAM_COUNT {
            let streams = self.streams.read();
            streams[index].clone()
        } else {
            None
        }
    }

    /**
        Fill the output buffer by mixing all active streams.
        This is called by the audio output callback on a real-time thread.

        Uses try_read to avoid blocking - outputs silence if lock unavailable.

        output: Interleaved stereo buffer to fill
    */
    pub fn fill_buffer(&self, output: &mut [f32]) {
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        static FILL_COUNT: AtomicU64 = AtomicU64::new(0);
        static LOCK_FAIL_COUNT: AtomicU64 = AtomicU64::new(0);

        let master_vol = self.master_volume();

        // Clear output buffer first
        for sample in output.iter_mut() {
            *sample = 0.0;
        }

        // Try to get read lock - if unavailable, we already have silence
        let Some(streams) = self.streams.try_read() else {
            let fails = LOCK_FAIL_COUNT.fetch_add(1, AtomicOrdering::Relaxed);
            if fails % 100 == 0 {
                eprintln!("[mixer] lock failed {} times", fails);
            }
            return;
        };

        let count = FILL_COUNT.fetch_add(1, AtomicOrdering::Relaxed);
        if count % 1000 == 0 {
            let stream_count = streams.iter().filter(|s| s.is_some()).count();
            eprintln!(
                "[mixer] fill #{}, {} streams registered",
                count, stream_count
            );
        }

        // Process in chunks to use stack-allocated buffer
        let mut stream_buffer = [0.0f32; MIX_BUFFER_SIZE];

        for chunk_start in (0..output.len()).step_by(MIX_BUFFER_SIZE) {
            let chunk_end = (chunk_start + MIX_BUFFER_SIZE).min(output.len());
            let chunk_len = chunk_end - chunk_start;
            let output_chunk = &mut output[chunk_start..chunk_end];
            let buffer_slice = &mut stream_buffer[..chunk_len];

            // Mix each stream into this chunk
            for stream_opt in streams.iter() {
                if let Some(stream) = stream_opt {
                    // Fill stream buffer (stream applies its own volume)
                    stream.fill_buffer(buffer_slice);

                    // Add to output
                    for (out, src) in output_chunk.iter_mut().zip(buffer_slice.iter()) {
                        *out += *src;
                    }

                    // Clear buffer for next stream
                    for sample in buffer_slice.iter_mut() {
                        *sample = 0.0;
                    }
                }
            }
        }

        // Apply master volume and clamp to prevent clipping
        for sample in output.iter_mut() {
            *sample = (*sample * master_vol).clamp(-1.0, 1.0);
        }
    }
}
