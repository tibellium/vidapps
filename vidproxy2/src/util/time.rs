use chrono::{DateTime, TimeZone, Utc};

/// Get the current time as a UTC datetime.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Parse a timestamp string into a UTC datetime.
///
/// Supports:
/// - RFC 3339 / ISO 8601 (e.g. `"2026-02-08T05:00:00.000Z"`)
/// - Millisecond epoch as string (13+ digits, e.g. `"1770526800000"`)
/// - Second epoch as string (10-12 digits, e.g. `"1770526800"`)
pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();

    // Try RFC 3339 / ISO 8601
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try numeric epoch
    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        match trimmed.len() {
            13.. => {
                let ms = trimmed.parse::<i64>().ok()?;
                return Utc.timestamp_millis_opt(ms).single();
            }
            10..=12 => {
                let secs = trimmed.parse::<i64>().ok()?;
                return Utc.timestamp_opt(secs, 0).single();
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rfc3339() {
        let dt = parse_timestamp("2026-02-08T05:00:00.000Z").unwrap();
        assert_eq!(dt.timestamp(), 1770526800);
    }

    #[test]
    fn test_parse_millisecond_epoch() {
        let dt = parse_timestamp("1770526800000").unwrap();
        assert_eq!(dt.timestamp(), 1770526800);
    }

    #[test]
    fn test_parse_second_epoch() {
        let dt = parse_timestamp("1770526800").unwrap();
        assert_eq!(dt.timestamp(), 1770526800);
    }

    #[test]
    fn test_parse_invalid() {
        assert!(parse_timestamp("not-a-timestamp").is_none());
        assert!(parse_timestamp("").is_none());
        assert!(parse_timestamp("123").is_none());
    }
}
