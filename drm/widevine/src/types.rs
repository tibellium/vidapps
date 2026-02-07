use core::fmt;
use core::str::FromStr;

use drm_core::ParseError;

use crate::utils::{eq_ignore_ascii_case, trim_ascii};

/**
    Device type as encoded in WVD file byte offset 4.
    Values: Chrome=1, Android=2. These are defined by the WVD file format specification,
    not by Google's license_protocol.proto (the closest proto enum,
    ClientIdentification.TokenType, has unrelated values).
*/
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceType {
    Chrome = 1,
    Android = 2,
}

impl DeviceType {
    pub const fn from_u8(u: u8) -> Option<Self> {
        match u {
            1 => Some(Self::Chrome),
            2 => Some(Self::Android),
            _ => None,
        }
    }

    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            6 if eq_ignore_ascii_case(name, b"chrome") => Some(Self::Chrome),
            7 if eq_ignore_ascii_case(name, b"android") => Some(Self::Android),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Android => "Android",
        }
    }
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for DeviceType {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "device type",
            value: s.to_owned(),
        })
    }
}

/**
    Widevine security level.
    Ref: license_protocol.proto.
*/
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecurityLevel {
    L1 = 1,
    L2 = 2,
    L3 = 3,
}

impl SecurityLevel {
    pub const fn from_u8(u: u8) -> Option<Self> {
        match u {
            1 => Some(Self::L1),
            2 => Some(Self::L2),
            3 => Some(Self::L3),
            _ => None,
        }
    }

    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            1 => match name[0] {
                b'1' => Some(Self::L1),
                b'2' => Some(Self::L2),
                b'3' => Some(Self::L3),
                _ => None,
            },
            2 if eq_ignore_ascii_case(name, b"l1") => Some(Self::L1),
            2 if eq_ignore_ascii_case(name, b"l2") => Some(Self::L2),
            2 if eq_ignore_ascii_case(name, b"l3") => Some(Self::L3),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
        }
    }
}

impl fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for SecurityLevel {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "security level",
            value: s.to_owned(),
        })
    }
}

/**
    Widevine license type.
    Ref: license_protocol.proto, LicenseType enum.
*/
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LicenseType {
    /// Normal one-time-use license for streaming content.
    #[default]
    Streaming,
    /// Offline-use license, usually for downloaded content.
    Offline,
    /// License type decision is left to the provider.
    Automatic,
}

impl LicenseType {
    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            4 if eq_ignore_ascii_case(name, b"auto") => Some(Self::Automatic),
            7 if eq_ignore_ascii_case(name, b"offline") => Some(Self::Offline),
            9 if eq_ignore_ascii_case(name, b"streaming") => Some(Self::Streaming),
            9 if eq_ignore_ascii_case(name, b"automatic") => Some(Self::Automatic),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Offline => "offline",
            Self::Automatic => "automatic",
        }
    }
}

impl fmt::Display for LicenseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for LicenseType {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "license type",
            value: s.to_owned(),
        })
    }
}

type ProtoLicenseType = drm_widevine_proto::LicenseType;

impl From<LicenseType> for ProtoLicenseType {
    fn from(lt: LicenseType) -> Self {
        match lt {
            LicenseType::Streaming => Self::Streaming,
            LicenseType::Offline => Self::Offline,
            LicenseType::Automatic => Self::Automatic,
        }
    }
}

impl From<ProtoLicenseType> for LicenseType {
    fn from(proto: ProtoLicenseType) -> Self {
        match proto {
            ProtoLicenseType::Streaming => Self::Streaming,
            ProtoLicenseType::Offline => Self::Offline,
            ProtoLicenseType::Automatic => Self::Automatic,
        }
    }
}

/**
    The three derived keys from a session key.
*/
pub(crate) struct DerivedKeys {
    /**
        16 bytes. AES-CMAC(session_key, 0x01 || enc_context).
        Used to decrypt KeyContainer.key fields.
    */
    pub(crate) enc_key: [u8; 16],
    /**
        32 bytes. CMAC(session_key, 0x01 || mac_context) || CMAC(session_key, 0x02 || mac_context).
        Used to verify license response signature via HMAC-SHA256.
    */
    pub(crate) mac_key_server: [u8; 32],
    /**
        32 bytes. CMAC(session_key, 0x03 || mac_context) || CMAC(session_key, 0x04 || mac_context).
        Used for license renewal requests.
    */
    #[allow(dead_code)]
    pub(crate) mac_key_client: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    use drm_core::KeyType;

    type ProtoKeyType = drm_widevine_proto::license::key_container::KeyType;

