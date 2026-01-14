/*!
    2x2 Randomized Video Grid Player

    Opens a folder, finds all videos, and plays 4 random videos in a grid.
    When a video ends, it's replaced with a new random video from the folder.

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
*/

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, AppContext, Application, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use rand::seq::SliceRandom;
use walkdir::WalkDir;

mod audio;
mod decode;
mod playback;
mod ui;

use audio::{AudioMixer, AudioOutput, DEFAULT_CHANNELS, DEFAULT_SAMPLE_RATE};
use playback::VideoPlayer;
use ui::{AppState, RootView, register_shortcuts};

// Supported video extensions
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv", "flv", "mpeg", "mpg", "3gp", "ts", "mts",
];

// Default window dimensions
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

/// Recursively scan a directory for video files
fn scan_for_videos(folder: &PathBuf) -> Vec<PathBuf> {
    WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

/// Pick n random videos from the list (with repetition if fewer than n videos)
fn pick_random_videos(videos: &[PathBuf], n: usize) -> Vec<PathBuf> {
    let mut rng = rand::thread_rng();

    if videos.len() >= n {
        videos.choose_multiple(&mut rng, n).cloned().collect()
    } else {
        (0..n)
            .map(|_| videos.choose(&mut rng).unwrap().clone())
            .collect()
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        // Register keyboard shortcuts at the app level
        register_shortcuts(cx);

        let folder_path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);

        if let Some(path) = folder_path {
            open_grid_with_folder(path, cx);
        } else {
            let future = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Select video folder".into()),
            });

            cx.spawn(async |cx: &mut AsyncApp| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if let Some(path) = paths.into_iter().next() {
                        cx.update(|cx| {
                            open_grid_with_folder(path, cx);
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
    });
}

fn open_grid_with_folder(folder: PathBuf, cx: &mut App) {
    let all_videos = scan_for_videos(&folder);

    if all_videos.is_empty() {
        eprintln!("No video files found in {:?}", folder);
        eprintln!("Supported formats: {}", VIDEO_EXTENSIONS.join(", "));
        cx.quit();
        return;
    }

    println!("Found {} videos in {:?}", all_videos.len(), folder);

    // Initialize audio mixer
    let mixer = Arc::new(AudioMixer::new(DEFAULT_SAMPLE_RATE, DEFAULT_CHANNELS));

    // Pick initial 4 random videos
    let selected = pick_random_videos(&all_videos, 4);
    println!("Initial videos:");
    for (i, path) in selected.iter().enumerate() {
        println!(
            "  [{}] {}",
            i,
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    // Create VideoPlayer instances
    let players: Result<Vec<Arc<VideoPlayer>>, _> = selected
        .iter()
        .map(|path| VideoPlayer::with_options(path, None, None).map(Arc::new))
        .collect();

    let players = match players {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create video players: {}", e);
            cx.quit();
            return;
        }
    };

    // Register audio consumers with the mixer
    for (i, player) in players.iter().enumerate() {
        if let Some(audio_consumer) = player.audio_consumer() {
            mixer.set_stream(i, Some(audio_consumer));
            println!("  [{}] Audio stream registered", i);
        }
    }

    let players_array: [Arc<VideoPlayer>; 4] = match players.clone().try_into() {
        Ok(arr) => arr,
        Err(_) => panic!("Should have exactly 4 players"),
    };

    // Set up global application state (includes players for pause/resume)
    cx.set_global(AppState::new(all_videos, Arc::clone(&mixer), players));

    let folder_name = folder
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Video Grid".to_string());

    let bounds = Bounds::centered(
        None,
        size(px(DEFAULT_WIDTH as f32), px(DEFAULT_HEIGHT as f32)),
        cx,
    );

    // Store audio output in a Box::leak to keep it alive for app lifetime
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

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            focus: true,
            kind: gpui::WindowKind::PopUp,
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(format!("Video Grid - {}", folder_name).into()),
                appears_transparent: false,
                ..Default::default()
            }),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| RootView::new(players_array, cx)),
    )
    .expect("Failed to open window");

    cx.activate(true);

    println!("\nKeyboard shortcuts:");
    println!("  Space  - Pause/Resume");
    println!("  M      - Mute/Unmute");
    println!("  Up     - Volume up");
    println!("  Down   - Volume down");
    println!("  Cmd+Q  - Quit");
}
