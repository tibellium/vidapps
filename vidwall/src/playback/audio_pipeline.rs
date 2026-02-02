use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::{
    AudioStreamClock, AudioStreamConsumer, AudioStreamProducer, create_audio_stream,
    create_audio_stream_with_clock,
};
use crate::decode::{
    AudioStreamInfo, DecoderError, PacketQueue, audio_demux, decode_audio_packets,
    get_audio_stream_info,
};

const AUDIO_PACKET_QUEUE_CAPACITY: usize = 240;

/**
    Mutable state that needs to be updated during seeking
*/
struct AudioPipelineInner {
    demux_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    decode_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    producer: Arc<AudioStreamProducer>,
    consumer: Arc<AudioStreamConsumer>,
}

/**
    Completely self-contained audio pipeline.
    Owns its own file handle, demux thread, decode thread, and output buffer.

    CRITICAL: This pipeline is completely independent from VideoPipeline.
    Blocking in this pipeline cannot affect video, and vice versa.
    The only shared state with video is the AudioStreamClock, which video
    reads (never writes) to know when to present frames.
*/
pub struct AudioPipeline {
    // Immutable config
    path: PathBuf,
    stream_info: AudioStreamInfo,

    // Mutable state behind Mutex for seeking
    inner: Mutex<AudioPipelineInner>,

    // Shared control (atomic, no lock needed)
    stop_flag: Arc<AtomicBool>,
    packet_queue: Arc<PacketQueue>,

    // Clock is shared and internally synchronized
    clock: Arc<AudioStreamClock>,
}

impl AudioPipeline {
    /**
        Create and start a new audio pipeline for the given file.
        Returns Ok(None) if the file has no audio stream.
        Returns Err if there's an error opening or processing the file.
    */
    pub fn new(path: PathBuf) -> Result<Option<Self>, DecoderError> {
        Self::new_at(path, None)
    }

    /**
        Create and start a new audio pipeline, optionally starting at a specific position.
    */
    fn new_at(
        path: PathBuf,
        start_position: Option<Duration>,
    ) -> Result<Option<Self>, DecoderError> {
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

        // If starting at a position, reset the clock
        if let Some(pos) = start_position {
            clock.reset_to(pos);
        }

        // Spawn demux thread (opens its own file handle)
        let demux_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || audio_demux(path, packets, stop, start_position))
        };

        // Spawn decode thread
        let decode_handle = {
            let packets = Arc::clone(&packet_queue);
            let prod = Arc::clone(&producer);
            let params = stream_info.codec_params.clone();
            let tb = stream_info.time_base;
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || decode_audio_packets(packets, prod, params, tb, stop))
        };

        Ok(Some(Self {
            path,
            stream_info,
            inner: Mutex::new(AudioPipelineInner {
                demux_handle: Some(demux_handle),
                decode_handle: Some(decode_handle),
                producer,
                consumer,
            }),
            stop_flag,
            packet_queue,
            clock,
        }))
    }

    /**
        Seek to a new position.
        Stops current threads, clears buffers, and restarts from the new position.
        Returns the new consumer (caller must update mixer).

        Note: Volume, mute, and pause state are preserved from the old consumer.
    */
    pub fn seek_to(&self, position: Duration) -> Result<Arc<AudioStreamConsumer>, DecoderError> {
        // 1. Signal threads to stop and capture old consumer state
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();

        // Capture state from old consumer before we replace it
        let (old_volume, old_muted, old_paused) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.consumer.volume(),
                inner.consumer.is_muted(),
                inner.consumer.is_paused(),
            )
        };

        // 2. Wait for threads to finish and get mutable access
        {
            let mut inner = self.inner.lock().unwrap();

            // Close producer to unblock decode thread
            inner.producer.close();

            if let Some(handle) = inner.demux_handle.take() {
                let _ = handle.join();
            }
            if let Some(handle) = inner.decode_handle.take() {
                let _ = handle.join();
            }
        }

        // 3. Reset state for new playback
        self.stop_flag.store(false, Ordering::Relaxed);
        self.packet_queue.reopen();
        self.clock.reset_to(position);

        // 4. Create fresh producer/consumer (keeps same clock)
        let (new_producer, new_consumer) = create_audio_stream_with_clock(Arc::clone(&self.clock));
        let new_producer = Arc::new(new_producer);
        let new_consumer = Arc::new(new_consumer);

        // 5. Restore state to new consumer
        new_consumer.set_volume(old_volume);
        if old_muted {
            new_consumer.mute();
        }
        if old_paused {
            new_consumer.pause();
        }

        // 6. Spawn new threads at position
        let demux_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || audio_demux(path, packets, stop, Some(position)))
        };

        let decode_handle = {
            let packets = Arc::clone(&self.packet_queue);
            let prod = Arc::clone(&new_producer);
            let params = self.stream_info.codec_params.clone();
            let tb = self.stream_info.time_base;
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || decode_audio_packets(packets, prod, params, tb, stop))
        };

        // 7. Store new state
        {
            let mut inner = self.inner.lock().unwrap();
            inner.demux_handle = Some(demux_handle);
            inner.decode_handle = Some(decode_handle);
            inner.producer = new_producer;
            inner.consumer = Arc::clone(&new_consumer);
        }

        Ok(new_consumer)
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
    pub fn consumer(&self) -> Arc<AudioStreamConsumer> {
        self.inner.lock().unwrap().consumer.clone()
    }

    /**
        Set the volume for this audio stream (0.0 to 1.0).
    */
    pub fn set_volume(&self, volume: f32) {
        self.inner.lock().unwrap().consumer.set_volume(volume);
    }

    /**
        Get the current volume (0.0 to 1.0).
    */
    pub fn volume(&self) -> f32 {
        self.inner.lock().unwrap().consumer.volume()
    }

    /**
        Stop the pipeline and wait for threads to finish.
    */
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();

        let mut inner = self.inner.lock().unwrap();
        inner.producer.close();

        if let Some(handle) = inner.demux_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = inner.decode_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}