    #[test]
    fn device_type_round_trip() {
        for val in [1u8, 2] {
            let dt = DeviceType::from_u8(val).unwrap();
            assert_eq!(dt.to_u8(), val);
        }
        assert!(DeviceType::from_u8(0).is_none());
        assert!(DeviceType::from_u8(3).is_none());
    }

    #[test]
    fn security_level_round_trip() {
        for val in [1u8, 2, 3] {
            let sl = SecurityLevel::from_u8(val).unwrap();
            assert_eq!(sl.to_u8(), val);
        }
        assert!(SecurityLevel::from_u8(0).is_none());
        assert!(SecurityLevel::from_u8(4).is_none());
    }

    #[test]
    fn license_type_default_is_streaming() {
        assert_eq!(LicenseType::default(), LicenseType::Streaming);
    }

    #[test]
    fn key_type_proto_round_trip() {
        let variants = [
            KeyType::Signing,
            KeyType::Content,
            KeyType::KeyControl,
            KeyType::OperatorSession,
            KeyType::Entitlement,
            KeyType::OemContent,
        ];
        for kt in variants {
            let proto: ProtoKeyType = kt.into();
            let back: KeyType = proto.into();
            assert_eq!(back, kt);
        }
    }

    #[test]
    fn device_type_name_round_trip() {
        for dt in [DeviceType::Chrome, DeviceType::Android] {
            let name = dt.to_name();
            let parsed = DeviceType::from_name(name.as_bytes()).unwrap();
            assert_eq!(parsed, dt);
        }
    }

    #[test]
    fn device_type_from_name_case_insensitive() {
        assert_eq!(DeviceType::from_name(b"chrome"), Some(DeviceType::Chrome));
        assert_eq!(DeviceType::from_name(b"CHROME"), Some(DeviceType::Chrome));
        assert_eq!(DeviceType::from_name(b"Chrome"), Some(DeviceType::Chrome));
        assert_eq!(DeviceType::from_name(b"ANDROID"), Some(DeviceType::Android));
        assert_eq!(DeviceType::from_name(b"android"), Some(DeviceType::Android));
        assert_eq!(DeviceType::from_name(b"unknown"), None);
        assert_eq!(DeviceType::from_name(b""), None);
    }

    #[test]
    fn security_level_name_round_trip() {
        for sl in [SecurityLevel::L1, SecurityLevel::L2, SecurityLevel::L3] {
            let name = sl.to_name();
            let parsed = SecurityLevel::from_name(name.as_bytes()).unwrap();
            assert_eq!(parsed, sl);
        }
    }

    #[test]
    fn security_level_from_name_bare_digits() {
        assert_eq!(SecurityLevel::from_name(b"1"), Some(SecurityLevel::L1));
        assert_eq!(SecurityLevel::from_name(b"2"), Some(SecurityLevel::L2));
        assert_eq!(SecurityLevel::from_name(b"3"), Some(SecurityLevel::L3));
        assert_eq!(SecurityLevel::from_name(b"0"), None);
        assert_eq!(SecurityLevel::from_name(b"4"), None);
    }

    #[test]
    fn security_level_from_name_case_insensitive() {
        assert_eq!(SecurityLevel::from_name(b"l1"), Some(SecurityLevel::L1));
        assert_eq!(SecurityLevel::from_name(b"L1"), Some(SecurityLevel::L1));
        assert_eq!(SecurityLevel::from_name(b"L3"), Some(SecurityLevel::L3));
        assert_eq!(SecurityLevel::from_name(b"l3"), Some(SecurityLevel::L3));
        assert_eq!(SecurityLevel::from_name(b""), None);
    }

    #[test]
    fn license_type_name_round_trip() {
        for lt in [
            LicenseType::Streaming,
            LicenseType::Offline,
            LicenseType::Automatic,
        ] {
            let name = lt.to_name();
            let parsed = LicenseType::from_name(name.as_bytes()).unwrap();
            assert_eq!(parsed, lt);
        }
    }

    #[test]
    fn license_type_from_name_alias_auto() {
        assert_eq!(
            LicenseType::from_name(b"auto"),
            Some(LicenseType::Automatic)
        );
        assert_eq!(
            LicenseType::from_name(b"AUTO"),
            Some(LicenseType::Automatic)
        );
        assert_eq!(
            LicenseType::from_name(b"automatic"),
            Some(LicenseType::Automatic)
        );
        assert_eq!(
            LicenseType::from_name(b"STREAMING"),
            Some(LicenseType::Streaming)
        );
        assert_eq!(
            LicenseType::from_name(b"Offline"),
            Some(LicenseType::Offline)
        );
        assert_eq!(LicenseType::from_name(b""), None);
        assert_eq!(LicenseType::from_name(b"bad"), None);
    }
}
