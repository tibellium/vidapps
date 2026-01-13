use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use super::decoder::{DecoderError, decode_video, get_video_info};
use super::frame::VideoFrame;
use super::queue::FrameQueue;

const DEFAULT_QUEUE_CAPACITY: usize = 60;

/// Playback state
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Ended,
    Error,
}

/// High-level video player that manages decoding and playback timing
pub struct VideoPlayer {
    path: PathBuf,
    queue: Arc<FrameQueue>,
    stop_flag: Arc<AtomicBool>,
    decoder_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    start_time: Instant,
    current_frame: Mutex<Option<VideoFrame>>,
    next_frame: Mutex<Option<VideoFrame>>, // Buffered frame waiting for its PTS
    base_pts: Mutex<Option<Duration>>,     // PTS of the first frame (used as offset)
    duration: Duration,
    state: Mutex<PlaybackState>,
    // Cached render image - only recreated when frame changes
    cached_render_image: Mutex<Option<Arc<RenderImage>>>,
    frame_generation: AtomicU64, // Incremented each time frame changes
}

impl VideoPlayer {
    /// Create a new video player for the given file
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, DecoderError> {
        Self::with_options(path, None, None)
    }

    /// Create a new video player with target dimensions
    pub fn with_options<P: AsRef<Path>>(
        path: P,
        target_width: Option<u32>,
        target_height: Option<u32>,
    ) -> Result<Self, DecoderError> {
        let path = path.as_ref().to_path_buf();
        let info = get_video_info(&path)?;

        let queue = Arc::new(FrameQueue::new(DEFAULT_QUEUE_CAPACITY));
        let stop_flag = Arc::new(AtomicBool::new(false));

        // Spawn decoder thread
        let decoder_path = path.clone();
        let decoder_queue = Arc::clone(&queue);
        let decoder_stop = Arc::clone(&stop_flag);

        let decoder_handle = thread::spawn(move || {
            decode_video(
                decoder_path,
                decoder_queue,
                decoder_stop,
                target_width,
                target_height,
            )
        });

        Ok(Self {
            path,
            queue,
            stop_flag,
            decoder_handle: Some(decoder_handle),
            start_time: Instant::now(),
            current_frame: Mutex::new(None),
            next_frame: Mutex::new(None),
            base_pts: Mutex::new(None),
            duration: info.duration,
            state: Mutex::new(PlaybackState::Playing),
            cached_render_image: Mutex::new(None),
            frame_generation: AtomicU64::new(0),
        })
    }

    /// Get the video file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the video duration
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Get the current playback position
    pub fn position(&self) -> Duration {
        let current = self.current_frame.lock().unwrap();
        current.as_ref().map(|f| f.pts).unwrap_or(Duration::ZERO)
    }

    /// Get the current playback state
    pub fn state(&self) -> PlaybackState {
        *self.state.lock().unwrap()
    }

    /// Check if playback has ended
    pub fn is_ended(&self) -> bool {
        self.state() == PlaybackState::Ended
    }

    /// Get the cached RenderImage for the current frame.
    /// Only creates a new RenderImage when the frame actually changes.
    /// Returns (current_image, old_image_to_drop) - caller should drop the old image via window.drop_image()
    pub fn get_render_image(&self) -> (Option<Arc<RenderImage>>, Option<Arc<RenderImage>>) {
        let elapsed = self.start_time.elapsed();

        let mut current = self.current_frame.lock().unwrap();
        let mut next = self.next_frame.lock().unwrap();
        let mut base_pts = self.base_pts.lock().unwrap();
        let mut state = self.state.lock().unwrap();
        let mut cached = self.cached_render_image.lock().unwrap();

        let mut frame_changed = false;
        let mut old_image: Option<Arc<RenderImage>> = None;

        // If we don't have a next frame buffered, try to get one
        if next.is_none() {
            *next = self.queue.try_pop();
        }

        // Initialize base_pts from the first frame
        if base_pts.is_none() {
            if let Some(ref frame) = *next {
                *base_pts = Some(frame.pts);
            }
        }

        // Advance to the next frame if its PTS has passed
        // Only advance one frame per render to ensure smooth playback
        // (aggressive catch-up causes visible frame skipping on VFR videos)
        if let Some(ref frame) = *next {
            let base = base_pts.unwrap_or(Duration::ZERO);
            let relative_pts = frame.pts.saturating_sub(base);

            if elapsed >= relative_pts {
                // Time to show this frame
                *current = next.take();
                frame_changed = true;
                self.frame_generation.fetch_add(1, Ordering::Relaxed);

                // Pre-fetch the next frame
                *next = self.queue.try_pop();
            }
        }

        // Check for end of playback
        if next.is_none() && self.queue.is_closed() {
            if current.is_some() {
                let base = base_pts.unwrap_or(Duration::ZERO);
                let adjusted_duration = self.duration.saturating_sub(base);
                if elapsed > adjusted_duration {
                    *state = PlaybackState::Ended;
                }
            }
        }

        // Only create new RenderImage if frame changed or we don't have one yet
        if frame_changed || cached.is_none() {
            if let Some(ref frame) = *current {
                if let Some(render_image) = frame_to_render_image(frame) {
                    // Save old image to be dropped
                    old_image = cached.take();
                    *cached = Some(Arc::new(render_image));
                }
            }
        }

        (cached.clone(), old_image)
    }

    /// Get the current frame for rendering based on elapsed time.
    /// Returns a reference to the current frame if available.
    pub fn get_frame(&self) -> Option<VideoFrame> {
        self.current_frame.lock().unwrap().clone()
    }

    /// Get the number of buffered frames
    pub fn buffered_frames(&self) -> usize {
        self.queue.len()
    }

    /// Stop playback and clean up resources
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.queue.close();

        if let Some(handle) = self.decoder_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Convert a VideoFrame to a RenderImage
fn frame_to_render_image(frame: &VideoFrame) -> Option<RenderImage> {
    // Data is already in BGRA format from the decoder (matches GPUI's expected format)
    let image = RgbaImage::from_raw(frame.width, frame.height, frame.data.clone())?;

    // Create a Frame from the image
    let img_frame = Frame::new(image);

    // Create RenderImage
    Some(RenderImage::new(vec![img_frame]))
}
