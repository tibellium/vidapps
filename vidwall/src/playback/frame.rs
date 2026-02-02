use std::time::Duration;

/**
    A decoded video frame ready for rendering
*/
#[derive(Clone)]
pub struct VideoFrame {
    /// BGRA pixel data (width * height * 4 bytes)
    pub data: Vec<u8>,
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
    /// Presentation timestamp
    pub pts: Duration,
}

impl VideoFrame {
    pub fn new(data: Vec<u8>, width: u32, height: u32, pts: Duration) -> Self {
        Self {
            data,
            width,
            height,
            pts,
        }
    }
}
