#![allow(clippy::doc_overindented_list_items)]

pub use drm_core as core;

mod constants;
mod crypto;
mod device;
mod error;
mod pssh_ext;
mod session;
mod types;

pub mod format {
    pub use drm_playready_format::*;
}

#[cfg(feature = "static-devices")]
pub mod static_devices;

pub use self::device::Device;
pub use self::error::{CdmError, CdmResult};
pub use self::pssh_ext::PlayReadyExt;
pub use self::session::Session;
pub use self::types::SecurityLevel;
