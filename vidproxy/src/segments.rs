use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;

/**
    Manages HLS segments in a temporary directory.
    Handles cleanup of old segments to prevent unbounded disk usage.
*/
pub struct SegmentManager {
    temp_dir: TempDir,
    max_segments: usize,
    segments: Mutex<VecDeque<String>>,
}

impl SegmentManager {
    /**
        Create a new segment manager with a temp directory.
    */
    pub fn new(max_segments: usize) -> std::io::Result<Self> {
        let temp_dir = TempDir::new()?;
        Ok(Self {
            temp_dir,
            max_segments,
            segments: Mutex::new(VecDeque::new()),
        })
    }

    /**
        Get the output directory path.
    */
    pub fn output_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    /**
        Register a new segment and clean up old ones if needed.
    */
    #[allow(dead_code)]
    pub fn register_segment(&self, filename: &str) {
        let mut segments = self.segments.lock().unwrap();

        // Add new segment
        segments.push_back(filename.to_string());

        // Remove old segments if over limit
        while segments.len() > self.max_segments {
            if let Some(old_segment) = segments.pop_front() {
                let path = self.temp_dir.path().join(&old_segment);
                let _ = fs::remove_file(path);
            }
        }
    }

    /**
        Scan the output directory for new .ts segments.
        Call this periodically to detect segments written by FFmpeg.
    */
    pub fn scan_for_new_segments(&self) {
        let dir = self.temp_dir.path();
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        let mut segments = self.segments.lock().unwrap();
        let known: std::collections::HashSet<_> = segments.iter().cloned().collect();

        let mut new_segments: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".ts") && !known.contains(&name) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        // Sort by name (FFmpeg uses sequential numbering)
        new_segments.sort();

        for segment in new_segments {
            segments.push_back(segment);
        }

        // Cleanup old segments
        while segments.len() > self.max_segments {
            if let Some(old_segment) = segments.pop_front() {
                let path = dir.join(&old_segment);
                let _ = fs::remove_file(path);
            }
        }
    }

    /**
        Get the playlist path.
    */
    #[allow(dead_code)]
    pub fn playlist_path(&self) -> PathBuf {
        self.temp_dir.path().join("playlist.m3u8")
    }

    /**
        Get current segment count.
    */
    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }
}
