mod constants;
mod crypto;
mod device;
mod error;
mod pssh_ext;
mod session;

pub mod format {
    pub use drm_playready_format::*;
}

#[cfg(feature = "static-devices")]
pub mod static_devices;

// Re-export shared DRM types from drm-core
pub use drm_core::{
    ContentKey, KeyType, PLAYREADY_SYSTEM_ID, ParseKid, PsshBox, PsshError, SystemId, parse_kid,
};

// PlayReady-specific exports (uncomment as implementations are added)
// pub use self::device::Device;
// pub use self::error::{CdmError, CdmResult};
// pub use self::pssh_ext::PlayReadyExt;
// pub use self::session::Session;
