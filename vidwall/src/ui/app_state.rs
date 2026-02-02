use std::sync::Arc;

use gpui::Global;

use crate::audio::AudioMixer;
use crate::playback::VideoPlayer;
use crate::video::ReadyVideos;

/**
    Global application state shared across all views.

    This is a GPUI Global - a singleton that can be accessed from any context.
    It holds shared state that multiple parts of the app need to access.
*/
pub struct AppState {
    /// Thread-safe storage for validated video files
    pub ready_videos: Arc<ReadyVideos>,
    /// Audio mixer for combining all video streams
    pub mixer: Arc<AudioMixer>,
    /// Current video players (dynamic length based on grid configuration)
    pub players: Vec<Arc<VideoPlayer>>,
    /// Master volume level (0.0 to 1.0)
    pub master_volume: f32,
    /// Whether master audio is muted
    pub master_muted: bool,
    /// Whether all videos are paused
    pub paused: bool,
    /// Flag to request skipping all videos (set by action, consumed by grid)
    pub skip_all_requested: bool,
}

impl Global for AppState {}

impl AppState {
    /**
        Create a new AppState with the given ready videos storage and mixer.
    */
    pub fn new(ready_videos: Arc<ReadyVideos>, mixer: Arc<AudioMixer>) -> Self {
        Self {
            ready_videos,
            mixer,
            players: Vec::new(),
            master_volume: 1.0,
            master_muted: false,
            paused: false,
            skip_all_requested: false,
        }
    }

    /**
        Request skipping all videos (will be handled by the grid view).
    */
    pub fn request_skip_all(&mut self) {
        self.skip_all_requested = true;
    }

    /**
        Check and consume the skip all request.
        Returns true if skip was requested.
    */
    pub fn take_skip_all_request(&mut self) -> bool {
        let was_requested = self.skip_all_requested;
        self.skip_all_requested = false;
        was_requested
    }

    /**
        Toggle pause state for all videos.
        Returns the new paused state.
    */
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

    /**
        Toggle mute state for all videos.
        Returns the new muted state.
    */
    pub fn toggle_mute(&mut self) -> bool {
        self.master_muted = !self.master_muted;

        if self.master_muted {
            self.mixer.mute();
        } else {
            self.mixer.unmute();
        }

        self.master_muted
    }

    /**
        Adjust master volume by the given delta.
        Volume is clamped to [0.0, 1.0].
    */
    pub fn adjust_volume(&mut self, delta: f32) {
        self.master_volume = (self.master_volume + delta).clamp(0.0, 1.0);
        self.mixer.set_master_volume(self.master_volume);
    }

    /**
        Set master volume to a specific value.
        Volume is clamped to [0.0, 1.0].
    */
    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
        self.mixer.set_master_volume(self.master_volume);
    }

    /**
        Set or add a player at the given index.
        Automatically grows the players vector if needed.
    */
    pub fn set_player(&mut self, index: usize, player: Arc<VideoPlayer>) {
        // Grow vector if needed
        while self.players.len() <= index {
            // This is a placeholder - will be replaced immediately
            self.players.push(Arc::clone(&player));
        }
        self.players[index] = player;
    }

    /**
        Truncate the players list to the given length.
        Used when the grid shrinks.
    */
    pub fn truncate_players(&mut self, len: usize) {
        self.players.truncate(len);
    }

    /**
        Get the number of active players.
    */
    pub fn player_count(&self) -> usize {
        self.players.len()
    }
}
