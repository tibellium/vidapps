//! 2x2 Randomized Video Grid Player
//!
//! Opens a folder, finds all videos, and plays 4 random videos in a grid.
//! When a video ends, it's replaced with a new random video from the folder.
//!
//! Prerequisites:
//! - GStreamer: `brew install gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly`
//! - Xcode (for Metal on macOS)
//!
//! Usage:
//!   cargo run --release
//!   cargo run --release -- /path/to/videos

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, AsyncApp, Bounds, Context, SharedString,
    WeakEntity, Window, WindowBounds, WindowOptions,
};
use gpui_video_player::{video, Video, VideoOptions};
use rand::seq::SliceRandom;
use std::path::PathBuf;
use std::time::Duration;
use url::Url;
use walkdir::WalkDir;

// Supported video extensions
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv", "flv", "mpeg", "mpg", "3gp", "ts", "mts",
];

// How often to check if videos have ended
const POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// Pick one random video from the list
fn pick_random_video(videos: &[PathBuf]) -> PathBuf {
    let mut rng = rand::thread_rng();
    videos.choose(&mut rng).unwrap().clone()
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

/// Create a Video instance from a path (no looping, muted)
fn create_video(path: &PathBuf) -> Result<Video, Box<dyn std::error::Error>> {
    let uri = Url::from_file_path(path).map_err(|_| "Invalid file path")?;

    // No looping - we'll swap videos when they end
    let options = VideoOptions {
        looping: Some(false),
        ..VideoOptions::default()
    };

    let video = Video::new_with_options(&uri, options)?;
    video.set_muted(true);
    Ok(video)
}

/// The main view holding our 4 video players
struct VideoGridView {
    videos: [Video; 4],
    all_video_paths: Vec<PathBuf>,
    #[allow(dead_code)]
    folder_name: SharedString,
}

impl VideoGridView {
    fn new(videos: [Video; 4], all_video_paths: Vec<PathBuf>, folder_name: String, cx: &mut Context<Self>) -> Self {
        // Start the polling task to check for ended videos
        cx.spawn(async |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;

                let should_continue = view
                    .update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                        this.check_and_replace_ended_videos(cx);
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
            videos,
            all_video_paths,
            folder_name: folder_name.into(),
        }
    }

    /// Update video display sizes based on cell dimensions
    fn update_video_sizes(&self, cell_width: u32, cell_height: u32) {
        for video in &self.videos {
            video.set_display_width(Some(cell_width));
            video.set_display_height(Some(cell_height));
        }
    }

    /// Check each video slot and replace any that have finished
    fn check_and_replace_ended_videos(&mut self, cx: &mut Context<Self>) {
        let mut any_replaced = false;

        for i in 0..4 {
            if self.video_has_ended(i) {
                if let Ok(new_video) = self.create_replacement_video() {
                    self.videos[i] = new_video;
                    any_replaced = true;
                    println!("Slot {}: replaced with new video", i);
                }
            }
        }

        if any_replaced {
            cx.notify();
        }
    }

    /// Check if a video at the given slot has ended
    fn video_has_ended(&self, slot: usize) -> bool {
        let video = &self.videos[slot];

        // Get position and duration - these return Duration directly
        let position = video.position();
        let duration = video.duration();

        // Video has ended if position >= duration (with small tolerance)
        // Also check if duration is valid (> 0)
        if duration > Duration::ZERO {
            // Consider ended if within 100ms of the end
            let threshold = duration.saturating_sub(Duration::from_millis(100));
            return position >= threshold;
        }

        false
    }

    /// Create a new random video from the available paths
    fn create_replacement_video(&self) -> Result<Video, Box<dyn std::error::Error>> {
        let path = pick_random_video(&self.all_video_paths);
        create_video(&path)
    }
}

impl Render for VideoGridView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Get window size and calculate cell dimensions (2 columns, 2 rows)
        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        let window_height: f32 = bounds.size.height.into();

        let cell_width = (window_width / 2.0) as u32;
        let cell_height = (window_height / 2.0) as u32;

        // Update video display sizes to match cells
        self.update_video_sizes(cell_width, cell_height);

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
                    .children([
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                video(self.videos[0].clone())
                                    .id("video-0")
                                    .buffer_capacity(30),
                            ),
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                video(self.videos[1].clone())
                                    .id("video-1")
                                    .buffer_capacity(30),
                            ),
                    ]),
                // Bottom row
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .children([
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                video(self.videos[2].clone())
                                    .id("video-2")
                                    .buffer_capacity(30),
                            ),
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                video(self.videos[3].clone())
                                    .id("video-3")
                                    .buffer_capacity(30),
                            ),
                    ]),
            ])
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
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

    // Pick initial 4 random videos
    let selected = pick_random_videos(&all_videos, 4);
    println!("Initial videos:");
    for (i, path) in selected.iter().enumerate() {
        println!("  [{}] {}", i, path.file_name().unwrap_or_default().to_string_lossy());
    }

    // Create Video instances
    let video_players: Result<Vec<Video>, _> = selected.iter().map(create_video).collect();

    let video_players = match video_players {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to create video players: {}", e);
            cx.quit();
            return;
        }
    };

    let videos_array: [Video; 4] = video_players
        .try_into()
        .expect("Should have exactly 4 videos");

    let folder_name = folder
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Video Grid".to_string());

    let bounds = Bounds::centered(None, size(px(1280.0), px(720.0)), cx);

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
        |_, cx| cx.new(|cx| VideoGridView::new(videos_array, all_videos, folder_name, cx)),
    )
    .expect("Failed to open window");

    cx.activate(true);
}
