use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::decode::{
    DecoderError, PacketQueue, VideoStreamInfo, decode_video_packets, get_video_stream_info,
    video_demux,
};

use super::frame_queue::FrameQueue;

const VIDEO_PACKET_QUEUE_CAPACITY: usize = 120;
const VIDEO_FRAME_QUEUE_CAPACITY: usize = 60;

/**
    Internal mutable state for seeking support.
*/
struct VideoPipelineInner {
    demux_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    decode_handle: Option<JoinHandle<Result<(), DecoderError>>>,
}

/**
    Completely self-contained video pipeline.
    Owns its own file handle, demux thread, decode thread, and frame queue.

    CRITICAL: This pipeline is completely independent from AudioPipeline.
    Blocking in this pipeline cannot affect audio, and vice versa.
*/
pub struct VideoPipeline {
    // Configuration (immutable)
    path: PathBuf,
    stream_info: VideoStreamInfo,
    target_width: Option<u32>,
    target_height: Option<u32>,

    // Thread handles behind mutex for seeking
    inner: Mutex<VideoPipelineInner>,

    // Control
    stop_flag: Arc<AtomicBool>,
    packet_queue: Arc<PacketQueue>,

    // Output
    frame_queue: Arc<FrameQueue>,
}

impl VideoPipeline {
    /**
        Create and start a new video pipeline for the given file.
    */
    pub fn new(
        path: PathBuf,
        target_width: Option<u32>,
        target_height: Option<u32>,
    ) -> Result<Self, DecoderError> {
        let stream_info = get_video_stream_info(&path)?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let packet_queue = Arc::new(PacketQueue::new(VIDEO_PACKET_QUEUE_CAPACITY));
        let frame_queue = Arc::new(FrameQueue::new(VIDEO_FRAME_QUEUE_CAPACITY));

        // Spawn demux thread (opens its own file handle)
        let demux_handle = {
            let path = path.clone();
            let packets = Arc::clone(&packet_queue);
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || video_demux(path, packets, stop, None))
        };

        // Spawn decode thread
        let decode_handle = {
            let packets = Arc::clone(&packet_queue);
            let frames = Arc::clone(&frame_queue);
            let params = stream_info.codec_params.clone();
            let tb = stream_info.time_base;
            let stop = Arc::clone(&stop_flag);
            thread::spawn(move || {
                decode_video_packets(
                    packets,
                    frames,
                    params,
                    tb,
                    stop,
                    target_width,
                    target_height,
                )
            })
        };

        Ok(Self {
            path,
            stream_info,
            target_width,
            target_height,
            inner: Mutex::new(VideoPipelineInner {
                demux_handle: Some(demux_handle),
                decode_handle: Some(decode_handle),
            }),
            stop_flag,
            packet_queue,
            frame_queue,
        })
    }

    /**
        Get the frame queue for reading decoded frames.
    */
    pub fn frame_queue(&self) -> &Arc<FrameQueue> {
        &self.frame_queue
    }

    /**
        Seek to a new position in the video.
        Stops current threads, clears queues, and restarts from the new position.
    */
    pub fn seek_to(&self, position: Duration) -> Result<(), DecoderError> {
        // 1. Signal threads to stop
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();
        self.frame_queue.close();

        // 2. Wait for threads to finish
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(handle) = inner.demux_handle.take() {
                let _ = handle.join();
            }
            if let Some(handle) = inner.decode_handle.take() {
                let _ = handle.join();
            }
        }

        // 3. Reset state
        self.stop_flag.store(false, Ordering::Relaxed);
        self.packet_queue.reopen();
        self.frame_queue.reopen();

        // 4. Spawn new threads starting at the seek position
        let demux_handle = {
            let path = self.path.clone();
            let packets = Arc::clone(&self.packet_queue);
            let stop = Arc::clone(&self.stop_flag);
            thread::spawn(move || video_demux(path, packets, stop, Some(position)))
        };

        let decode_handle = {
            let packets = Arc::clone(&self.packet_queue);
            let frames = Arc::clone(&self.frame_queue);
            let params = self.stream_info.codec_params.clone();
            let tb = self.stream_info.time_base;
            let stop = Arc::clone(&self.stop_flag);
            let tw = self.target_width;
            let th = self.target_height;
            thread::spawn(move || decode_video_packets(packets, frames, params, tb, stop, tw, th))
        };

        // 5. Store new handles
        {
            let mut inner = self.inner.lock().unwrap();
            inner.demux_handle = Some(demux_handle);
            inner.decode_handle = Some(decode_handle);
        }

        Ok(())
    }

    /**
        Stop the pipeline and wait for threads to finish.
    */
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
