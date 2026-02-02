mod output;
mod stream;

pub use output::{AudioError, AudioOutput, DEFAULT_CHANNELS, DEFAULT_SAMPLE_RATE};
pub use stream::{AudioStream, AudioStreamConsumer, AudioStreamProducer};
