use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::audio::{AudioStreamClock, AudioStreamConsumer};
use crate::decode::{DecoderError, get_video_info};

use super::audio_pipeline::AudioPipeline;
use super::frame::VideoFrame;
use super::video_pipeline::VideoPipeline;

/**
    Playback state
*/
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Ended,
    Error,
}

/**
    Playback clock abstraction.

    For videos WITH audio: Uses AudioStreamClock (audio is master).
    Pausing works by pausing the audio consumer, which stops samples
    from being consumed, naturally freezing the clock position.

    For videos WITHOUT audio: Uses wall clock with pause support
    via accumulated time tracking.
*/
pub enum PlaybackClock {
    /// Audio-driven clock - position comes from samples consumed
    Audio(Arc<AudioStreamClock>),
    /// Wall-time clock with pause support
    WallTime {
        /// Time accumulated before current play session
        accumulated: Mutex<Duration>,
        /// When we last started/resumed, None if paused
        playing_since: Mutex<Option<Instant>>,
    },
}

impl PlaybackClock {
    pub fn wall_time() -> Self {
        Self::WallTime {
            accumulated: Mutex::new(Duration::ZERO),
            playing_since: Mutex::new(Some(Instant::now())),
        }
    }

    pub fn audio(clock: Arc<AudioStreamClock>) -> Self {
        Self::Audio(clock)
    }

    pub fn position(&self) -> Duration {
        match self {
            Self::Audio(clock) => clock.position(),
            Self::WallTime {
                accumulated,
                playing_since,
            } => {
                let acc = *accumulated.lock().unwrap();
                match *playing_since.lock().unwrap() {
                    Some(since) => acc + since.elapsed(),
                    None => acc, // Paused - return frozen position
                }
            }
        }
    }

    /// Pause the clock. For wall-time clocks, freezes the position.
    /// For audio clocks, this is a no-op (audio consumer handles pause).
    pub fn pause(&self) {
        if let Self::WallTime {
            accumulated,
            playing_since,
        } = self
        {
            let mut since = playing_since.lock().unwrap();
            if let Some(start) = since.take() {
                // Save accumulated time and clear playing_since
                *accumulated.lock().unwrap() += start.elapsed();
            }
        }
    }

    /// Resume the clock. For wall-time clocks, starts tracking time again.
    /// For audio clocks, this is a no-op (audio consumer handles resume).
    pub fn resume(&self) {
        if let Self::WallTime { playing_since, .. } = self {
            let mut since = playing_since.lock().unwrap();
            if since.is_none() {
                *since = Some(Instant::now());
            }
        }
    }

    /// Seek the clock to a new position.
    /// For wall-time clocks, resets accumulated time.
    /// For audio clocks, this is handled by AudioStreamClock::reset_to().
    pub fn seek_to(&self, position: Duration) {
        match self {
            Self::Audio(clock) => {
                clock.reset_to(position);
            }
            Self::WallTime {
                accumulated,
                playing_since,
            } => {
                *accumulated.lock().unwrap() = position;
                // If currently playing, reset the start time to now
                let mut since = playing_since.lock().unwrap();
                if since.is_some() {
                    *since = Some(Instant::now());
                }
            }
        }
    }
}

/**
    High-level video player that manages decoding and playback timing.

    Uses completely separated pipelines for audio and video:
    - AudioPipeline: owns its own file handle, demux thread, decode thread, ring buffer
    - VideoPipeline: owns its own file handle, demux thread, decode thread, frame queue

    The ONLY shared state between pipelines is the AudioClock, which audio writes
    and video reads. This architecture prevents deadlocks caused by blocking in
    one pipeline affecting the other.
*/
pub struct VideoPlayer {
    path: PathBuf,

    // Separated pipelines (completely independent)
    audio_pipeline: Option<AudioPipeline>,
    video_pipeline: VideoPipeline,

    // Timing
    playback_clock: PlaybackClock,

    // Frame state
    current_frame: Mutex<Option<VideoFrame>>,
    next_frame: Mutex<Option<VideoFrame>>,
    base_pts: Mutex<Option<Duration>>,
    duration: Duration,
    state: Mutex<PlaybackState>,

    // Render cache
    cached_render_image: Mutex<Option<Arc<RenderImage>>>,
    frame_generation: AtomicU64,
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

        // Create audio pipeline (if file has audio)
        // This is completely independent - owns its own file handle and threads
        let audio_pipeline = match AudioPipeline::new(path.clone()) {
            Ok(pipeline) => pipeline,
            Err(e) => {
                eprintln!("Warning: Audio pipeline failed: {}. Using wall clock.", e);
                None
            }
        };

