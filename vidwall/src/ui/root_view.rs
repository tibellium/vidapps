use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Context, Entity, IntoElement, Pixels, PlatformDisplay, Point, Render, Size, Timer, Window, div,
    prelude::*, rgb,
};

use crate::video::ReadyVideos;
use crate::window_state::WindowState;

use super::app_state::AppState;
use super::grid_config::GridConfig;
use super::grid_view::GridView;
use super::welcome_view::{VideosSelected, WelcomeView};

/**
    Minimum time between window state saves (to avoid excessive disk writes during resize)
*/
const SAVE_DEBOUNCE_SECS: f32 = 1.0;

/**
    How often to check for new videos (in milliseconds)
*/
const VIDEO_POLL_INTERVAL_MS: u64 = 100;

/**
    The current view state of the application.
*/
enum ViewState {
    /// Welcome screen - waiting for user to select videos.
    Welcome(Entity<WelcomeView>),
    /// Grid view - playing videos.
    Grid {
        grid: Entity<GridView>,
        ready_videos: Arc<ReadyVideos>,
        last_video_count: usize,
    },
}

/**
    The root view of the application.

    Contains either a welcome screen or the video grid, and handles window resize events.
*/
pub struct RootView {
    state: ViewState,
    last_size: Option<Size<Pixels>>,
    last_origin: Option<Point<Pixels>>,
    last_save_time: Option<Instant>,
}

impl RootView {
    /**
        Create a new root view with the welcome screen (no videos yet).
    */
    pub fn new_welcome(cx: &mut Context<Self>) -> Self {
        let welcome = cx.new(|_cx| WelcomeView::new());

        // Subscribe to VideosSelected events from the welcome view
        cx.subscribe(&welcome, Self::on_videos_selected).detach();

        Self {
            state: ViewState::Welcome(welcome),
            last_size: None,
            last_origin: None,
            last_save_time: None,
        }
    }

    /**
        Create a new root view directly with videos (for CLI paths).
    */
    pub fn new_with_videos(ready_videos: Arc<ReadyVideos>, cx: &mut Context<Self>) -> Self {
        let ready_videos_clone = Arc::clone(&ready_videos);
        let grid = cx.new(|cx| GridView::new(ready_videos_clone, cx));

        // Start polling for new videos
        Self::start_video_polling(cx);

        Self {
            state: ViewState::Grid {
                grid,
                ready_videos,
                last_video_count: 0,
            },
            last_size: None,
            last_origin: None,
            last_save_time: None,
        }
    }

    /**
        Handle VideosSelected event from the welcome view.
    */
    fn on_videos_selected(
        &mut self,
        _welcome: Entity<WelcomeView>,
        event: &VideosSelected,
        cx: &mut Context<Self>,
    ) {
        self.transition_to_grid(event.paths.clone(), cx);
    }

    /**
        Transition from welcome screen to grid view after paths are selected.
    */
    fn transition_to_grid(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // Initialize video playback system (deref Context to App)
        let ready_videos = crate::initialize_video_playback(paths.clone(), &mut **cx);

        // Update window title - we'll do this in render when we have window access
        let window_title = if paths.len() == 1 {
            paths[0]
                .file_name()
                .map(|s| format!("Video Wall - {}", s.to_string_lossy()))
                .unwrap_or_else(|| "Video Wall".to_string())
        } else {
            format!("Video Wall - {} sources", paths.len())
        };

        // Create grid view
        let ready_videos_clone = Arc::clone(&ready_videos);
        let grid = cx.new(|cx| GridView::new(ready_videos_clone, cx));

        // Start polling for new videos
        Self::start_video_polling(cx);

        // Switch state - store the title for updating in render
        self.state = ViewState::Grid {
            grid,
            ready_videos,
            last_video_count: 0,
        };

        // Store title to set on next render
        cx.spawn({
            let title = window_title;
            async move |_this, cx| {
                cx.update(|cx| {
                    if let Some(window) = cx.active_window() {
                        window
                            .update(cx, |_, window, _cx| {
                                window.set_window_title(&title);
                            })
                            .ok();
                    }
                })
                .ok();
            }
        })
        .detach();

        cx.notify();
    }

