use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};

use super::decoder::{DecoderError, decode_video_packets, get_video_stream_info, video_demux};
use super::packet_queue::PacketQueue;
use super::queue::FrameQueue;

const VIDEO_PACKET_QUEUE_CAPACITY: usize = 120;
const VIDEO_FRAME_QUEUE_CAPACITY: usize = 60;

/**
    Completely self-contained video pipeline.
    Owns its own file handle, demux thread, decode thread, and frame queue.

    CRITICAL: This pipeline is completely independent from AudioPipeline.
    Blocking in this pipeline cannot affect audio, and vice versa.
*/
pub struct VideoPipeline {
    // Threads
    demux_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    decode_handle: Option<JoinHandle<Result<(), DecoderError>>>,

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
            thread::spawn(move || video_demux(path, packets, stop))
        };

        // Spawn decode thread
        let decode_handle = {
            let packets = Arc::clone(&packet_queue);
            let frames = Arc::clone(&frame_queue);
            let params = stream_info.codec_params;
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
            demux_handle: Some(demux_handle),
            decode_handle: Some(decode_handle),
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
        Stop the pipeline and wait for threads to finish.
    */
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.packet_queue.close();
        self.frame_queue.close();

        if let Some(handle) = self.demux_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.decode_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}
