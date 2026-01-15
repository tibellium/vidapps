use std::path::PathBuf;
use std::sync::RwLock;

use rand::seq::SliceRandom;

use crate::ui::VideoOrientation;

use super::VideoInfo;

/// Thread-safe storage for validated video files, organized by orientation.
///
/// Videos are added to this storage after being validated by ffprobe.
/// The GridView pulls videos from here when filling slots, filtered by orientation.
pub struct ReadyVideos {
    landscape: RwLock<Vec<VideoInfo>>,
    portrait: RwLock<Vec<VideoInfo>>,
}

impl ReadyVideos {
    /// Create a new empty ReadyVideos storage.
    pub fn new() -> Self {
        Self {
            landscape: RwLock::new(Vec::new()),
            portrait: RwLock::new(Vec::new()),
        }
    }

    /// Add a validated video to the appropriate storage based on orientation.
    pub fn push(&self, info: VideoInfo) {
        match info.orientation() {
            VideoOrientation::Landscape => {
                self.landscape.write().unwrap().push(info);
            }
            VideoOrientation::Portrait => {
                self.portrait.write().unwrap().push(info);
            }
        }
    }

    /// Get the total number of ready videos (both orientations).
    pub fn len(&self) -> usize {
        self.landscape.read().unwrap().len() + self.portrait.read().unwrap().len()
    }

    /// Get the number of videos for a specific orientation.
    pub fn len_for_orientation(&self, orientation: VideoOrientation) -> usize {
        match orientation {
            VideoOrientation::Landscape => self.landscape.read().unwrap().len(),
            VideoOrientation::Portrait => self.portrait.read().unwrap().len(),
        }
    }

    /// Check if the storage is empty (both orientations).
    pub fn is_empty(&self) -> bool {
        self.landscape.read().unwrap().is_empty() && self.portrait.read().unwrap().is_empty()
    }

    /// Check if a specific orientation has videos.
    pub fn has_videos_for_orientation(&self, orientation: VideoOrientation) -> bool {
        match orientation {
            VideoOrientation::Landscape => !self.landscape.read().unwrap().is_empty(),
            VideoOrientation::Portrait => !self.portrait.read().unwrap().is_empty(),
        }
    }

    /// Pick a random video of the specified orientation.
    ///
    /// Returns None if no videos of that orientation are available.
    pub fn pick_random_for_orientation(&self, orientation: VideoOrientation) -> Option<VideoInfo> {
        let videos = match orientation {
            VideoOrientation::Landscape => self.landscape.read().unwrap(),
            VideoOrientation::Portrait => self.portrait.read().unwrap(),
        };
        let mut rng = rand::thread_rng();
        videos.choose(&mut rng).cloned()
    }

    /// Pick a random video of the specified orientation that is not in the exclusion list.
    ///
    /// If all videos of that orientation are excluded, falls back to picking any video of that orientation.
    /// Returns None if no videos of that orientation are available.
    pub fn pick_random_except_for_orientation(
        &self,
        orientation: VideoOrientation,
        exclude: &[PathBuf],
    ) -> Option<VideoInfo> {
        let videos = match orientation {
            VideoOrientation::Landscape => self.landscape.read().unwrap(),
            VideoOrientation::Portrait => self.portrait.read().unwrap(),
        };

        if videos.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();

        // Try to find a video not in the exclusion list
        let available: Vec<_> = videos
            .iter()
            .filter(|v| !exclude.contains(&v.path))
            .collect();

        if available.is_empty() {
            // Fall back to any video of this orientation if all are excluded
            videos.choose(&mut rng).cloned()
        } else {
            available.choose(&mut rng).cloned().cloned()
        }
    }
}

impl Default for ReadyVideos {
    fn default() -> Self {
        Self::new()
    }
}
