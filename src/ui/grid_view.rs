use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{div, prelude::*, rgb, AsyncApp, Context, WeakEntity, Window};
use rand::seq::SliceRandom;

use crate::video::VideoPlayer;

use super::video_element::video_element;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The main view holding our 4 video players in a 2x2 grid
pub struct VideoGridView {
    players: [Arc<VideoPlayer>; 4],
    all_video_paths: Vec<PathBuf>,
}

impl VideoGridView {
    pub fn new(
        players: [Arc<VideoPlayer>; 4],
        all_video_paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Start polling task to check for ended videos and request repaints
        cx.spawn(async |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;

                let should_continue = view
                    .update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                        this.check_and_replace_ended_videos(cx);
                        // Always notify to keep rendering new frames
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }
            }
        })
        .detach();

        Self {
            players,
            all_video_paths,
        }
    }

    /// Check each video slot and replace any that have finished
    fn check_and_replace_ended_videos(&mut self, _cx: &mut Context<Self>) {
        for i in 0..4 {
            if self.players[i].is_ended() {
                if let Some(new_player) = self.create_replacement_player() {
                    let path = new_player.path();
                    println!(
                        "Slot {}: replaced with {}",
                        i,
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    self.players[i] = Arc::new(new_player);
                }
            }
        }
    }

    /// Create a new random video player from the available paths
    fn create_replacement_player(&self) -> Option<VideoPlayer> {
        let mut rng = rand::thread_rng();
        let path = self.all_video_paths.choose(&mut rng)?;

        match VideoPlayer::with_options(path, None, None) {
            Ok(player) => Some(player),
            Err(e) => {
                eprintln!("Failed to create player for {:?}: {}", path, e);
                None
            }
        }
    }
}

impl Render for VideoGridView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x000000))
            .flex()
            .flex_col()
            .children([
                // Top row
                div().flex_1().flex().flex_row().children([
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(video_element(Arc::clone(&self.players[0]), "video-0")),
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(video_element(Arc::clone(&self.players[1]), "video-1")),
                ]),
                // Bottom row
                div().flex_1().flex().flex_row().children([
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(video_element(Arc::clone(&self.players[2]), "video-2")),
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .child(video_element(Arc::clone(&self.players[3]), "video-3")),
                ]),
            ])
    }
}
