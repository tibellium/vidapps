#![allow(clippy::doc_overindented_list_items)]

mod constants;
mod crypto;
mod device;
mod error;
mod pssh;
mod session;
mod types;
mod utils;

pub mod proto {
    pub use prost::Message;
    pub use wdv3_proto::*;
}

#[cfg(feature = "static-devices")]
pub mod static_devices;

pub use self::constants::{
    CLEARKEY_SYSTEM_ID, FAIRPLAY_SYSTEM_ID, PLAYREADY_SYSTEM_ID, WIDEVINE_SYSTEM_ID,
};
pub use self::device::Device;
pub use self::error::{CdmError, CdmResult, ParseError};
pub use self::pssh::PsshBox;
pub use self::session::Session;
pub use self::types::{ContentKey, DeviceType, KeyType, LicenseType, SecurityLevel, SystemId};
pub use self::utils::{ParseKid, parse_kid};
