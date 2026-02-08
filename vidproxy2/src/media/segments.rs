use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/**
    Manages HLS segments in a directory.
    Handles cleanup of old segments to prevent unbounded disk usage.
*/
pub struct SegmentManager {
    output_dir: PathBuf,
    max_segments: usize,
    segments: Mutex<VecDeque<String>>,
}

impl SegmentManager {
    pub fn new(output_dir: PathBuf, max_segments: usize) -> Self {
        Self {
            output_dir,
            max_segments,
            segments: Mutex::new(VecDeque::new()),
        }
    }

    pub fn _output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub fn _register_segment(&self, filename: &str) {
        let mut segments = self.segments.lock().unwrap();
        segments.push_back(filename.to_string());

        while segments.len() > self.max_segments {
            if let Some(old_segment) = segments.pop_front() {
                let path = self.output_dir.join(&old_segment);
                let _ = fs::remove_file(path);
            }
        }
    }

    /**
        Scan the output directory for new .ts segments written by FFmpeg.
    */
    pub fn scan_for_new_segments(&self) {
        let Ok(entries) = fs::read_dir(&self.output_dir) else {
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

        new_segments.sort();

        for segment in new_segments {
            segments.push_back(segment);
        }

        while segments.len() > self.max_segments {
            if let Some(old_segment) = segments.pop_front() {
                let path = self.output_dir.join(&old_segment);
                let _ = fs::remove_file(path);
            }
        }
    }

    pub fn _playlist_path(&self) -> PathBuf {
        self.output_dir.join("playlist.m3u8")
    }

    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }

    /**
        Clear all segments and remove files from disk.
    */
    pub fn clear(&self) {
        let mut segments = self.segments.lock().unwrap();

        for segment in segments.drain(..) {
            let path = self.output_dir.join(&segment);
            let _ = fs::remove_file(path);
        }

        let _ = fs::remove_file(self.output_dir.join("playlist.m3u8"));
    }
}
