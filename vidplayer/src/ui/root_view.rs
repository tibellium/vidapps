use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    Context, Entity, IntoElement, Pixels, PlatformDisplay, Point, Render, Size, Window, div,
    prelude::*, rgb,
};

use crate::initialize_audio_output;
use crate::playback::VideoPlayer;
use crate::window_state::WindowState;

use super::app_state::AppState;
use super::player_view::PlayerView;
use super::welcome_view::{VideoSelected, WelcomeView};

const SAVE_DEBOUNCE_SECS: f32 = 1.0;

enum ViewState {
    Welcome(Entity<WelcomeView>),
    Player(Entity<PlayerView>),
}

pub struct RootView {
    state: ViewState,
    last_size: Option<Size<Pixels>>,
    last_origin: Option<Point<Pixels>>,
    last_save_time: Option<Instant>,
}

impl RootView {
    pub fn new_welcome(cx: &mut Context<Self>) -> Self {
        let welcome = cx.new(|_cx| WelcomeView::new());
        cx.subscribe(&welcome, Self::on_video_selected).detach();

        Self {
            state: ViewState::Welcome(welcome),
            last_size: None,
            last_origin: None,
            last_save_time: None,
        }
    }

    pub fn new_with_video(path: PathBuf, cx: &mut Context<Self>) -> Self {
        match VideoPlayer::new(&path) {
            Ok(player) => {
                let player = Arc::new(player);

                // Update app state
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.set_player(Arc::clone(&player));
                });

                let player_view = cx.new(|_cx| PlayerView::new(player));

                Self {
                    state: ViewState::Player(player_view),
                    last_size: None,
                    last_origin: None,
                    last_save_time: None,
                }
            }
            Err(e) => {
                eprintln!("Failed to open video: {}", e);
                // Fall back to welcome view
                let welcome = cx.new(|_cx| WelcomeView::new());
                cx.subscribe(&welcome, Self::on_video_selected).detach();

                Self {
                    state: ViewState::Welcome(welcome),
                    last_size: None,
                    last_origin: None,
                    last_save_time: None,
                }
            }
        }
    }

    fn on_video_selected(
        &mut self,
        _welcome: Entity<WelcomeView>,
        event: &VideoSelected,
        cx: &mut Context<Self>,
    ) {
        self.transition_to_player(event.path.clone(), cx);
    }

    fn transition_to_player(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Initialize app state if not already done
        if !cx.has_global::<AppState>() {
            cx.set_global(AppState::new());
            initialize_audio_output(&mut **cx);
        }

        match VideoPlayer::new(&path) {
            Ok(player) => {
                let player = Arc::new(player);

                // Update window title
                let window_title = path
                    .file_name()
                    .map(|s| format!("Video Player - {}", s.to_string_lossy()))
                    .unwrap_or_else(|| "Video Player".to_string());

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

                // Update app state with player
                cx.update_global::<AppState, _>(|state, _cx| {
                    state.set_player(Arc::clone(&player));
                });

                // Create player view
                let player_view = cx.new(|_cx| PlayerView::new(player));
                self.state = ViewState::Player(player_view);

                println!("\nKeyboard shortcuts:");
                println!("  Space      - Pause/Resume");
                println!("  M          - Mute/Unmute");
                println!("  Up         - Volume up");
                println!("  Down       - Volume down");
                println!("  Left       - Seek backward 10s");
                println!("  Right      - Seek forward 10s");
                println!("  Shift+Left - Seek backward 30s");
                println!("  Shift+Right- Seek forward 30s");
                println!("  Cmd+Q      - Quit");

                cx.notify();
            }
            Err(e) => {
                eprintln!("Failed to open video: {}", e);
            }
        }
    }

    fn maybe_save_window_state(
        &mut self,
        display: Option<Rc<dyn PlatformDisplay>>,
        origin: Point<Pixels>,
        size: Size<Pixels>,
    ) {
        if self.last_size == Some(size) && self.last_origin == Some(origin) {
            return;
        }
        self.last_size = Some(size);
        self.last_origin = Some(origin);

        let now = Instant::now();
        if let Some(last_save) = self.last_save_time {
            if now.duration_since(last_save).as_secs_f32() < SAVE_DEBOUNCE_SECS {
                return;
            }
        }

        let state = WindowState::new(display, origin, size);
        if let Err(e) = state.save() {
            eprintln!("Failed to save window state: {}", e);
        }
        self.last_save_time = Some(now);
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let origin = window.bounds().origin;
        let display = window.display(&*cx);

        self.maybe_save_window_state(display, origin, size);

        match &self.state {
            ViewState::Welcome(welcome) => div()
                .id("root")
                .size_full()
                .bg(rgb(0x111111))
                .child(welcome.clone()),
            ViewState::Player(player) => div()
                .id("root")
                .size_full()
                .bg(rgb(0x000000))
                .child(player.clone()),
        }
    }
}
