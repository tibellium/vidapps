use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Bounds, DisplayId, PlatformDisplay, Point, Size, px};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/**
    Saved window state for persistence across sessions.
*/
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    /// UUID of the display the window was on (stable across restarts)
    pub display_uuid: Option<String>,
    /// X position relative to display origin
    pub x: f32,
    /// Y position relative to display origin
    pub y: f32,
    /// Content width (not including window chrome)
    pub width: f32,
    /// Content height (not including window chrome)
    pub height: f32,
}

impl WindowState {
    /**
        Create a new WindowState.

        - `display`: The display the window is on (to get UUID and calculate relative position)
        - `window_origin`: The window origin from bounds()
        - `content_size`: The content size from viewport_size()
    */
    pub fn new(
        display: Option<Rc<dyn PlatformDisplay>>,
        window_origin: Point<gpui::Pixels>,
        content_size: Size<gpui::Pixels>,
    ) -> Self {
        let (display_uuid, relative_x, relative_y) = if let Some(display) = display {
            let uuid = display.uuid().ok().map(|u| u.to_string());
            let display_bounds = display.bounds();
            // Store position relative to display origin
            let rel_x: f32 = window_origin.x.into();
            let rel_y: f32 = window_origin.y.into();
            (uuid, rel_x, rel_y)
        } else {
            (None, window_origin.x.into(), window_origin.y.into())
        };

        Self {
            display_uuid,
            x: relative_x,
            y: relative_y,
            width: content_size.width.into(),
            height: content_size.height.into(),
        }
    }

    /**
        Find the display this window state belongs to.
    */
    pub fn find_display(&self, cx: &App) -> Option<(Rc<dyn PlatformDisplay>, DisplayId)> {
        let saved_uuid = self.display_uuid.as_ref()?;
        let saved_uuid: Uuid = saved_uuid.parse().ok()?;

        for display in cx.displays() {
            if let Ok(uuid) = display.uuid() {
                if uuid == saved_uuid {
                    return Some((display.clone(), display.id()));
                }
            }
        }
        None
    }

    /**
        Convert to GPUI bounds, optionally using display info for positioning.
    */
    pub fn to_bounds(&self, cx: &App) -> Bounds<gpui::Pixels> {
        // Try to find the original display
        if let Some((_display, _id)) = self.find_display(cx) {
            // Found the display - use relative coordinates
            // The x,y are already in the coordinate system GPUI expects
            Bounds {
                origin: Point {
                    x: px(self.x),
                    y: px(self.y),
                },
                size: Size {
                    width: px(self.width),
                    height: px(self.height),
                },
            }
        } else {
            // Display not found - just use the coordinates as-is
            Bounds {
                origin: Point {
                    x: px(self.x),
                    y: px(self.y),
                },
                size: Size {
                    width: px(self.width),
                    height: px(self.height),
                },
            }
        }
    }

    /**
        Get the display ID if the display still exists.
    */
    pub fn display_id(&self, cx: &App) -> Option<DisplayId> {
        self.find_display(cx).map(|(_, id)| id)
    }

    /**
        Get the path to the window state file.
    */
    fn state_file_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|p| p.join("vidwall").join("window_state.json"))
    }

    /**
        Load window state from disk.
    */
    pub fn load() -> Option<Self> {
        let path = Self::state_file_path()?;
        let contents = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /**
        Save window state to disk.
    */
    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = match Self::state_file_path() {
            Some(p) => p,
            None => return Ok(()), // Silently skip if no data dir
        };

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)
    }
}