    /**
        Start the background polling task for new videos.
    */
    fn start_video_polling(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(VIDEO_POLL_INTERVAL_MS)).await;
                let should_stop = cx
                    .update(|cx| {
                        this.update(cx, |this, cx| this.check_for_new_videos(cx))
                            .unwrap_or(true)
                    })
                    .unwrap_or(true);
                if should_stop {
                    break;
                }
            }
        })
        .detach();
    }

    /**
        Check if new videos have been added and fill empty slots.
        Returns true if polling should stop (grid is full).
    */
    fn check_for_new_videos(&mut self, cx: &mut Context<Self>) -> bool {
        let ViewState::Grid {
            grid,
            ready_videos,
            last_video_count,
        } = &mut self.state
        else {
            return true; // Stop polling if not in grid state
        };

        let current_count = ready_videos.len();
        if current_count > *last_video_count {
            *last_video_count = current_count;
            grid.update(cx, |grid, cx| {
                grid.fill_empty_slots(cx);
            });
        }

        // Stop polling once we have enough videos to fill the grid
        let grid_slots = grid.read(cx).config().total_slots() as usize;
        current_count >= grid_slots && grid_slots > 0
    }

    /**
        Handle window resize by reconfiguring the grid if needed.
    */
    fn handle_resize(&self, size: Size<Pixels>, cx: &mut Context<Self>) {
        let ViewState::Grid { grid, .. } = &self.state else {
            return;
        };

        // Calculate optimal grid for new size
        let new_config = GridConfig::optimal_for_window(size.width.into(), size.height.into());

        // Reconfigure grid if needed
        grid.update(cx, |grid, cx| {
            grid.reconfigure(new_config, cx);
        });
    }

    /**
        Save window state to disk (debounced).
    */
    fn maybe_save_window_state(
        &mut self,
        display: Option<Rc<dyn PlatformDisplay>>,
        origin: Point<Pixels>,
        size: Size<Pixels>,
    ) {
        // Check if state actually changed
        if self.last_size == Some(size) && self.last_origin == Some(origin) {
            return;
        }
        self.last_size = Some(size);
        self.last_origin = Some(origin);

        // Debounce saves
        let now = Instant::now();
        if let Some(last_save) = self.last_save_time {
            if now.duration_since(last_save).as_secs_f32() < SAVE_DEBOUNCE_SECS {
                return;
            }
        }

        // Save the state
        let state = WindowState::new(display, origin, size);
        if let Err(e) = state.save() {
            eprintln!("Failed to save window state: {}", e);
        }
        self.last_save_time = Some(now);
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Get current window info for state saving
        let size = window.viewport_size();
        let origin = window.bounds().origin;
        let display = window.display(&*cx);

        // Check if size changed (before updating last_size in save)
        let size_changed = self.last_size != Some(size);

        // Save window state (common to both views)
        self.maybe_save_window_state(display, origin, size);

        match &self.state {
            ViewState::Welcome(welcome) => div()
                .id("root")
                .size_full()
                .bg(rgb(0x111111))
                .child(welcome.clone()),
            ViewState::Grid { grid, .. } => {
                // Handle resize for grid
                if size_changed {
                    self.handle_resize(size, cx);
                }

                // Check if skip all was requested (AppState exists in grid mode)
                let skip_requested =
                    cx.update_global::<AppState, _>(|state, _cx| state.take_skip_all_request());
                if skip_requested {
                    grid.update(cx, |grid, cx| {
                        grid.skip_all(cx);
                    });
                }

                div()
                    .id("root")
                    .size_full()
                    .bg(rgb(0x000000))
                    .child(grid.clone())
            }
        }
    }
}
