use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytemuck::cast_slice;

use ffmpeg_decode::{AudioDecoder, AudioDecoderConfig};
use ffmpeg_source::{Source, SourceConfig, StreamFilter};
use ffmpeg_transform::{AudioTransform, AudioTransformConfig};
use ffmpeg_types::{AudioClock, Clock, StreamType};

use crate::audio::{AudioStream, AudioStreamConsumer, AudioStreamProducer};
use crate::decode::PacketQueue;

const AUDIO_PACKET_QUEUE_CAPACITY: usize = 240;

struct AudioPipelineInner {
    demux_handle: Option<JoinHandle<()>>,
    decode_handle: Option<JoinHandle<()>>,
    consumer: Arc<AudioStreamConsumer>,
}

pub struct AudioPipeline {
    path: PathBuf,
    inner: Mutex<AudioPipelineInner>,
    stop_flag: Arc<AtomicBool>,
    packet_queue: Arc<PacketQueue>,
    clock: Arc<AudioClock>,
    producer: Arc<AudioStreamProducer>,
}

impl AudioPipeline {
    pub fn new(path: PathBuf) -> Option<Self> {
        Self::new_at(path, None)
    }

    fn new_at(path: PathBuf, start_position: Option<Duration>) -> Option<Self> {
        // Check if file has audio
        let path_str = path.to_str()?;
        let rt = tokio::runtime::Runtime::new().ok()?;
        let source = rt
            .block_on(Source::open(
                path_str,
                SourceConfig {
                    stream_filter: Some(StreamFilter::AudioOnly),
                    ..Default::default()
                },
            ))
            .ok()?;

        if !source.has_audio() {
            return None;
        }
        drop(source);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let packet_queue = Arc::new(PacketQueue::new(AUDIO_PACKET_QUEUE_CAPACITY));
        let clock = Arc::new(AudioClock::new(48000, 2));

        if let Some(pos) = start_position {
            clock.reset_to(pos);
        }

        let audio_stream = AudioStream::with_clock(Arc::clone(&clock));
        let producer = Arc::new(audio_stream.producer);
        let consumer = audio_stream.consumer;

        // Spawn demux thread
        let demux_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || {
                if let Err(e) = audio_demux(&path, packets, stop, start_position) {
                    eprintln!("[audio_demux] error: {}", e);
                }
            })
        };

        // Spawn decode thread
        let decode_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let prod = Arc::clone(&producer);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || {
                if let Err(e) = decode_audio_packets(&path, packets, &prod, stop) {
                    eprintln!("[audio_decode] error: {}", e);
                }
            })
        };

        Some(Self {
            path,
            inner: Mutex::new(AudioPipelineInner {
                demux_handle: Some(demux_handle),
                decode_handle: Some(decode_handle),
                consumer,
            }),
            stop_flag,
            packet_queue,
            clock,
            producer,
        })
    }

    pub fn seek_to(&self, position: Duration) {
        // Stop current threads
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();

        // Wait for threads to finish
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(handle) = inner.demux_handle.take() {
                let _ = handle.join();
            }
            if let Some(handle) = inner.decode_handle.take() {
                let _ = handle.join();
            }
            // Clear the ring buffer
            inner.consumer.clear();
        }

        // Reset state
        self.stop_flag.store(false, Ordering::Relaxed);
        self.packet_queue.reopen();
        self.clock.reset_to(position);

        // Reopen the producer so it can push again
        self.producer.reopen();

        // Get the existing producer - we need to keep using the same one
        let producer = Arc::clone(&self.producer);

        // Spawn new threads
        let demux_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || {
                if let Err(e) = audio_demux(&path, packets, stop, Some(position)) {
                    eprintln!("[audio_demux] error: {}", e);
                }
            })
        };

        let decode_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let prod = Arc::clone(&producer);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || {
                if let Err(e) = decode_audio_packets(&path, packets, &prod, stop) {
                    eprintln!("[audio_decode] error: {}", e);
                }
            })
        };

        // Store new thread handles
        {
            let mut inner = self.inner.lock().unwrap();
            inner.demux_handle = Some(demux_handle);
            inner.decode_handle = Some(decode_handle);
        }
    }

    pub fn clock(&self) -> &Arc<AudioClock> {
        &self.clock
    }

    pub fn consumer(&self) -> Arc<AudioStreamConsumer> {
        self.inner.lock().unwrap().consumer.clone()
    }

    pub fn set_volume(&self, volume: f32) {
        self.inner.lock().unwrap().consumer.set_volume(volume);
    }

    pub fn volume(&self) -> f32 {
        self.inner.lock().unwrap().consumer.volume()
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();

        let mut inner = self.inner.lock().unwrap();
        inner.consumer.mark_closed();

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

fn audio_demux(
    path: &Path,
    packets: Arc<PacketQueue>,
    stop_flag: Arc<AtomicBool>,
    start_position: Option<Duration>,
) -> Result<(), ffmpeg_types::Error> {
    let path_str = path
        .to_str()
        .ok_or_else(|| ffmpeg_types::Error::codec("Invalid path"))?;
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e: std::io::Error| ffmpeg_types::Error::codec(e.to_string()))?;
    let mut source = rt.block_on(Source::open(
        path_str,
        SourceConfig {
            stream_filter: Some(StreamFilter::AudioOnly),
            ..Default::default()
        },
    ))?;

    if let Some(pos) = start_position {
        // seek() returns the actual position, but for audio we don't need
        // it since the video pipeline determines the actual seek position
        let _ = source.seek(pos)?;
    }

    for result in &mut source {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let packet = result?;
        if packet.stream_type == StreamType::Audio && !packets.push(packet) {
            break;
        }
    }

    packets.close();
    Ok(())
}

fn decode_audio_packets(
    path: &Path,
    packets: Arc<PacketQueue>,
    producer: &AudioStreamProducer,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), ffmpeg_types::Error> {
    // Open source to get codec config
    let path_str = path
        .to_str()
        .ok_or_else(|| ffmpeg_types::Error::codec("Invalid path"))?;
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e: std::io::Error| ffmpeg_types::Error::codec(e.to_string()))?;
    let mut source = rt.block_on(Source::open(
        path_str,
        SourceConfig {
            stream_filter: Some(StreamFilter::AudioOnly),
            ..Default::default()
        },
    ))?;

    let codec_config = source
        .take_audio_codec_config()
        .ok_or_else(|| ffmpeg_types::Error::codec("No audio codec config"))?;
    let time_base = source
        .audio_time_base()
        .ok_or_else(|| ffmpeg_types::Error::codec("No audio time base"))?;
    drop(source);

    let mut decoder = AudioDecoder::new(codec_config, time_base, AudioDecoderConfig::new())?;
    let mut transform = AudioTransform::new(AudioTransformConfig::playback());

    while let Some(packet) = packets.pop() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let frames = match decoder.decode(&packet) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for frame in frames {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            let transformed = match transform.transform(&frame) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let samples: &[f32] = cast_slice(&transformed.data);
            if !producer.push(samples) {
                return Ok(());
            }
        }
    }

    // Flush decoder
    let remaining = decoder.flush().unwrap_or_default();
    for frame in remaining {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        let transformed = match transform.transform(&frame) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let samples: &[f32] = cast_slice(&transformed.data);
        if !producer.push(samples) {
            break;
        }
    }

    // Flush transform
    if let Ok(Some(final_frame)) = transform.flush() {
        let samples: &[f32] = cast_slice(&final_frame.data);
        let _ = producer.push(samples);
    }

    producer.close();
    Ok(())
}
