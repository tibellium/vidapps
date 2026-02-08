use chrono::Utc;

/// Get current unix timestamp in seconds.
pub fn now() -> u64 {
    Utc::now().timestamp() as u64
}
