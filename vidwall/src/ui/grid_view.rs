use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Render, Window, div, prelude::*, rgb};

use crate::playback::VideoPlayer;
use crate::video::ReadyVideos;

use super::app_state::AppState;
use super::grid_config::GridConfig;
use super::video_element::video_element;
use super::video_slot::{VideoEnded, VideoSlot};

/**
    The main grid view that displays videos in a dynamic grid layout.

    Uses VideoSlot entities for each video position and subscribes to their
    VideoEnded events for automatic video replacement.
    Videos are filtered by orientation to match the grid's orientation.
*/
pub struct GridView {
    slots: Vec<Entity<VideoSlot>>,
    config: GridConfig,
    ready_videos: Arc<ReadyVideos>,
}

impl GridView {
    /**
        Create a new empty grid view that will pull videos from the given storage.
    */
    pub fn new(ready_videos: Arc<ReadyVideos>, _cx: &mut Context<Self>) -> Self {
        Self {
            slots: Vec::new(),
            config: GridConfig::default(),
            ready_videos,
        }
    }

    /**
        Get the current grid configuration.
    */
    pub fn config(&self) -> GridConfig {
        self.config
    }

    /**
        Reconfigure the grid to the new configuration.

        This will clear all slots and refill them if the orientation changes,
        or add/remove slots if only the count changes.
    */
    pub fn reconfigure(&mut self, new_config: GridConfig, cx: &mut Context<Self>) {
        if new_config == self.config && !self.slots.is_empty() {
            return; // No change needed
        }

        let orientation_changed = new_config.orientation != self.config.orientation;
        let old_count = self.slots.len();
        let new_count = new_config.total_slots() as usize;

        if orientation_changed {
            // Clear all slots when orientation changes - we need different videos
            let app_state = cx.global::<AppState>();
            let mixer = Arc::clone(&app_state.mixer);

            for index in 0..old_count {
                mixer.set_stream(index, None);
            }

            // Explicitly stop all players before dropping to release file handles
            for slot in &self.slots {
                slot.read(cx).player().stop();
            }

            self.slots.clear();
            cx.update_global::<AppState, _>(|state, _cx| {
                state.truncate_players(0);
            });

            // Update config first so fill_empty_slots uses the right orientation
            self.config = new_config;

            // Fill with new videos of the correct orientation
            self.fill_empty_slots(cx);
        } else if new_count > old_count {
            self.config = new_config;
            // Add new slots
            for index in old_count..new_count {
                if let Some(slot) = self.create_slot(index, cx) {
                    self.slots.push(slot);
                }
            }
        } else if new_count < old_count {
            // Remove excess slots
            let app_state = cx.global::<AppState>();
            let mixer = Arc::clone(&app_state.mixer);

            for index in new_count..old_count {
                // Clear audio stream for this slot
                mixer.set_stream(index, None);
            }

            // Explicitly stop players being removed to release file handles
            for index in new_count..old_count {
                self.slots[index].read(cx).player().stop();
            }

            // Remove slots and update AppState
            self.slots.truncate(new_count);
            cx.update_global::<AppState, _>(|state, _cx| {
                state.truncate_players(new_count);
            });

            self.config = new_config;
        }

        cx.notify();
    }

    /**
        Try to fill any empty slots with videos from the ready pool.
    */
    pub fn fill_empty_slots(&mut self, cx: &mut Context<Self>) {
        let target_count = self.config.total_slots() as usize;
        let orientation = self.config.orientation;

        // First, ensure we have enough slot entities
        while self.slots.len() < target_count {
            if let Some(slot) = self.create_slot_for_orientation(self.slots.len(), orientation, cx)
            {
                self.slots.push(slot);
            } else {
                break; // No more videos available for this orientation
            }
        }

        cx.notify();
    }