        // Create video pipeline (always required)
        // This is completely independent - owns its own file handle and threads
        let video_pipeline = VideoPipeline::new(path.clone(), target_width, target_height)?;

        // Determine clock source based on audio availability
        let playback_clock = if let Some(ref audio) = audio_pipeline {
            PlaybackClock::audio(Arc::clone(audio.clock()))
        } else {
            PlaybackClock::wall_time()
        };

        Ok(Self {
            path,
            audio_pipeline,
            video_pipeline,
            playback_clock,
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
        self.playback_clock.position()
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
        Check if playback is paused
    */
    pub fn is_paused(&self) -> bool {
        self.state() == PlaybackState::Paused
    }

    /**
        Pause video and audio playback
    */
    pub fn pause(&self) {
        let mut state = self.state.lock().unwrap();
        if *state == PlaybackState::Playing {
            *state = PlaybackState::Paused;
            self.playback_clock.pause();
            if let Some(ref audio) = self.audio_pipeline {
                audio.consumer().pause();
            }
        }
    }

    /**
        Resume video and audio playback
    */
    pub fn resume(&self) {
        let mut state = self.state.lock().unwrap();
        if *state == PlaybackState::Paused {
            *state = PlaybackState::Playing;
            self.playback_clock.resume();
            if let Some(ref audio) = self.audio_pipeline {
                audio.consumer().resume();
            }
        }
    }

    /**
        Toggle between paused and playing states
    */
    pub fn toggle_pause(&self) {
        if self.is_paused() {
            self.resume();
        } else {
            self.pause();
        }
    }

    /**
        Get the audio stream consumer if this video has audio
    */
    pub fn audio_consumer(&self) -> Option<Arc<AudioStreamConsumer>> {
        self.audio_pipeline.as_ref().map(|p| p.consumer())
    }

    /**
        Get the audio clock if this video has audio.
    */
    pub fn audio_clock(&self) -> Option<&Arc<AudioStreamClock>> {
        self.audio_pipeline.as_ref().map(|p| p.clock())
    }

    /**
        Set the volume for this video's audio (0.0 to 1.0)
    */
    pub fn set_volume(&self, volume: f32) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.set_volume(volume);
        }
    }

