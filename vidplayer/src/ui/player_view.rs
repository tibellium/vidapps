use std::sync::Arc;

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, rgb};

use crate::playback::VideoPlayer;

use super::video_element::video_element;

pub struct PlayerView {
    player: Arc<VideoPlayer>,
}

impl PlayerView {
    pub fn new(player: Arc<VideoPlayer>) -> Self {
        Self { player }
    }

    pub fn player(&self) -> &Arc<VideoPlayer> {
        &self.player
    }
}

impl Render for PlayerView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let aspect_ratio = self.player.aspect_ratio();

        div()
            .id("player")
            .size_full()
            .bg(rgb(0x000000))
            .overflow_hidden()
            .child(video_element(
                Arc::clone(&self.player),
                aspect_ratio,
                "video",
            ))
    }
}
