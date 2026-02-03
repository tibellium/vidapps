use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ffmpeg_decode::{VideoDecoder, VideoDecoderConfig};
use ffmpeg_source::{Source, SourceConfig, StreamFilter};
use ffmpeg_transform::{VideoTransform, VideoTransformConfig};
use ffmpeg_types::StreamType;

use crate::decode::PacketQueue;

use super::frame::VideoFrame;
use super::frame_queue::FrameQueue;

const VIDEO_PACKET_QUEUE_CAPACITY: usize = 120;
const VIDEO_FRAME_QUEUE_CAPACITY: usize = 60;

struct VideoPipelineInner {
    demux_handle: Option<JoinHandle<()>>,
    decode_handle: Option<JoinHandle<()>>,
}

pub struct VideoPipeline {
    path: PathBuf,
    inner: Mutex<VideoPipelineInner>,
    stop_flag: Arc<AtomicBool>,
    packet_queue: Arc<PacketQueue>,
    frame_queue: Arc<FrameQueue>,
    width: u32,
    height: u32,
}

impl VideoPipeline {
    pub fn new(path: PathBuf) -> Result<Self, ffmpeg_types::Error> {
        Self::new_at(path, None)
    }

    fn new_at(
        path: PathBuf,
        start_position: Option<Duration>,
    ) -> Result<Self, ffmpeg_types::Error> {
        // Probe to get video dimensions
        let path_str = path
            .to_str()
            .ok_or_else(|| ffmpeg_types::Error::codec("Invalid path"))?;
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e: std::io::Error| ffmpeg_types::Error::codec(e.to_string()))?;
        let info = rt.block_on(ffmpeg_source::probe(path_str))?;
        let video_info = info
            .video
            .ok_or_else(|| ffmpeg_types::Error::codec("No video stream"))?;
        let width = video_info.width;
        let height = video_info.height;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let packet_queue = Arc::new(PacketQueue::new(VIDEO_PACKET_QUEUE_CAPACITY));
        let frame_queue = Arc::new(FrameQueue::new(VIDEO_FRAME_QUEUE_CAPACITY));

        // Spawn demux thread
        let demux_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || {
                if let Err(e) = video_demux(&path, packets, stop, start_position, None) {
                    eprintln!("[video_demux] error: {}", e);
                }
            })
        };

        // Spawn decode thread
        let decode_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let frames = Arc::clone(&frame_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || {
                if let Err(e) = decode_video_packets(&path, packets, frames, stop) {
                    eprintln!("[video_decode] error: {}", e);
                }
            })
        };

        Ok(Self {
            path,
            inner: Mutex::new(VideoPipelineInner {
                demux_handle: Some(demux_handle),
                decode_handle: Some(decode_handle),
            }),
            stop_flag,
            packet_queue,
            frame_queue,
            width,
            height,
        })
    }

    pub fn frame_queue(&self) -> &Arc<FrameQueue> {
        &self.frame_queue
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /**
        Seek to a position in the video.

        Returns the actual position that was seeked to (nearest keyframe),
        which may be before the requested position.
    */
    pub fn seek_to(&self, position: Duration) -> Result<Duration, ffmpeg_types::Error> {
        // Stop threads
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();
        self.frame_queue.close();

        // Wait for threads
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(handle) = inner.demux_handle.take() {
                let _ = handle.join();
            }
            if let Some(handle) = inner.decode_handle.take() {
                let _ = handle.join();
            }
        }

        // Reset state
        self.stop_flag.store(false, Ordering::Relaxed);
        self.packet_queue.reopen();
        self.frame_queue.reopen();

        // Channel to receive actual position from demux thread
        let (position_tx, position_rx) = mpsc::channel();

        // Spawn new threads
        let demux_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || {
                if let Err(e) = video_demux(&path, packets, stop, Some(position), Some(position_tx))
                {
                    eprintln!("[video_demux] error: {}", e);
                }
            })
        };

        let decode_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let frames = Arc::clone(&self.frame_queue);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || {
                if let Err(e) = decode_video_packets(&path, packets, frames, stop) {
                    eprintln!("[video_decode] error: {}", e);
                }
            })
        };

        // Store handles
        {
            let mut inner = self.inner.lock().unwrap();
            inner.demux_handle = Some(demux_handle);
            inner.decode_handle = Some(decode_handle);
        }

        // Wait for actual position from demux thread (with timeout)
        let actual_position = position_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or(position);

        Ok(actual_position)
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();
        self.frame_queue.close();

        let mut inner = self.inner.lock().unwrap();
        if let Some(handle) = inner.demux_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = inner.decode_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

fn video_demux(
    path: &Path,
    packets: Arc<PacketQueue>,
    stop_flag: Arc<AtomicBool>,
    start_position: Option<Duration>,
    position_tx: Option<mpsc::Sender<Duration>>,
) -> Result<(), ffmpeg_types::Error> {
    let path_str = path
        .to_str()
        .ok_or_else(|| ffmpeg_types::Error::codec("Invalid path"))?;
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e: std::io::Error| ffmpeg_types::Error::codec(e.to_string()))?;
    let mut source = rt.block_on(Source::open(
        path_str,
        SourceConfig {
            stream_filter: Some(StreamFilter::VideoOnly),
            ..Default::default()
        },
    ))?;

    if let Some(pos) = start_position {
        let actual_position = source.seek(pos)?;
        // Send actual position back to caller
        if let Some(tx) = position_tx {
            let _ = tx.send(actual_position);
        }
    }

    for result in &mut source {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let packet = result?;
        if packet.stream_type == StreamType::Video && !packets.push(packet) {
            break;
        }
    }

    packets.close();
    Ok(())
}

fn decode_video_packets(
    path: &Path,
    packets: Arc<PacketQueue>,
    frames: Arc<FrameQueue>,
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
            stream_filter: Some(StreamFilter::VideoOnly),
            ..Default::default()
        },
    ))?;

    let codec_config = source
        .take_video_codec_config()
        .ok_or_else(|| ffmpeg_types::Error::codec("No video codec config"))?;
    let time_base = source
        .video_time_base()
        .ok_or_else(|| ffmpeg_types::Error::codec("No video time base"))?;

    // Get dimensions for transform
    let info = source.media_info();
    let video_info = info
        .video
        .as_ref()
        .ok_or_else(|| ffmpeg_types::Error::codec("No video info"))?;
    let width = video_info.width;
    let height = video_info.height;
    drop(source);

    let mut decoder =
        VideoDecoder::new(codec_config, time_base, VideoDecoderConfig::with_hw_accel())?;

    // Transform to BGRA for display
    let mut transform = VideoTransform::new(VideoTransformConfig::to_bgra(width, height));

    while let Some(packet) = packets.pop() {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        let decoded_frames = match decoder.decode(&packet) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[video_decode] decode error: {}", e);
                continue;
            }
        };
        for frame in decoded_frames {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let bgra_frame = match transform.transform(&frame) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("[video_decode] transform error: {}", e);
                    continue;
                }
            };
            let pts = bgra_frame.presentation_time().unwrap_or(Duration::ZERO);

            let video_frame =
                VideoFrame::new(bgra_frame.data, bgra_frame.width, bgra_frame.height, pts);

            if !frames.push(video_frame) {
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

        let bgra_frame = match transform.transform(&frame) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let pts = bgra_frame.presentation_time().unwrap_or(Duration::ZERO);

        let video_frame =
            VideoFrame::new(bgra_frame.data, bgra_frame.width, bgra_frame.height, pts);

        if !frames.push(video_frame) {
            break;
        }
    }

    frames.close();
    Ok(())
}
