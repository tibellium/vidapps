use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use super::decoder::{DecoderError, decode_video, get_video_info};
use super::frame::VideoFrame;
use super::queue::FrameQueue;
use crate::audio::{
    AudioStreamClock, AudioStreamConsumer, AudioStreamProducer, create_audio_stream,
};

const DEFAULT_VIDEO_QUEUE_CAPACITY: usize = 60;

/**
    Playback state
*/
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Ended,
    Error,
}

/**
    High-level video player that manages decoding and playback timing.

    For videos with audio, timing is driven by the audio clock (samples consumed).
    For videos without audio, timing falls back to wall clock.
*/
pub struct VideoPlayer {
    path: PathBuf,
    video_queue: Arc<FrameQueue>,
    audio_producer: Option<Arc<AudioStreamProducer>>,
    audio_consumer: Option<Arc<AudioStreamConsumer>>,
    /// Shared audio clock for A/V sync (None for videos without audio)
    audio_clock: Option<Arc<AudioStreamClock>>,
    stop_flag: Arc<AtomicBool>,
    decoder_handle: Option<JoinHandle<Result<(), DecoderError>>>,
    /// Wall clock start time (used as fallback for videos without audio)
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
    /**
        Create a new video player for the given file
    */
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, DecoderError> {
        Self::with_options(path, None, None)
    }

    /**
        Create a new video player with target dimensions
    */
    pub fn with_options<P: AsRef<Path>>(
        path: P,
        target_width: Option<u32>,
        target_height: Option<u32>,
    ) -> Result<Self, DecoderError> {
        let path = path.as_ref().to_path_buf();
        let info = get_video_info(&path)?;
        let start_time = Instant::now();

        let video_queue = Arc::new(FrameQueue::new(DEFAULT_VIDEO_QUEUE_CAPACITY));
        let stop_flag = Arc::new(AtomicBool::new(false));

        // Create audio stream if video has audio
        let (audio_producer, audio_consumer, audio_clock) = if info.has_audio {
            let (producer, consumer, clock) = create_audio_stream();
            (
                Some(Arc::new(producer)),
                Some(Arc::new(consumer)),
                Some(clock),
            )
        } else {
            (None, None, None)
        };

        // Spawn decoder thread
        let decoder_path = path.clone();
        let decoder_video_queue = Arc::clone(&video_queue);
        let decoder_audio_producer = audio_producer.clone();
        let decoder_stop = Arc::clone(&stop_flag);

        let decoder_handle = thread::spawn(move || {
            decode_video(
                decoder_path,
                decoder_video_queue,
                decoder_audio_producer,
                decoder_stop,
                target_width,
                target_height,
            )
        });

        Ok(Self {
            path,
            video_queue,
            audio_producer,
            audio_consumer,
            audio_clock,
            stop_flag,
            decoder_handle: Some(decoder_handle),
            start_time,
            current_frame: Mutex::new(None),
            next_frame: Mutex::new(None),
            base_pts: Mutex::new(None),
            duration: info.duration,
            state: Mutex::new(PlaybackState::Playing),
            cached_render_image: Mutex::new(None),
            frame_generation: AtomicU64::new(0),
        })
    }

    /**
        Get the video file path
    */
    pub fn path(&self) -> &Path {
        &self.path
    }

    /**
        Get the video duration
    */
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /**
        Get the current playback position.
        For videos with audio, this is the audio clock position.
        For videos without audio, this is based on wall clock.
    */
    pub fn position(&self) -> Duration {
        self.get_playback_time()
    }

    /**
        Get the current playback time to use for frame timing.
        Uses audio clock if available, otherwise wall clock.
    */
    fn get_playback_time(&self) -> Duration {
        if let Some(ref clock) = self.audio_clock {
            // Audio-driven: use the audio playback position
            clock.position()
        } else {
            // No audio: fall back to wall clock
            self.start_time.elapsed()
        }
    }

    /**
        Get the current playback state
    */
    pub fn state(&self) -> PlaybackState {
        *self.state.lock().unwrap()
    }

    /**
        Check if playback has ended
    */
    pub fn is_ended(&self) -> bool {
        self.state() == PlaybackState::Ended
    }

    /**
        Get the audio stream consumer if this video has audio
    */
    pub fn audio_consumer(&self) -> Option<&Arc<AudioStreamConsumer>> {
        self.audio_consumer.as_ref()
    }

    /**
        Get the audio clock if this video has audio.
        This can be shared with other components for A/V sync.
    */
    pub fn audio_clock(&self) -> Option<&Arc<AudioStreamClock>> {
        self.audio_clock.as_ref()
    }

    /**
        Set the volume for this video's audio (0.0 to 1.0)
    */
    pub fn set_volume(&self, volume: f32) {
        if let Some(ref consumer) = self.audio_consumer {
            consumer.set_volume(volume);
        }
    }

    /**
        Get the current volume for this video's audio (0.0 to 1.0)
    */
    pub fn volume(&self) -> f32 {
        self.audio_consumer
            .as_ref()
            .map(|c| c.volume())
            .unwrap_or(0.0)
    }

    /**
        Check if this video has an audio track
    */
    pub fn has_audio(&self) -> bool {
        self.audio_consumer.is_some()
    }

    /**
        Get the cached RenderImage for the current frame.
        Only creates a new RenderImage when the frame actually changes.

        For videos with audio, frame timing is driven by the audio clock.
        For videos without audio, frame timing uses wall clock.

        Returns (current_image, old_image_to_drop) - caller should drop the old image via window.drop_image()
    */
    pub fn get_render_image(&self) -> (Option<Arc<RenderImage>>, Option<Arc<RenderImage>>) {
        // Get playback time from audio clock or wall clock
        let elapsed = self.get_playback_time();

        let mut current = self.current_frame.lock().unwrap();
        let mut next = self.next_frame.lock().unwrap();
        let mut base_pts = self.base_pts.lock().unwrap();
        let mut state = self.state.lock().unwrap();
        let mut cached = self.cached_render_image.lock().unwrap();

        let mut frame_changed = false;
        let mut old_image: Option<Arc<RenderImage>> = None;

        // If we don't have a next frame buffered, try to get one
        if next.is_none() {
            *next = self.video_queue.try_pop();
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
                *next = self.video_queue.try_pop();
            }
        }

        // Check for end of playback
        if next.is_none() && self.video_queue.is_closed() {
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

    /**
        Get the current frame for rendering based on elapsed time.
        Returns a reference to the current frame if available.
    */
    pub fn get_frame(&self) -> Option<VideoFrame> {
        self.current_frame.lock().unwrap().clone()
    }

    /**
        Get the number of buffered video frames
    */
    pub fn buffered_frames(&self) -> usize {
        self.video_queue.len()
    }

    /**
        Get the number of buffered audio samples
    */
    pub fn buffered_audio_samples(&self) -> usize {
        self.audio_consumer
            .as_ref()
            .map(|c| c.available())
            .unwrap_or(0)
    }

    /**
        Stop playback and clean up resources
    */
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.video_queue.close();
        if let Some(ref producer) = self.audio_producer {
            producer.close();
        }

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

/**
    Convert a VideoFrame to a RenderImage
*/
fn frame_to_render_image(frame: &VideoFrame) -> Option<RenderImage> {
    // Data is already in BGRA format from the decoder (matches GPUI's expected format)
    let image = RgbaImage::from_raw(frame.width, frame.height, frame.data.clone())?;

    // Create a Frame from the image
    let img_frame = Frame::new(image);

    // Create RenderImage
    Some(RenderImage::new(vec![img_frame]))
}
