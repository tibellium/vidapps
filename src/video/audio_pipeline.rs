use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

use crate::audio::{
    AudioStreamClock, AudioStreamConsumer, AudioStreamProducer, create_audio_stream,
};

use super::decoder::{
    AudioStreamInfo, DecoderError, audio_demux, decode_audio_packets, get_audio_stream_info,
};
use super::packet_queue::PacketQueue;

const AUDIO_PACKET_QUEUE_CAPACITY: usize = 240;

/**
    Completely self-contained audio pipeline.
    Owns its own file handle, demux thread, decode thread, and output buffer.

    CRITICAL: This pipeline is completely independent from VideoPipeline.
    Blocking in this pipeline cannot affect video, and vice versa.
    The only shared state with video is the AudioStreamClock, which video
    reads (never writes) to know when to present frames.
*/
pub struct AudioPipeline {
    // Threads
    demux_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    decode_handle: Option<JoinHandle<Result<(), DecoderError>>>,

    // Control
    stop_flag: Arc<AtomicBool>,
    packet_queue: Arc<PacketQueue>,

    // Output
    producer: Arc<AudioStreamProducer>,
    consumer: Arc<AudioStreamConsumer>,
    clock: Arc<AudioStreamClock>,
}

impl AudioPipeline {
    /**
        Create and start a new audio pipeline for the given file.
        Returns Ok(None) if the file has no audio stream.
        Returns Err if there's an error opening or processing the file.
    */
    pub fn new(path: PathBuf) -> Result<Option<Self>, DecoderError> {
        // Check if file has audio and get stream info
        let stream_info: AudioStreamInfo = match get_audio_stream_info(&path) {
            Ok(info) => info,
            Err(DecoderError::NoAudioStream) => return Ok(None),
            Err(e) => return Err(e),
        };

        let stop_flag = Arc::new(AtomicBool::new(false));
        let packet_queue = Arc::new(PacketQueue::new(AUDIO_PACKET_QUEUE_CAPACITY));

        // Create audio stream (producer, consumer, clock)
        let (producer, consumer, clock) = create_audio_stream();
        let producer = Arc::new(producer);
        let consumer = Arc::new(consumer);

        // Spawn demux thread (opens its own file handle)
        let demux_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || audio_demux(path, packets, stop))
        };

        // Spawn decode thread
        let decode_handle = {
            let packets = Arc::clone(&packet_queue);
            let prod = Arc::clone(&producer);
            let params = stream_info.codec_params;
            let tb = stream_info.time_base;
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || decode_audio_packets(packets, prod, params, tb, stop))
        };

        Ok(Some(Self {
            demux_handle: Some(demux_handle),
            decode_handle: Some(decode_handle),
            stop_flag,
            packet_queue,
            producer,
            consumer,
            clock,
        }))
    }

    /**
        Get the shared audio clock.
        Video pipeline reads this (READ-ONLY) to know when to present frames.
    */
    pub fn clock(&self) -> &Arc<AudioStreamClock> {
        &self.clock
    }

    /**
        Get the audio consumer for the mixer.
    */
    pub fn consumer(&self) -> &Arc<AudioStreamConsumer> {
        &self.consumer
    }

    /**
        Set the volume for this audio stream (0.0 to 1.0).
    */
    pub fn set_volume(&self, volume: f32) {
        self.consumer.set_volume(volume);
    }

    /**
        Get the current volume (0.0 to 1.0).
    */
    pub fn volume(&self) -> f32 {
        self.consumer.volume()
    }

    /**
        Stop the pipeline and wait for threads to finish.
    */
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();
        self.producer.close();

        if let Some(handle) = self.demux_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.decode_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}
