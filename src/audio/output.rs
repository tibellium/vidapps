use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, Stream, StreamConfig};

use super::mixer::AudioMixer;

/// Default sample rate for audio output
pub const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Default number of channels (stereo)
pub const DEFAULT_CHANNELS: u16 = 2;

/// Default buffer size in samples (per channel)
pub const DEFAULT_BUFFER_SIZE: u32 = 1024;

/**
    Error type for audio output operations
*/
#[derive(Debug)]
pub enum AudioError {
    NoDevice,
    DeviceError(String),
    StreamError(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "No audio output device found"),
            AudioError::DeviceError(e) => write!(f, "Audio device error: {}", e),
            AudioError::StreamError(e) => write!(f, "Audio stream error: {}", e),
        }
    }
}

impl std::error::Error for AudioError {}

/**
    Audio output device manager using cpal.
    Manages the audio stream and calls the mixer to fill buffers.
*/
pub struct AudioOutput {
    _stream: Stream,
}

impl AudioOutput {
    /**
        Create a new audio output with the given mixer.
        Starts playback immediately.
    */
    pub fn new(mixer: Arc<AudioMixer>) -> Result<Self, AudioError> {
        Self::with_config(
            mixer,
            DEFAULT_SAMPLE_RATE,
            DEFAULT_CHANNELS,
            DEFAULT_BUFFER_SIZE,
        )
    }

    /**
        Create a new audio output with custom configuration.
    */
    pub fn with_config(
        mixer: Arc<AudioMixer>,
        sample_rate: u32,
        channels: u16,
        buffer_size: u32,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();

        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;

        eprintln!("Audio device: {}", device.name().unwrap_or_default());

        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(sample_rate),
            buffer_size: BufferSize::Fixed(buffer_size),
        };

        use std::sync::atomic::{AtomicU64, Ordering};
        static CALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Fill the buffer directly - mixer uses lock-free reads
                    mixer.fill_buffer(data);

                    let count = CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
                    if count % 1000 == 0 {
                        eprintln!("[audio] callback #{}, buffer size: {}", count, data.len());
                    }
                },
                |err| {
                    eprintln!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| AudioError::StreamError(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioError::StreamError(e.to_string()))?;

        Ok(Self { _stream: stream })
    }
}
