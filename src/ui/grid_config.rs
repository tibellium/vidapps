/// Threshold for classifying a video as portrait (aspect ratio <= this value)
const PORTRAIT_THRESHOLD: f32 = 0.8;

/// Video orientation category based on aspect ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoOrientation {
    /// Landscape videos (aspect ratio > 0.8, typically 16:9)
    Landscape,
    /// Portrait videos (aspect ratio <= 0.8, typically 9:16)
    Portrait,
}

impl VideoOrientation {
    /// Determine the orientation from an aspect ratio.
    pub fn from_aspect_ratio(aspect_ratio: f32) -> Self {
        if aspect_ratio <= PORTRAIT_THRESHOLD {
            Self::Portrait
        } else {
            Self::Landscape
        }
    }

    /// Get the reference cell aspect ratio for this orientation.
    /// Landscape uses 16:9, Portrait uses 9:16.
    pub fn cell_aspect_ratio(&self) -> f32 {
        match self {
            Self::Landscape => 16.0 / 9.0,
            Self::Portrait => 9.0 / 16.0,
        }
    }
}

/// Configuration for a video grid layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridConfig {
    /// Number of columns in the grid
    pub cols: u32,
    /// Number of rows in the grid
    pub rows: u32,
    /// Video orientation for this grid
    pub orientation: VideoOrientation,
}

/// All candidate grid configurations.
/// Constraints: max 4 videos total, max 3 on any axis.
const CANDIDATE_GRIDS: &[(u32, u32)] = &[
    (1, 1), // 1 video
    (2, 1), // 2 videos, side by side
    (1, 2), // 2 videos, stacked
    (2, 2), // 4 videos, 2x2
    (3, 1), // 3 videos, side by side
    (1, 3), // 3 videos, stacked
];

impl GridConfig {
    /// Create a new grid configuration.
    pub fn new(cols: u32, rows: u32, orientation: VideoOrientation) -> Self {
        Self {
            cols,
            rows,
            orientation,
        }
    }

    /// Get the total number of slots in this grid.
    pub fn total_slots(&self) -> u32 {
        self.cols * self.rows
    }

    /// Calculate the aspect ratio of this grid based on orientation.
    /// Landscape grids use 16:9 cells, Portrait grids use 9:16 cells.
    pub fn aspect_ratio(&self) -> f32 {
        let cell_ratio = self.orientation.cell_aspect_ratio();
        (self.cols as f32 * cell_ratio) / (self.rows as f32)
    }

    /// Find the optimal grid configuration for the given window dimensions.
    ///
    /// The algorithm:
    /// 1. Calculate the window's aspect ratio
    /// 2. For each candidate grid AND each orientation (landscape/portrait):
    ///    - Calculate how close the grid's aspect ratio is to the window
    /// 3. Select the grid with the smallest difference
    /// 4. Tie-breaker: prefer grids with more videos
    pub fn optimal_for_window(width: f32, height: f32) -> Self {
        let window_ratio = width / height;

        let mut best_config = GridConfig::new(2, 2, VideoOrientation::Landscape);
        let mut best_score = f32::MAX;
        let mut best_slots = 0u32;

        // Try both orientations
        for orientation in [VideoOrientation::Landscape, VideoOrientation::Portrait] {
            for &(cols, rows) in CANDIDATE_GRIDS {
                let config = GridConfig::new(cols, rows, orientation);
                let grid_ratio = config.aspect_ratio();

                // Score is the absolute difference in aspect ratios
                let score = (window_ratio - grid_ratio).abs();

                // Select if better score, or same score but more videos
                if score < best_score || (score == best_score && config.total_slots() > best_slots)
                {
                    best_config = config;
                    best_score = score;
                    best_slots = config.total_slots();
                }
            }
        }

        best_config
    }
}

