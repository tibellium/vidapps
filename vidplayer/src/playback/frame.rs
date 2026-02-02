use std::time::Duration;

#[derive(Clone)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
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
