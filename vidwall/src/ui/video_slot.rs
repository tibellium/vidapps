use std::sync::Arc;
use std::time::Duration;

use gpui::{AsyncApp, Context, EventEmitter};

use crate::playback::VideoPlayer;
use crate::video::VideoInfo;

/**
    Interval for checking if a video has ended
*/
const MONITOR_INTERVAL: Duration = Duration::from_millis(100);

/**
    Event emitted when a video slot's video has finished playing.
*/
pub struct VideoEnded;

/**
    A video slot entity that owns a video player and emits events.

    Each slot monitors its player and emits `VideoEnded` when playback completes.
    This allows the parent GridView to subscribe and handle video replacement.
*/
pub struct VideoSlot {
    /// The video player for this slot
    player: Arc<VideoPlayer>,
    /// Metadata about the video (including aspect ratio)
    video_info: VideoInfo,
    /// Index of this slot in the grid
    index: usize,
}

impl EventEmitter<VideoEnded> for VideoSlot {}

impl VideoSlot {
    /**
        Create a new video slot with the given player, video info, and index.

        Automatically starts a background task to monitor for video end.
    */
    pub fn new(
        player: Arc<VideoPlayer>,
        video_info: VideoInfo,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        let slot = Self {
            player,
            video_info,
            index,
        };
        slot.start_monitor(cx);
        slot
    }

    /**
        Get the video player for this slot.
    */
    pub fn player(&self) -> &Arc<VideoPlayer> {
        &self.player
    }

    /**
        Get the video metadata for this slot.
    */
    pub fn video_info(&self) -> &VideoInfo {
        &self.video_info
    }

    /**
        Get the index of this slot in the grid.
    */
    pub fn index(&self) -> usize {
        self.index
    }

    /**
        Pause this slot's video playback.
    */
    pub fn pause(&self) {
        self.player.pause();
    }

    /**
        Resume this slot's video playback.
    */
    pub fn resume(&self) {
        self.player.resume();
    }

    /**
        Check if this slot's video is paused.
    */
    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    /**
        Check if this slot's video has ended.
    */
    pub fn is_ended(&self) -> bool {
        self.player.is_ended()
    }

    /**
        Start the background task that monitors for video end.
    */
    fn start_monitor(&self, cx: &mut Context<Self>) {
        // Clone the player for the async task to check
        let player = Arc::clone(&self.player);

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                // Wait for the monitoring interval
                cx.background_executor().timer(MONITOR_INTERVAL).await;

                // Check if video has ended
                if player.is_ended() {
                    // Try to emit the event back on the main thread
                    let result = this.update(cx, |_slot, cx: &mut Context<VideoSlot>| {
                        cx.emit(VideoEnded);
                    });

                    if result.is_err() {
                        // Entity was dropped
                    }
                    break; // Stop monitoring after emitting
                }
            }
        })
        .detach();
    }
}
