use std::sync::Arc;

use gpui::Global;

use ffmpeg_types::AudioClock;

use crate::audio::{AudioStream, AudioStreamConsumer};
use crate::playback::VideoPlayer;

pub struct AppState {
    pub player: Option<Arc<VideoPlayer>>,
    pub audio_consumer: Arc<AudioStreamConsumer>,
    pub volume: f32,
    pub muted: bool,
}

impl Global for AppState {}

impl AppState {
    pub fn new() -> Self {
        // Create a default audio stream for the consumer
        // This will be replaced when a video is loaded
        let audio_stream = AudioStream::new();

        Self {
            player: None,
            audio_consumer: audio_stream.consumer,
            volume: 1.0,
            muted: false,
        }
    }

    pub fn set_player(&mut self, player: Arc<VideoPlayer>) {
        // Update audio consumer if player has audio
        if let Some(consumer) = player.audio_consumer() {
            consumer.set_volume(self.volume);
            if self.muted {
                consumer.mute();
            }
            self.audio_consumer = consumer;
        }
        self.player = Some(player);
    }

    pub fn adjust_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        self.audio_consumer.set_volume(self.volume);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.audio_consumer.set_volume(self.volume);
    }

    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        if self.muted {
            self.audio_consumer.mute();
        } else {
            self.audio_consumer.unmute();
        }
        self.muted
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