    /**
        Get the current volume for this video's audio (0.0 to 1.0)
    */
    pub fn volume(&self) -> f32 {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.volume())
            .unwrap_or(0.0)
    }

    /**
        Check if this video has an audio track
    */
    pub fn has_audio(&self) -> bool {
        self.audio_pipeline.is_some()
    }

    /**
        Mute this video's audio
    */
    pub fn mute(&self) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.consumer().mute();
        }
    }

    /**
        Unmute this video's audio
    */
    pub fn unmute(&self) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.consumer().unmute();
        }
    }

    /**
        Toggle mute state. Returns the new muted state.
    */
    pub fn toggle_mute(&self) -> bool {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.consumer().toggle_mute())
            .unwrap_or(false)
    }

    /**
        Check if this video's audio is muted
    */
    pub fn is_muted(&self) -> bool {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.consumer().is_muted())
            .unwrap_or(false)
    }

    /**
        Seek to a specific position in the video.

        This will:
        1. Stop current demux/decode threads
        2. Clear all buffers
        3. Seek in the file
        4. Restart threads from the new position
        5. Reset the playback clock

        Returns the new audio consumer if this video has audio (caller must update mixer).
        The current playback state (playing/paused) is preserved.
    */
    pub fn seek_to(
        &self,
        position: Duration,
    ) -> Result<Option<Arc<AudioStreamConsumer>>, DecoderError> {
        // Clamp position to valid range
        let position = position.min(self.duration);

        // Remember current state
        let was_paused = self.is_paused();

        // Seek video pipeline
        self.video_pipeline.seek_to(position)?;

        // Seek audio pipeline if present, get new consumer
        let new_consumer = if let Some(ref audio) = self.audio_pipeline {
            Some(audio.seek_to(position)?)
        } else {
            None
        };

        // Reset playback clock (for wall-time clock; audio clock is reset by pipeline)
        self.playback_clock.seek_to(position);

        // Clear frame state
        {
            *self.current_frame.lock().unwrap() = None;
            *self.next_frame.lock().unwrap() = None;
            *self.base_pts.lock().unwrap() = None;
            *self.cached_render_image.lock().unwrap() = None;
            self.frame_generation.fetch_add(1, Ordering::Relaxed);
        }

        // Reset state to playing (unless it was paused)
        {
            let mut state = self.state.lock().unwrap();
            if *state == PlaybackState::Ended || *state == PlaybackState::Error {
                *state = PlaybackState::Playing;
            }
        }

        // Restore pause state if needed
        if was_paused {
            self.pause();
        }

        Ok(new_consumer)
    }

    /**
        Seek forward by the given duration.
        Returns the new audio consumer if this video has audio (caller must update mixer).
    */
    pub fn seek_forward(
        &self,
        amount: Duration,
    ) -> Result<Option<Arc<AudioStreamConsumer>>, DecoderError> {
        let new_position = self.position().saturating_add(amount);
        self.seek_to(new_position)
    }

    /**
        Seek backward by the given duration.
        Returns the new audio consumer if this video has audio (caller must update mixer).
    */
    pub fn seek_backward(
        &self,
        amount: Duration,
    ) -> Result<Option<Arc<AudioStreamConsumer>>, DecoderError> {
        let new_position = self.position().saturating_sub(amount);
        self.seek_to(new_position)
    }

    /**
        Get the cached RenderImage for the current frame.
        Only creates a new RenderImage when the frame actually changes.

        For videos with audio, frame timing is driven by the audio clock.
        For videos without audio, frame timing uses wall clock.

        Returns (current_image, old_image_to_drop)
    */
    pub fn get_render_image(&self) -> (Option<Arc<RenderImage>>, Option<Arc<RenderImage>>) {
        let elapsed = self.playback_clock.position();
        let frame_queue = self.video_pipeline.frame_queue();

        let mut current = self.current_frame.lock().unwrap();
        let mut next = self.next_frame.lock().unwrap();
        let mut base_pts = self.base_pts.lock().unwrap();
        let mut state = self.state.lock().unwrap();
        let mut cached = self.cached_render_image.lock().unwrap();

        let mut frame_changed = false;
        let mut old_image: Option<Arc<RenderImage>> = None;

        // If we don't have a next frame buffered, try to get one
        if next.is_none() {
            *next = frame_queue.try_pop();
        }

        // Initialize base_pts from the first frame
        if base_pts.is_none() {
            if let Some(ref frame) = *next {
                *base_pts = Some(frame.pts);
            }
        }

        // Advance to the next frame if its PTS has passed
        if let Some(ref frame) = *next {
            let base = base_pts.unwrap_or(Duration::ZERO);
            let relative_pts = frame.pts.saturating_sub(base);

            if elapsed >= relative_pts {
                *current = next.take();
                frame_changed = true;
                self.frame_generation.fetch_add(1, Ordering::Relaxed);
                *next = frame_queue.try_pop();
            }
        }

        // Check for end of playback
        // Only mark as ended when:
        // 1. No next frame buffered
        // 2. Frame queue is closed (no more frames coming)
        // 3. Frame queue is empty (all frames have been consumed)
        // 4. We've shown the current frame long enough (elapsed > its PTS)
        if next.is_none() && frame_queue.is_closed() && frame_queue.is_empty() {
            if let Some(ref frame) = *current {
                let base = base_pts.unwrap_or(Duration::ZERO);
                let frame_pts = frame.pts.saturating_sub(base);
                // Only end after we've been past the last frame's PTS for a bit
                // This ensures the last frame is displayed
                if elapsed > frame_pts {
                    *state = PlaybackState::Ended;
                }
            } else {
                // No current frame and nothing left - we're done
                *state = PlaybackState::Ended;
            }
        }

        // Only create new RenderImage if frame changed or we don't have one yet
        if frame_changed || cached.is_none() {
            if let Some(ref frame) = *current {
                if let Some(render_image) = frame_to_render_image(frame) {
                    old_image = cached.take();
                    *cached = Some(Arc::new(render_image));
                }
            }
        }

        (cached.clone(), old_image)
    }

    /**
        Get the current frame for rendering based on elapsed time.
    */
    pub fn get_frame(&self) -> Option<VideoFrame> {
        self.current_frame.lock().unwrap().clone()
    }

    /**
        Get the number of buffered video frames
    */
    pub fn buffered_frames(&self) -> usize {
        self.video_pipeline.frame_queue().len()
    }

    /**
        Get the number of buffered audio samples
    */
    pub fn buffered_audio_samples(&self) -> usize {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.consumer().available())
            .unwrap_or(0)
    }

    /**
        Stop playback and clean up resources
    */
    pub fn stop(&self) {
        // Stop both pipelines (they handle their own cleanup)
        if let Some(ref audio) = self.audio_pipeline {
            audio.stop();
        }
        self.video_pipeline.stop();
    }
}

/**
    Convert a VideoFrame to a RenderImage
*/
fn frame_to_render_image(frame: &VideoFrame) -> Option<RenderImage> {
    let image = RgbaImage::from_raw(frame.width, frame.height, frame.data.clone())?;
    let img_frame = Frame::new(image);
    Some(RenderImage::new(vec![img_frame]))
}
