#![allow(dead_code)]

use tokio::sync::watch;

/**
    Information about a discovered stream.
*/
#[derive(Clone, Debug)]
pub struct StreamInfo {
    /// Channel display name
    pub channel_name: String,
    /// The DASH/HLS manifest URL
    pub mpd_url: String,
    /// Decryption key in "key_id:key" format
    pub decryption_key: String,
    /// License server URL (for potential refresh)
    pub license_url: String,
    /// PSSH box in base64 (for potential refresh)
    pub pssh: String,
    /// Optional thumbnail URL for channel logo
    pub thumbnail_url: Option<String>,
    /// Optional expiration timestamp (Unix seconds)
    pub expires_at: Option<u64>,
}

pub type StreamInfoReceiver = watch::Receiver<Option<StreamInfo>>;
pub type StreamInfoSender = watch::Sender<Option<StreamInfo>>;

/**
    Create a new stream info channel pair.
*/
pub fn stream_info_channel() -> (StreamInfoSender, StreamInfoReceiver) {
    watch::channel(None)
}
