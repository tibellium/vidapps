use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use ffmpeg_types::{AudioClock, Clock, WallClock};

use crate::audio::AudioStreamConsumer;

use super::audio_pipeline::AudioPipeline;
use super::frame::VideoFrame;
use super::video_pipeline::VideoPipeline;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Ended,
    Error,
}

pub enum PlaybackClock {
    Audio(Arc<AudioClock>),
    WallTime(Arc<WallClock>),
}

impl PlaybackClock {
    pub fn wall_time() -> Self {
        Self::WallTime(Arc::new(WallClock::new()))
    }

    pub fn audio(clock: Arc<AudioClock>) -> Self {
        Self::Audio(clock)
    }

    pub fn position(&self) -> Duration {
        match self {
            Self::Audio(clock) => clock.position(),
            Self::WallTime(clock) => clock.position(),
        }
    }

    pub fn pause(&self) {
        if let Self::WallTime(clock) = self {
            clock.pause();
        }
    }

    pub fn resume(&self) {
        if let Self::WallTime(clock) = self {
            clock.resume();
        }
    }

    pub fn seek_to(&self, position: Duration) {
        match self {
            Self::Audio(clock) => clock.reset_to(position),
            Self::WallTime(clock) => clock.reset_to(position),
        }
    }
}

pub struct VideoPlayer {
    path: PathBuf,
    audio_pipeline: Option<AudioPipeline>,
    video_pipeline: VideoPipeline,
    playback_clock: PlaybackClock,
    current_frame: Mutex<Option<VideoFrame>>,
    next_frame: Mutex<Option<VideoFrame>>,
    /// Base PTS from first frame - used to align video timeline with clock
    base_pts: Mutex<Option<Duration>>,
    duration: Duration,
    state: Mutex<PlaybackState>,
    cached_render_image: Mutex<Option<Arc<RenderImage>>>,
    frame_generation: AtomicU64,
}

impl VideoPlayer {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, ffmpeg_types::Error> {
        let path = path.as_ref().to_path_buf();

        // Probe for duration
        let info = ffmpeg_source::probe(&path)?;
        let duration = info.duration.unwrap_or(Duration::ZERO);

        // Create audio pipeline (if file has audio)
        let audio_pipeline = AudioPipeline::new(path.clone());

        // Create video pipeline
        let video_pipeline = VideoPipeline::new(path.clone())?;

        // Determine clock source
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
            duration,
            state: Mutex::new(PlaybackState::Playing),
            cached_render_image: Mutex::new(None),
            frame_generation: AtomicU64::new(0),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn position(&self) -> Duration {
        self.playback_clock.position()
    }

    pub fn state(&self) -> PlaybackState {
        *self.state.lock().unwrap()
    }

    pub fn is_ended(&self) -> bool {
        self.state() == PlaybackState::Ended
    }

    pub fn is_paused(&self) -> bool {
        self.state() == PlaybackState::Paused
    }

    pub fn width(&self) -> u32 {
        self.video_pipeline.width()
    }

    pub fn height(&self) -> u32 {
        self.video_pipeline.height()
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.width() as f32 / self.height() as f32
    }

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

    pub fn toggle_pause(&self) {
        if self.is_paused() {
            self.resume();
        } else {
            self.pause();
        }
    }

    pub fn audio_consumer(&self) -> Option<Arc<AudioStreamConsumer>> {
        self.audio_pipeline.as_ref().map(|p| p.consumer())
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.set_volume(volume);
        }
    }

    pub fn volume(&self) -> f32 {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.volume())
            .unwrap_or(0.0)
    }

    pub fn has_audio(&self) -> bool {
        self.audio_pipeline.is_some()
    }

    pub fn mute(&self) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.consumer().mute();
        }
    }

    pub fn unmute(&self) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.consumer().unmute();
        }
    }

    pub fn toggle_mute(&self) -> bool {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.consumer().toggle_mute())
            .unwrap_or(false)
    }

    pub fn is_muted(&self) -> bool {
        self.audio_pipeline
            .as_ref()
            .map(|a| a.consumer().is_muted())
            .unwrap_or(false)
    }

    pub fn seek_to(&self, position: Duration) {
        let position = position.min(self.duration);
        let was_paused = self.is_paused();

        // Seek video pipeline
        if let Err(e) = self.video_pipeline.seek_to(position) {
            eprintln!("[seek] video pipeline error: {}", e);
        }

        // Seek audio pipeline
        if let Some(ref audio) = self.audio_pipeline {
            audio.seek_to(position);
        }

        // Reset playback clock
        self.playback_clock.seek_to(position);

        // Clear frame state
        {
            *self.current_frame.lock().unwrap() = None;
            *self.next_frame.lock().unwrap() = None;
            *self.base_pts.lock().unwrap() = None;
            *self.cached_render_image.lock().unwrap() = None;
            self.frame_generation.fetch_add(1, Ordering::Relaxed);
        }

        // Reset state
        {
            let mut state = self.state.lock().unwrap();
            if *state == PlaybackState::Ended || *state == PlaybackState::Error {
                *state = PlaybackState::Playing;
            }
        }

        // Restore pause state
        if was_paused {
            self.pause();
        }
    }

    pub fn seek_forward(&self, amount: Duration) {
        let new_position = self.position().saturating_add(amount);
        self.seek_to(new_position);
    }

    pub fn seek_backward(&self, amount: Duration) {
        let new_position = self.position().saturating_sub(amount);
        self.seek_to(new_position);
    }

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

        // Buffer next frame if needed
        if next.is_none() {
            *next = frame_queue.try_pop();
        }

        // Initialize base_pts from first frame
        if base_pts.is_none() {
            if let Some(ref frame) = *next {
                *base_pts = Some(frame.pts);
            }
        }

        // Advance frame if its PTS has passed
        // Both times are relative to base_pts for consistent comparison
        if let Some(ref frame) = *next {
            let base = base_pts.unwrap_or(Duration::ZERO);
            let relative_pts = frame.pts.saturating_sub(base);
            let relative_elapsed = elapsed.saturating_sub(base);

            if relative_elapsed >= relative_pts {
                *current = next.take();
                frame_changed = true;
                self.frame_generation.fetch_add(1, Ordering::Relaxed);
                *next = frame_queue.try_pop();
            }
        }

        // Check for end of playback
        if next.is_none() && frame_queue.is_closed() && frame_queue.is_empty() {
            if let Some(ref frame) = *current {
                let base = base_pts.unwrap_or(Duration::ZERO);
                let relative_pts = frame.pts.saturating_sub(base);
                let relative_elapsed = elapsed.saturating_sub(base);
                if relative_elapsed > relative_pts {
                    *state = PlaybackState::Ended;
                }
            } else {
                *state = PlaybackState::Ended;
            }
        }

        // Create new RenderImage if frame changed
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

    pub fn stop(&self) {
        if let Some(ref audio) = self.audio_pipeline {
            audio.stop();
        }
        self.video_pipeline.stop();
    }
}

fn frame_to_render_image(frame: &VideoFrame) -> Option<RenderImage> {
    // Note: Despite the name, RgbaImage just holds raw bytes.
    // GPUI expects BGRA on macOS, which is what we provide.
    let image = RgbaImage::from_raw(frame.width, frame.height, frame.data.clone())?;
    let img_frame = Frame::new(image);
    Some(RenderImage::new(vec![img_frame]))
}
