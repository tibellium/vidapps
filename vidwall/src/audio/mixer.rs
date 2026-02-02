use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use parking_lot::RwLock;

use super::stream::{AtomicF32, AudioStreamConsumer};

/**
    Maximum number of audio streams the mixer supports
*/
pub const MIXER_MAX_STREAMS: usize = 4;

/**
    Pre-allocated buffer size for mixing
*/
const MIX_BUFFER_SIZE: usize = 4096;

/**
    Audio mixer that combines multiple audio streams into a single output.
    Supports per-stream volume (via AudioStreamConsumer), master volume, and master mute.

    Designed for real-time audio: uses RwLock with try_read to avoid blocking.
*/
pub struct AudioMixer {
    streams: RwLock<Vec<Option<Arc<AudioStreamConsumer>>>>,
    master_volume: AtomicF32,
    master_muted: AtomicBool,
    sample_rate: u32,
    channels: u16,
}

impl AudioMixer {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            streams: RwLock::new(Vec::new()),
            master_volume: AtomicF32::new(1.0),
            master_muted: AtomicBool::new(false),
            sample_rate,
            channels,
        }
    }

    /**
        Get the output sample rate
    */
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /**
        Get the number of output channels
    */
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /**
        Get the master volume (0.0 to 1.0)
    */
    pub fn master_volume(&self) -> f32 {
        self.master_volume.load(Ordering::Relaxed)
    }

    /**
        Set the master volume (0.0 to 1.0)
    */
    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume
            .store(volume.clamp(0.0, 1.0), Ordering::Relaxed);
    }

    /**
        Mute all audio output
    */
    pub fn mute(&self) {
        self.master_muted.store(true, Ordering::Relaxed);
    }

    /**
        Unmute audio output
    */
    pub fn unmute(&self) {
        self.master_muted.store(false, Ordering::Relaxed);
    }

    /**
        Toggle master mute state. Returns the new muted state.
    */
    pub fn toggle_mute(&self) -> bool {
        let was_muted = self.master_muted.load(Ordering::Relaxed);
        self.master_muted.store(!was_muted, Ordering::Relaxed);
        !was_muted
    }

    /**
        Check if master audio is muted
    */
    pub fn is_muted(&self) -> bool {
        self.master_muted.load(Ordering::Relaxed)
    }

    /**
        Set a stream at the given index. Uses write lock.
        Automatically grows the streams vector if needed.
    */
    pub fn set_stream(&self, index: usize, stream: Option<Arc<AudioStreamConsumer>>) {
        if index >= MIXER_MAX_STREAMS {
            return;
        }
        let mut streams = self.streams.write();
        // Grow vector if needed
        while streams.len() <= index {
            streams.push(None);
        }
        streams[index] = stream;
    }

    /**
        Get a clone of a stream at the given index
    */
    pub fn stream(&self, index: usize) -> Option<Arc<AudioStreamConsumer>> {
        let streams = self.streams.read();
        streams.get(index).cloned().flatten()
    }

    /**
        Clear all streams (used when reconfiguring the grid)
    */
    pub fn clear_streams(&self) {
        let mut streams = self.streams.write();
        streams.clear();
    }

    /**
        Get the current number of stream slots
    */
    pub fn stream_count(&self) -> usize {
        self.streams.read().len()
    }

    /**
        Fill the output buffer by mixing all active streams.
        This is called by the audio output callback on a real-time thread.

        Uses try_read to avoid blocking - outputs silence if lock unavailable.

        output: Interleaved stereo buffer to fill
    */
    pub fn fill_buffer(&self, output: &mut [f32]) {
        // If master muted, output silence (but still consume samples from streams)
        let is_muted = self.master_muted.load(Ordering::Relaxed);
        let master_vol = self.master_volume();

        // Clear output buffer first
        for sample in output.iter_mut() {
            *sample = 0.0;
        }

        // Try to get read lock - if unavailable, we already have silence
        let Some(streams) = self.streams.try_read() else {
            return;
        };

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

        // Apply master volume and clamp to prevent clipping (or silence if muted)
        if is_muted {
            for sample in output.iter_mut() {
                *sample = 0.0;
            }
        } else {
            for sample in output.iter_mut() {
                *sample = (*sample * master_vol).clamp(-1.0, 1.0);
            }
        }
    }
}
