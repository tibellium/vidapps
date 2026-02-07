use include_dir::{Dir, include_dir};

use crate::device::Device;

static DEVICES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/devices");

/**
    Return an iterator over all embedded devices, lazily parsed.
    Skips files that fail to parse (shouldn't happen with valid .prd files).
*/
pub fn all() -> impl Iterator<Item = Device> {
    DEVICES_DIR
        .files()
        .filter_map(|file| Device::from_bytes(file.contents()).ok())
}

/**
    Pick a random embedded device.

    # Panics

    Panics if the embedded devices directory is empty or contains an unparseable file.
*/
pub fn random() -> Device {
    let files: Vec<_> = DEVICES_DIR.files().collect();
    assert!(!files.is_empty(), "no embedded device files");
    let idx = rand::random_range(0..files.len());
    Device::from_bytes(files[idx].contents()).expect("embedded .prd file failed to parse")
}

/**
    Return the number of embedded device files.
*/
pub fn count() -> usize {
    DEVICES_DIR.files().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_matches_dir() {
        assert_eq!(count(), 27);
    }

    #[test]
    fn all_devices_parse() {
        assert_eq!(all().count(), count());
    }

    #[test]
    fn random_returns_valid_device() {
        let device = random();
        assert_eq!(device.security_level, crate::types::SecurityLevel::SL3000);
    }

    #[test]
    fn all_devices_are_sl3000() {
        for device in all() {
            assert_eq!(device.security_level, crate::types::SecurityLevel::SL3000);
        }
    }
}