impl Default for GridConfig {
    fn default() -> Self {
        Self::new(2, 2, VideoOrientation::Landscape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orientation_from_aspect_ratio() {
        // 16:9 = 1.78 -> Landscape
        assert_eq!(
            VideoOrientation::from_aspect_ratio(16.0 / 9.0),
            VideoOrientation::Landscape
        );
        // 9:16 = 0.5625 -> Portrait
        assert_eq!(
            VideoOrientation::from_aspect_ratio(9.0 / 16.0),
            VideoOrientation::Portrait
        );
        // 4:3 = 1.33 -> Landscape
        assert_eq!(
            VideoOrientation::from_aspect_ratio(4.0 / 3.0),
            VideoOrientation::Landscape
        );
        // 3:4 = 0.75 -> Portrait
        assert_eq!(
            VideoOrientation::from_aspect_ratio(3.0 / 4.0),
            VideoOrientation::Portrait
        );
        // Edge case: 0.8 -> Portrait
        assert_eq!(
            VideoOrientation::from_aspect_ratio(0.8),
            VideoOrientation::Portrait
        );
        // Just above threshold -> Landscape
        assert_eq!(
            VideoOrientation::from_aspect_ratio(0.81),
            VideoOrientation::Landscape
        );
    }

    #[test]
    fn test_landscape_aspect_ratios() {
        // 1x1 landscape grid = 16:9 = 1.78
        let config = GridConfig::new(1, 1, VideoOrientation::Landscape);
        assert!((config.aspect_ratio() - 1.778).abs() < 0.01);

        // 2x2 landscape grid = (2 * 16/9) / 2 = 16:9 = 1.78
        let config = GridConfig::new(2, 2, VideoOrientation::Landscape);
        assert!((config.aspect_ratio() - 1.778).abs() < 0.01);

        // 2x1 landscape grid = (2 * 16/9) / 1 = 32:9 = 3.56
        let config = GridConfig::new(2, 1, VideoOrientation::Landscape);
        assert!((config.aspect_ratio() - 3.556).abs() < 0.01);
    }

    #[test]
    fn test_portrait_aspect_ratios() {
        // 1x1 portrait grid = 9:16 = 0.5625
        let config = GridConfig::new(1, 1, VideoOrientation::Portrait);
        assert!((config.aspect_ratio() - 0.5625).abs() < 0.01);

        // 2x2 portrait grid = (2 * 9/16) / 2 = 9:16 = 0.5625
        let config = GridConfig::new(2, 2, VideoOrientation::Portrait);
        assert!((config.aspect_ratio() - 0.5625).abs() < 0.01);

        // 3x1 portrait grid = (3 * 9/16) / 1 = 27:16 = 1.6875
        let config = GridConfig::new(3, 1, VideoOrientation::Portrait);
        assert!((config.aspect_ratio() - 1.6875).abs() < 0.01);
    }

    #[test]
    fn test_optimal_for_16_9_window() {
        // 16:9 window should prefer 2x2 landscape
        let config = GridConfig::optimal_for_window(1920.0, 1080.0);
        assert_eq!(config.cols, 2);
        assert_eq!(config.rows, 2);
        assert_eq!(config.orientation, VideoOrientation::Landscape);
    }

    #[test]
    fn test_optimal_for_9_16_window() {
        // 9:16 window (phone aspect ratio) should prefer 2x2 portrait
        let config = GridConfig::optimal_for_window(1080.0, 1920.0);
        assert_eq!(config.cols, 2);
        assert_eq!(config.rows, 2);
        assert_eq!(config.orientation, VideoOrientation::Portrait);
    }

    #[test]
    fn test_optimal_for_wide_window() {
        // Very wide window (32:9) should prefer 2x1 landscape
        let config = GridConfig::optimal_for_window(3200.0, 900.0);
        assert_eq!(config.cols, 2);
        assert_eq!(config.rows, 1);
        assert_eq!(config.orientation, VideoOrientation::Landscape);
    }

    #[test]
    fn test_optimal_for_tall_narrow_window() {
        // Tall narrow window should prefer portrait grid
        let config = GridConfig::optimal_for_window(540.0, 1920.0);
        assert_eq!(config.orientation, VideoOrientation::Portrait);
    }
}
