/*!
    Single Video Player

    A simple video player application that plays a single video file.
    Select a video file and it plays in a window with standard playback controls.

    Keyboard Controls:
    - Space: Pause/Resume
    - M: Mute/Unmute
    - Up/Down: Adjust volume
    - Left/Right: Seek backward/forward
    - Cmd+Q: Quit

    Prerequisites:
    - FFmpeg: `brew install ffmpeg`

    Usage:
      cargo run --release
      cargo run --release -- /path/to/video.mp4
*/

use std::path::PathBuf;

use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};

mod audio;
mod decode;
mod playback;
mod ui;
mod window_state;

use audio::{AudioOutput, DEFAULT_CHANNELS, DEFAULT_SAMPLE_RATE};
use ui::{AppState, RootView, register_shortcuts};
use window_state::WindowState;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

fn main() {
    Application::new().run(|cx: &mut App| {
        register_shortcuts(cx);

        let cli_path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

        if let Some(path) = cli_path {
            open_app_with_video(path, cx);
        } else {
            open_app_with_welcome(cx);
        }
    });
}

fn open_app_with_welcome(cx: &mut App) {
    let (bounds, display_id) = if let Some(saved_state) = WindowState::load() {
        let display_id = saved_state.display_id(cx);
        let bounds = saved_state.to_bounds(cx);
        (bounds, display_id)
    } else {
        let bounds = Bounds::centered(
            None,
            size(px(DEFAULT_WIDTH as f32), px(DEFAULT_HEIGHT as f32)),
            cx,
        );
        (bounds, None)
    };

    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id,
                focus: true,
                kind: gpui::WindowKind::PopUp,
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Video Player".into()),
                    appears_transparent: false,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| cx.new(RootView::new_welcome),
        )
        .expect("Failed to open window");

    let _ = window;
    cx.activate(true);
}

fn open_app_with_video(path: PathBuf, cx: &mut App) {
    let window_title = path
        .file_name()
        .map(|s| format!("Video Player - {}", s.to_string_lossy()))
        .unwrap_or_else(|| "Video Player".to_string());

    let (bounds, display_id) = if let Some(saved_state) = WindowState::load() {
        let display_id = saved_state.display_id(cx);
        let bounds = saved_state.to_bounds(cx);
        (bounds, display_id)
    } else {
        let bounds = Bounds::centered(
            None,
            size(px(DEFAULT_WIDTH as f32), px(DEFAULT_HEIGHT as f32)),
            cx,
        );
        (bounds, None)
    };

    // Initialize app state (will be populated when player is created)
    cx.set_global(AppState::new());

    // Initialize audio output
    let app_state = cx.global::<AppState>();
    let audio_consumer = app_state.audio_consumer.clone();
    let audio_output = match AudioOutput::new(audio_consumer) {
        Ok(output) => {
            eprintln!(
                "Audio output initialized ({}Hz, {} channels)",
                DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS
            );
            Some(Box::new(output))
        }
        Err(e) => {
            eprintln!("Warning: Failed to initialize audio output: {}", e);
            eprintln!("Video will play without audio");
            None
        }
    };
    if let Some(output) = audio_output {
        Box::leak(output);
    }

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

    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id,
                focus: true,
                kind: gpui::WindowKind::PopUp,
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(window_title.into()),
                    appears_transparent: false,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| cx.new(|cx| RootView::new_with_video(path, cx)),
        )
        .expect("Failed to open window");

    let _ = window;
    cx.activate(true);
}

pub fn initialize_audio_output(cx: &mut App) {
    let app_state = cx.global::<AppState>();
    let audio_consumer = app_state.audio_consumer.clone();

    let audio_output = match AudioOutput::new(audio_consumer) {
        Ok(output) => {
            eprintln!(
                "Audio output initialized ({}Hz, {} channels)",
                DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS
            );
            Some(Box::new(output))
        }
        Err(e) => {
            eprintln!("Warning: Failed to initialize audio output: {}", e);
            None
        }
    };
    if let Some(output) = audio_output {
        Box::leak(output);
    }
}
