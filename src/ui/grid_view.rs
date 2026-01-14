use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Render, Window, div, prelude::*, rgb};
use rand::seq::SliceRandom;

use crate::playback::VideoPlayer;

use super::app_state::AppState;
use super::video_element::video_element;
use super::video_slot::{VideoEnded, VideoSlot};

/// The main grid view that displays 4 videos in a 2x2 layout.
///
/// Uses VideoSlot entities for each video position and subscribes to their
/// VideoEnded events for automatic video replacement.
pub struct GridView {
    slots: [Entity<VideoSlot>; 4],
}

impl GridView {
    /// Create a new grid view with the given initial players.
    pub fn new(players: [Arc<VideoPlayer>; 4], cx: &mut Context<Self>) -> Self {
        // Create VideoSlot entities for each player
        let slots: [Entity<VideoSlot>; 4] = players
            .into_iter()
            .enumerate()
            .map(|(index, player)| {
                let slot = cx.new(|cx| VideoSlot::new(player, index, cx));

                // Subscribe to VideoEnded events from this slot
                cx.subscribe(&slot, Self::on_video_ended).detach();

                slot
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Should have exactly 4 slots");

        Self { slots }
    }

    /// Handle VideoEnded event from a slot - replace the video.
    fn on_video_ended(
        &mut self,
        slot: Entity<VideoSlot>,
        _event: &VideoEnded,
        cx: &mut Context<Self>,
    ) {
        let index = slot.read(cx).index();
        self.replace_video(index, cx);
    }

    /// Replace the video at the given slot index with a new random video.
    fn replace_video(&mut self, index: usize, cx: &mut Context<Self>) {
        // Get paths and mixer from global state
        let app_state = cx.global::<AppState>();
        let video_paths = app_state.video_paths.clone();
        let mixer = Arc::clone(&app_state.mixer);

        // Get paths of currently playing videos (excluding the one being replaced)
        let current_paths: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, slot)| slot.read(cx).player().path().to_path_buf())
            .collect();

        // Pick a random video that's not currently playing
        let mut rng = rand::thread_rng();
        let available: Vec<_> = video_paths
            .iter()
            .filter(|p| !current_paths.iter().any(|cp| cp == *p))
            .collect();

        let path = if available.is_empty() {
            match video_paths.choose(&mut rng) {
                Some(p) => p,
                None => return, // No videos available
            }
        } else {
            *available.choose(&mut rng).unwrap()
        };

        // Create new player
        let new_player = match VideoPlayer::with_options(path, None, None) {
            Ok(player) => Arc::new(player),
            Err(e) => {
                eprintln!("Failed to create player for {:?}: {}", path, e);
                return;
            }
        };

        println!(
            "Slot {}: replaced with {}",
            index,
            path.file_name().unwrap_or_default().to_string_lossy()
        );

        // Update mixer with new audio consumer
        mixer.set_stream(index, None); // Remove old stream
        if let Some(audio_consumer) = new_player.audio_consumer() {
            mixer.set_stream(index, Some(audio_consumer));
        }

        // Update the player in AppState so pause/resume works
        cx.update_global::<AppState, _>(|state, _cx| {
            state.set_player(index, Arc::clone(&new_player));
        });

        // Create new slot entity and subscribe to its events
        let new_slot = cx.new(|cx| VideoSlot::new(new_player, index, cx));
        cx.subscribe(&new_slot, Self::on_video_ended).detach();

        // Replace the slot
        self.slots[index] = new_slot;

        // Notify that the view needs to re-render
        cx.notify();
    }

    /// Pause all videos in the grid.
    pub fn pause_all(&self, cx: &Context<Self>) {
        for slot in &self.slots {
            slot.read(cx).pause();
        }
    }

    /// Resume all videos in the grid.
    pub fn resume_all(&self, cx: &Context<Self>) {
        for slot in &self.slots {
            slot.read(cx).resume();
        }
    }

    /// Render a single slot at the given index.
    fn render_slot(&self, index: usize, cx: &Context<Self>) -> impl IntoElement {
        let player = self.slots[index].read(cx).player().clone();
        let id = ("video", index);
        div()
            .flex_1()
            .overflow_hidden()
            .child(video_element(player, id))
    }
}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x000000))
            .flex()
            .flex_col()
            .children([
                // Top row
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .children([self.render_slot(0, cx), self.render_slot(1, cx)]),
                // Bottom row
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .children([self.render_slot(2, cx), self.render_slot(3, cx)]),
            ])
    }
}
