use std::sync::OnceLock;

use include_dir::{Dir, include_dir};

use crate::device::Device;

static DEVICES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/devices");
static DEVICES: OnceLock<Vec<Device>> = OnceLock::new();

fn load_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    for file in DEVICES_DIR.files() {
        let path = file.path().display();
        match Device::from_bytes(file.contents()) {
            Ok(device) => devices.push(device),
            Err(e) => eprintln!("warning: failed to parse embedded device {path}: {e}"),
        }
    }
    devices
}

fn devices() -> &'static [Device] {
    DEVICES.get_or_init(load_devices)
}

/**
    Return all embedded devices (parsed once, cached in memory).
    Devices that fail to parse are skipped with a warning on stderr.
*/
pub fn all() -> &'static [Device] {
    devices()
}

/**
    Pick a random embedded device.

    # Panics

    Panics if no embedded devices parsed successfully.
*/
pub fn random() -> Device {
    let devs = devices();
    assert!(!devs.is_empty(), "no usable embedded device files");
    let idx = rand::random_range(0..devs.len());
    devs[idx].clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SecurityLevel;

    #[test]
    fn random_returns_valid_device() {
        let device = random();
        assert_eq!(device.security_level, SecurityLevel::L3);
    }

    #[test]
    fn all_devices_are_l3() {
        for device in all() {
            assert_eq!(device.security_level, SecurityLevel::L3);
        }
    }
}
