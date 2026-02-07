#![allow(clippy::doc_overindented_list_items)]

mod constants;
mod crypto;
mod device;
mod error;
mod pssh_ext;
mod session;
mod types;
mod utils;

pub mod proto {
    pub use drm_widevine_proto::prost::Message;
    pub use drm_widevine_proto::*;
}

#[cfg(feature = "static-devices")]
pub mod static_devices;

// Re-export shared DRM types from drm-core
pub use drm_core::{
    CLEARKEY_SYSTEM_ID, ContentKey, FAIRPLAY_SYSTEM_ID, KeyType, PLAYREADY_SYSTEM_ID, ParseKid,
    PsshBox, PsshError, SystemId, WIDEVINE_SYSTEM_ID, parse_kid,
};

// Widevine-specific exports
pub use self::device::Device;
pub use self::error::{CdmError, CdmResult};
pub use self::pssh_ext::WidevineExt;
pub use self::session::Session;
pub use self::types::{DeviceType, LicenseType, SecurityLevel};