    /**
        Create a new slot at the given index with a video of the specified orientation.
    */
    fn create_slot_for_orientation(
        &self,
        index: usize,
        orientation: crate::ui::grid_config::VideoOrientation,
        cx: &mut Context<Self>,
    ) -> Option<Entity<VideoSlot>> {
        // Get paths of currently playing videos
        let current_paths: Vec<_> = self
            .slots
            .iter()
            .map(|slot| slot.read(cx).video_info().path.clone())
            .collect();

        // Pick a video of the correct orientation not currently playing
        let video_info = self
            .ready_videos
            .pick_random_except_for_orientation(orientation, &current_paths)?;

        // Create the player
        let player = match VideoPlayer::with_options(&video_info.path, None, None) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                eprintln!("Failed to create player: {}", e);
                return None;
            }
        };

        println!(
            "Slot {} ({:?}): {}",
            index,
            orientation,
            video_info
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );

        // Set up audio
        let app_state = cx.global::<AppState>();
        let mixer = Arc::clone(&app_state.mixer);
        if let Some(audio_consumer) = player.audio_consumer() {
            mixer.set_stream(index, Some(audio_consumer));
        }

        // Update AppState with the new player
        cx.update_global::<AppState, _>(|state, _cx| {
            state.set_player(index, Arc::clone(&player));
        });

        // Create the slot entity
        let slot = cx.new(|cx| VideoSlot::new(player, video_info, index, cx));
        cx.subscribe(&slot, Self::on_video_ended).detach();

        Some(slot)
    }

    /**
        Create a new slot using the current grid's orientation.
    */
    fn create_slot(&self, index: usize, cx: &mut Context<Self>) -> Option<Entity<VideoSlot>> {
        self.create_slot_for_orientation(index, self.config.orientation, cx)
    }

    /**
        Handle VideoEnded event from a slot - replace the video.
    */
    fn on_video_ended(
        &mut self,
        slot: Entity<VideoSlot>,
        _event: &VideoEnded,
        cx: &mut Context<Self>,
    ) {
        let index = slot.read(cx).index();
        self.replace_video(index, cx);
    }

    /**
        Replace the video at the given slot index with a new random video.
    */
    fn replace_video(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.slots.len() {
            return;
        }

        let orientation = self.config.orientation;

        // Stop the old player first to release file handles before opening new ones
        self.slots[index].read(cx).player().stop();

        // Get paths of currently playing videos (excluding the one being replaced)
        let current_paths: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, slot)| slot.read(cx).video_info().path.clone())
            .collect();

        // Pick a video of the correct orientation not currently playing
        let video_info = match self
            .ready_videos
            .pick_random_except_for_orientation(orientation, &current_paths)
        {
            Some(info) => info,
            None => return, // No videos available for this orientation
        };

        // Create new player
        let new_player = match VideoPlayer::with_options(&video_info.path, None, None) {
            Ok(player) => Arc::new(player),
            Err(e) => {
                eprintln!("Failed to create player for {:?}: {}", video_info.path, e);
                return;
            }
        };

        println!(
            "Slot {} ({:?}): replaced with {}",
            index,
            orientation,
            video_info
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );

        // Update mixer with new audio consumer
        let app_state = cx.global::<AppState>();
        let mixer = Arc::clone(&app_state.mixer);
        mixer.set_stream(index, None); // Remove old stream
        if let Some(audio_consumer) = new_player.audio_consumer() {
            mixer.set_stream(index, Some(audio_consumer));
        }

        // Update the player in AppState
        cx.update_global::<AppState, _>(|state, _cx| {
            state.set_player(index, Arc::clone(&new_player));
        });

        // Create new slot entity and subscribe to its events
        let new_slot = cx.new(|cx| VideoSlot::new(new_player, video_info, index, cx));
        cx.subscribe(&new_slot, Self::on_video_ended).detach();

        // Replace the slot
        self.slots[index] = new_slot;
        cx.notify();
    }

    /**
        Skip all videos and load new ones.
    */
    pub fn skip_all(&mut self, cx: &mut Context<Self>) {
        if self.slots.is_empty() {
            return;
        }

        // Clear audio streams
        let app_state = cx.global::<AppState>();
        let mixer = Arc::clone(&app_state.mixer);
        for index in 0..self.slots.len() {
            mixer.set_stream(index, None);
        }

        // Explicitly stop all players before dropping them to ensure file handles are released
        for slot in &self.slots {
            slot.read(cx).player().stop();
        }

        // Clear all slots
        self.slots.clear();
        cx.update_global::<AppState, _>(|state, _cx| {
            state.truncate_players(0);
        });

        // Refill with new videos
        self.fill_empty_slots(cx);
        cx.notify();
    }

    /**
        Pause all videos in the grid.
    */
    pub fn pause_all(&self, cx: &Context<Self>) {
        for slot in &self.slots {
            slot.read(cx).pause();
        }
    }

    /**
        Resume all videos in the grid.
    */
    pub fn resume_all(&self, cx: &Context<Self>) {
        for slot in &self.slots {
            slot.read(cx).resume();
        }
    }

    /**
        Render a single slot at the given index.
    */
    fn render_slot(&self, index: usize, cx: &Context<Self>) -> impl IntoElement {
        let slot = &self.slots[index];
        let slot_data = slot.read(cx);
        let player = slot_data.player().clone();
        let aspect_ratio = slot_data.video_info().aspect_ratio();
        let id = ("video", index);

        div()
            .flex_1()
            .overflow_hidden()
            .child(video_element(player, aspect_ratio, id))
    }
}

impl Render for GridView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Try to fill empty slots if videos of the right orientation are available
        let target_slots = self.config.total_slots() as usize;
        let orientation = self.config.orientation;
        if self.slots.len() < target_slots
            && self.ready_videos.has_videos_for_orientation(orientation)
        {
            self.fill_empty_slots(cx);
        }

        let cols = self.config.cols as usize;
        let rows = self.config.rows as usize;

        // Build rows of video slots
        let mut row_elements: Vec<_> = Vec::new();

        for row in 0..rows {
            let mut col_elements: Vec<_> = Vec::new();
            for col in 0..cols {
                let index = row * cols + col;
                if index < self.slots.len() {
                    col_elements.push(self.render_slot(index, cx).into_any_element());
                } else {
                    // Empty slot placeholder (black)
                    col_elements.push(div().flex_1().bg(rgb(0x000000)).into_any_element());
                }
            }

            row_elements.push(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .children(col_elements)
                    .into_any_element(),
            );
        }

        div()
            .size_full()
            .bg(rgb(0x000000))
            .flex()
            .flex_col()
            .children(row_elements)
    }
}
