use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Bounds, DisplayId, PlatformDisplay, Point, Size, px};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub display_uuid: Option<String>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowState {
    pub fn new(
        display: Option<Rc<dyn PlatformDisplay>>,
        window_origin: Point<gpui::Pixels>,
        content_size: Size<gpui::Pixels>,
    ) -> Self {
        let (display_uuid, relative_x, relative_y) = if let Some(display) = display {
            let uuid = display.uuid().ok().map(|u| u.to_string());
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

    pub fn to_bounds(&self, cx: &App) -> Bounds<gpui::Pixels> {
        if let Some((_display, _id)) = self.find_display(cx) {
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

    pub fn display_id(&self, cx: &App) -> Option<DisplayId> {
        self.find_display(cx).map(|(_, id)| id)
    }

    fn state_file_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|p| p.join("vidplayer").join("window_state.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::state_file_path()?;
        let contents = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = match Self::state_file_path() {
            Some(p) => p,
            None => return Ok(()),
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&path, contents)
    }
}
