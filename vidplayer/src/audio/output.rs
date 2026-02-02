use std::sync::Arc;

use cpal::{
    BufferSize, SampleRate, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use super::stream::AudioStreamConsumer;

pub const DEFAULT_SAMPLE_RATE: u32 = 48000;
pub const DEFAULT_CHANNELS: u16 = 2;
pub const DEFAULT_BUFFER_SIZE: u32 = 1024;

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

pub struct AudioOutput {
    _stream: Stream,
}

impl AudioOutput {
    pub fn new(consumer: Arc<AudioStreamConsumer>) -> Result<Self, AudioError> {
        Self::with_config(
            consumer,
            DEFAULT_SAMPLE_RATE,
            DEFAULT_CHANNELS,
            DEFAULT_BUFFER_SIZE,
        )
    }

    pub fn with_config(
        consumer: Arc<AudioStreamConsumer>,
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

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumer.fill_buffer(data);
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
