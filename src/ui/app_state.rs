use std::path::PathBuf;
use std::sync::Arc;

use gpui::Global;

use crate::audio::AudioMixer;
use crate::playback::VideoPlayer;

/// Global application state shared across all views.
///
/// This is a GPUI Global - a singleton that can be accessed from any context.
/// It holds shared state that multiple parts of the app need to access.
pub struct AppState {
    /// All video file paths discovered in the folder
    pub video_paths: Vec<PathBuf>,
    /// Audio mixer for combining all video streams
    pub mixer: Arc<AudioMixer>,
    /// Current video players (updated when videos are replaced)
    pub players: Vec<Arc<VideoPlayer>>,
    /// Master volume level (0.0 to 1.0)
    pub master_volume: f32,
    /// Whether master audio is muted
    pub master_muted: bool,
    /// Whether all videos are paused
    pub paused: bool,
}

impl Global for AppState {}

impl AppState {
    /// Create a new AppState with the given video paths and mixer.
    pub fn new(
        video_paths: Vec<PathBuf>,
        mixer: Arc<AudioMixer>,
        players: Vec<Arc<VideoPlayer>>,
    ) -> Self {
        Self {
            video_paths,
            mixer,
            players,
            master_volume: 1.0,
            master_muted: false,
            paused: false,
        }
    }

    /// Toggle pause state for all videos.
    /// Returns the new paused state.
    pub fn toggle_pause(&mut self) -> bool {
        self.paused = !self.paused;

        for player in &self.players {
            if self.paused {
                player.pause();
            } else {
                player.resume();
            }
        }

        self.paused
    }

    /// Toggle mute state for all videos.
    /// Returns the new muted state.
    pub fn toggle_mute(&mut self) -> bool {
        self.master_muted = !self.master_muted;

        if self.master_muted {
            self.mixer.mute();
        } else {
            self.mixer.unmute();
        }

        self.master_muted
    }

    /// Adjust master volume by the given delta.
    /// Volume is clamped to [0.0, 1.0].
    pub fn adjust_volume(&mut self, delta: f32) {
        self.master_volume = (self.master_volume + delta).clamp(0.0, 1.0);
        self.mixer.set_master_volume(self.master_volume);
    }

    /// Set master volume to a specific value.
    /// Volume is clamped to [0.0, 1.0].
    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
        self.mixer.set_master_volume(self.master_volume);
    }

    /// Update a player at the given index (called when a video is replaced).
    pub fn set_player(&mut self, index: usize, player: Arc<VideoPlayer>) {
        if index < self.players.len() {
            self.players[index] = player;
        }
    }
}
