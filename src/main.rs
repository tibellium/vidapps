/*!
    Dynamic Video Grid Player

    Select video files and/or folders, and plays random videos in a dynamic grid.
    The grid adapts to the window's aspect ratio in real-time.
    When a video ends, it's replaced with a new random video from the pool.
    Folders are scanned recursively for video files using ffprobe validation.

    Keyboard Controls:
    - Space: Pause/Resume all videos
    - M: Mute/Unmute audio
    - Up/Down: Adjust volume
    - Cmd+Q: Quit

    Prerequisites:
    - FFmpeg: `brew install ffmpeg`

    Usage:
      cargo run --release
      cargo run --release -- /path/to/videos
      cargo run --release -- /path/to/folder1 /path/to/video.mp4 /path/to/folder2
*/

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, AppContext, Application, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use rand::seq::SliceRandom;

mod audio;
mod decode;
mod playback;
mod ui;
mod video;
mod window_state;

use audio::{AudioMixer, AudioOutput, DEFAULT_CHANNELS, DEFAULT_SAMPLE_RATE};
use ui::{AppState, RootView, register_shortcuts};
use video::{ReadyVideos, VideoScanner};
use window_state::WindowState;

// Default window dimensions
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

fn main() {
    Application::new().run(|cx: &mut App| {
        // Register keyboard shortcuts at the app level
        register_shortcuts(cx);

        let cli_paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();

        if !cli_paths.is_empty() {
            open_app_with_paths(cli_paths, cx);
        } else {
            let future = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: true,
                multiple: true,
                prompt: Some("Select videos or folders".into()),
            });

            cx.spawn(async |cx: &mut AsyncApp| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if !paths.is_empty() {
                        cx.update(|cx| {
                            open_app_with_paths(paths, cx);
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
    });
}

fn open_app_with_paths(paths: Vec<PathBuf>, cx: &mut App) {
    // Initialize shared resources
    let ready_videos = Arc::new(ReadyVideos::new());
    let mixer = Arc::new(AudioMixer::new(DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS));

    // Set up global application state
    cx.set_global(AppState::new(Arc::clone(&ready_videos), Arc::clone(&mixer)));

    // Initialize audio output
    let audio_output = match AudioOutput::new(Arc::clone(&mixer)) {
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
    // Leak the audio output to keep it alive
    if let Some(output) = audio_output {
        Box::leak(output);
    }

    // Determine window title
    let window_title = if paths.len() == 1 {
        paths[0]
            .file_name()
            .map(|s| format!("Video Grid - {}", s.to_string_lossy()))
            .unwrap_or_else(|| "Video Grid".to_string())
    } else {
        format!("Video Grid - {} sources", paths.len())
    };

    // Try to load saved window state, or use defaults
    let (bounds, display_id) = if let Some(saved_state) = WindowState::load() {
        println!("Restored window state from saved state");
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

    // Open window with empty grid (will be filled as videos are validated)
    let ready_videos_for_window = Arc::clone(&ready_videos);
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
            |_window, cx| cx.new(|cx| RootView::new(ready_videos_for_window, cx)),
        )
        .expect("Failed to open window");

    // Save window bounds when closed
    // We need to save the bounds before the window is destroyed, so we do it via
    // observing window bounds changes in RootView instead
    let _ = window;

    cx.activate(true);

    println!("\nKeyboard shortcuts:");
    println!("  Space  - Pause/Resume");
    println!("  M      - Mute/Unmute");
    println!("  Up     - Volume up");
    println!("  Down   - Volume down");
    println!("  Cmd+Q  - Quit");

    // Start video scanning in the background
    let scanner = VideoScanner::new(Arc::clone(&ready_videos));
    let mut candidates = scanner.scan_paths(paths.clone());

    // Shuffle candidates for fairness across different sources
    candidates.shuffle(&mut rand::thread_rng());

    println!(
        "\nScanning {} candidate file(s) from {} path(s)...",
        candidates.len(),
        paths.len()
    );

    // Process videos in parallel using 8 worker threads
    let ready_videos_for_scan = Arc::clone(&ready_videos);
    std::thread::spawn(move || {
        VideoScanner::probe_all_parallel(ready_videos_for_scan, candidates);
    });
}
