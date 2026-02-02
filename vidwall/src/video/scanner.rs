use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;

use walkdir::WalkDir;

use super::{ReadyVideos, probe_video};

/**
    Supported video file extensions for quick pre-filtering.
*/
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "webm", "m4v", "wmv", "flv", "mpeg", "mpg", "3gp", "ts", "mts",
];

/**
    Number of worker threads for parallel ffprobe validation.
*/
const NUM_WORKERS: usize = 4;

/**
    Scanner that discovers and validates video files in the background.
*/
pub struct VideoScanner {
    /// The ready videos storage that validated videos are pushed to
    pub ready_videos: Arc<ReadyVideos>,
}

impl VideoScanner {
    /**
        Create a new VideoScanner that will push validated videos to the given storage.
    */
    pub fn new(ready_videos: Arc<ReadyVideos>) -> Self {
        Self { ready_videos }
    }

    /**
        Scan the given paths for video files.

        This performs a quick extension-based pre-filter, then validates each
        candidate file using ffprobe in the background.

        The `on_video_ready` callback is called on the main thread whenever
        a new video is validated and added to storage.
    */
    pub fn scan_paths(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        // Quick pre-filter: collect all files with video extensions
        let candidates = Self::collect_video_candidates(&paths);
        candidates
    }

    /**
        Probe a single video file and add it to ready videos if valid.

        Returns true if the video was successfully validated and added.
    */
    pub fn probe_and_add(&self, path: &PathBuf) -> bool {
        match probe_video(path) {
            Ok(info) => {
                println!(
                    "  Validated: {} ({}x{})",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    info.width,
                    info.height
                );
                self.ready_videos.push(info);
                true
            }
            Err(e) => {
                eprintln!(
                    "  Skipped: {} ({})",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    e
                );
                false
            }
        }
    }

    /**
        Probe multiple video files in parallel using a thread pool.

        This spawns `NUM_WORKERS` threads that pull from the candidate list
        and validate videos concurrently.
    */
    pub fn probe_all_parallel(ready_videos: Arc<ReadyVideos>, candidates: Vec<PathBuf>) {
        if candidates.is_empty() {
            println!("\nScanning complete. 0 valid videos found.");
            return;
        }

        let candidates = Arc::new(candidates);
        let next_index = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(NUM_WORKERS);

        for _ in 0..NUM_WORKERS {
            let ready_videos = Arc::clone(&ready_videos);
            let candidates = Arc::clone(&candidates);
            let next_index = Arc::clone(&next_index);

            let handle = thread::spawn(move || {
                loop {
                    // Atomically grab the next index
                    let index = next_index.fetch_add(1, Ordering::SeqCst);
                    if index >= candidates.len() {
                        break;
                    }

                    let path = &candidates[index];
                    match probe_video(path) {
                        Ok(info) => {
                            println!(
                                "  Validated: {} ({}x{})",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                info.width,
                                info.height
                            );
                            ready_videos.push(info);
                        }
                        Err(e) => {
                            eprintln!(
                                "  Skipped: {} ({})",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                e
                            );
                        }
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all workers to finish
        for handle in handles {
            let _ = handle.join();
        }

        println!(
            "\nScanning complete. {} valid videos found.",
            ready_videos.len()
        );

        if ready_videos.is_empty() {
            eprintln!("No valid video files found in selected paths.");
            eprintln!(
                "Supported formats: mp4, mov, avi, mkv, webm, m4v, wmv, flv, mpeg, mpg, 3gp, ts, mts"
            );
        }
    }

    /**
        Collect all files with video extensions from the given paths.
    */
    fn collect_video_candidates(paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        for path in paths {
            if path.is_file() {
                if Self::has_video_extension(path) {
                    candidates.push(path.clone());
                }
            } else if path.is_dir() {
                let dir_files: Vec<PathBuf> = WalkDir::new(path)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                    .filter(|e| Self::has_video_extension(e.path()))
                    .map(|e| e.path().to_path_buf())
                    .collect();
                candidates.extend(dir_files);
            }
        }

        // Remove duplicates
        candidates.sort();
        candidates.dedup();
        candidates
    }

    /**
        Check if a path has a video file extension.
    */
    fn has_video_extension(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }
}
