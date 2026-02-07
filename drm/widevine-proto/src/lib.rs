pub mod license_protocol {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

pub use license_protocol::*;

pub use prost;

/*
    drm-core ↔ proto conversions

    These live here (rather than in drm-widevine) because the orphan rule
    requires at least one of the two types to be local to the crate that
    implements `From`. The proto types are local to this crate.
*/

use drm_core::KeyType;

type ProtoKeyType = license::key_container::KeyType;

impl From<KeyType> for ProtoKeyType {
    fn from(kt: KeyType) -> Self {
        match kt {
            KeyType::Signing => Self::Signing,
            KeyType::Content => Self::Content,
            KeyType::KeyControl => Self::KeyControl,
            KeyType::OperatorSession => Self::OperatorSession,
            KeyType::Entitlement => Self::Entitlement,
            KeyType::OemContent => Self::OemContent,
        }
    }
}

impl From<ProtoKeyType> for KeyType {
    fn from(proto: ProtoKeyType) -> Self {
        match proto {
            ProtoKeyType::Signing => Self::Signing,
            ProtoKeyType::Content => Self::Content,
            ProtoKeyType::KeyControl => Self::KeyControl,
            ProtoKeyType::OperatorSession => Self::OperatorSession,
            ProtoKeyType::Entitlement => Self::Entitlement,
            ProtoKeyType::OemContent => Self::OemContent,
        }
    }
}
