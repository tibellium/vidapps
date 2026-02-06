use core::fmt;
use core::str::FromStr;

use crate::error::ParseError;
use crate::utils::{bytes_equal, eq_ignore_ascii_case, trim_ascii};

/// Device type as encoded in WVD file byte offset 4.
/// Values: Chrome=1, Android=2. These are defined by the WVD file format specification,
/// not by Google's license_protocol.proto (the closest proto enum,
/// ClientIdentification.TokenType, has unrelated values).
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

/// Widevine security level.
/// Ref: license_protocol.proto.
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

/// Key type enumeration from License.KeyContainer.KeyType.
/// Ref: license_protocol.proto, License.KeyContainer.KeyType enum.
///
/// Note: Protobuf default value 0 has no named variant in the proto definition.
/// If a KeyContainer has key_type == 0, it should be treated as an unknown type
/// and processed (decrypted, stored) but not included in the CONTENT key output.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyType {
    Signing = 1,
    Content = 2,
    KeyControl = 3,
    OperatorSession = 4,
    Entitlement = 5,
    OemContent = 6,
}

impl KeyType {
    pub const fn from_u8(u: u8) -> Option<Self> {
        match u {
            1 => Some(Self::Signing),
            2 => Some(Self::Content),
            3 => Some(Self::KeyControl),
            4 => Some(Self::OperatorSession),
            5 => Some(Self::Entitlement),
            6 => Some(Self::OemContent),
            _ => None,
        }
    }

    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            7 if eq_ignore_ascii_case(name, b"signing") => Some(Self::Signing),
            7 if eq_ignore_ascii_case(name, b"content") => Some(Self::Content),
            11 if eq_ignore_ascii_case(name, b"key_control") => Some(Self::KeyControl),
            11 if eq_ignore_ascii_case(name, b"oem_content") => Some(Self::OemContent),
            11 if eq_ignore_ascii_case(name, b"entitlement") => Some(Self::Entitlement),
            16 if eq_ignore_ascii_case(name, b"operator_session") => Some(Self::OperatorSession),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Signing => "SIGNING",
            Self::Content => "CONTENT",
            Self::KeyControl => "KEY_CONTROL",
            Self::OperatorSession => "OPERATOR_SESSION",
            Self::Entitlement => "ENTITLEMENT",
            Self::OemContent => "OEM_CONTENT",
        }
    }
}

impl fmt::Display for KeyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for KeyType {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "key type",
            value: s.to_owned(),
        })
    }
}

/// Proto KeyType (nested inside License.KeyContainer).
type ProtoKeyType = wdv3_proto::license::key_container::KeyType;

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

/// Widevine license type.
/// Ref: license_protocol.proto, LicenseType enum.
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

type ProtoLicenseType = wdv3_proto::LicenseType;

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

/// DRM content protection system identifier.
///
/// Recognizes the major DRM systems by their DASH-IF registered UUIDs.
/// Unrecognized system IDs are captured in the `Unknown` variant.
///
/// Reference: <https://dashif.org/identifiers/content_protection/>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemId {
    Widevine,
    PlayReady,
    FairPlay,
    ClearKey,
    Unknown([u8; 16]),
}

impl SystemId {
    /// Identify a DRM system from its 16-byte UUID.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        use crate::constants::*;
        if bytes_equal(&bytes, &WIDEVINE_SYSTEM_ID) {
            Self::Widevine
        } else if bytes_equal(&bytes, &PLAYREADY_SYSTEM_ID) {
            Self::PlayReady
        } else if bytes_equal(&bytes, &FAIRPLAY_SYSTEM_ID) {
            Self::FairPlay
        } else if bytes_equal(&bytes, &CLEARKEY_SYSTEM_ID) {
            Self::ClearKey
        } else {
            Self::Unknown(bytes)
        }
    }

    /// Return the raw 16-byte UUID for this system.
    pub const fn to_bytes(self) -> [u8; 16] {
        use crate::constants::*;
        match self {
            Self::Widevine => WIDEVINE_SYSTEM_ID,
            Self::PlayReady => PLAYREADY_SYSTEM_ID,
            Self::FairPlay => FAIRPLAY_SYSTEM_ID,
            Self::ClearKey => CLEARKEY_SYSTEM_ID,
            Self::Unknown(bytes) => bytes,
        }
    }

    /// Human-readable name for this system.
    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Widevine => "Widevine",
            Self::PlayReady => "PlayReady",
            Self::FairPlay => "FairPlay",
            Self::ClearKey => "ClearKey",
            Self::Unknown(_) => "Unknown",
        }
    }

    /// Parse a UUID string into a `SystemId`.
    ///
    /// Accepts both hyphenated (`edef8ba9-79d6-4ace-a3c8-27dcd51d21ed`) and
    /// plain (`edef8ba979d64acea3c827dcd51d21ed`) formats. Hex digits are
    /// case-insensitive.
    pub const fn from_uuid(s: &[u8]) -> Option<Self> {
        use crate::utils::hex_digit;

        let mut bytes = [0u8; 16];
        let mut bi = 0; // index into bytes
        let mut si = 0; // index into s

        while si < s.len() {
            if s[si] == b'-' {
                si += 1;
                continue;
            }
            if bi >= 16 {
                return None; // too many hex digits
            }
            // Need two hex digits for one byte
            if si + 1 >= s.len() {
                return None; // odd trailing digit
            }
            let hi = match hex_digit(s[si]) {
                Some(v) => v,
                None => return None,
            };
            let lo = match hex_digit(s[si + 1]) {
                Some(v) => v,
                None => return None,
            };
            bytes[bi] = (hi << 4) | lo;
            bi += 1;
            si += 2;
        }

        if bi != 16 {
            return None; // too few hex digits
        }

        Some(Self::from_bytes(bytes))
    }

    /// Format as a standard UUID string (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
    pub fn to_uuid(self) -> String {
        let b = self.to_bytes();
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15],
        )
    }

    /// Returns `true` for recognized DRM systems.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    /// Returns `true` for unrecognized DRM systems.
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

impl fmt::Display for SystemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.to_name(), self.to_uuid())
    }
}

/// A content decryption key extracted from a license response.
///
/// `Display` prints `kid_hex:key_hex` (e.g. `00000000000000000000000000000001:abcdef0123456789`).
/// `Debug` prints `[CONTENT] kid_hex:key_hex` (prefixed with the key type).
#[derive(Clone, PartialEq, Eq)]
pub struct ContentKey {
    /// Key ID: 16 bytes, from KeyContainer.id (proto field 1),
    /// normalized via kid_to_uuid conversion (see parse_license_response step 8c).
    pub kid: [u8; 16],
    /// Decrypted content key from KeyContainer.key (proto field 3)
    /// after AES-CBC decryption with enc_key and KeyContainer.iv (proto field 2),
    /// then PKCS#7 unpadding. Typically 16 bytes for AES-128 content, but the
    /// protocol does not constrain key length — Vec<u8> is used intentionally.
    pub key: Vec<u8>,
    /// Key type from KeyContainer.type (proto field 4).
    /// All types are decrypted and stored; consumers typically filter to CONTENT for output.
    pub key_type: KeyType,
}

impl ContentKey {
    /// Key ID as a lowercase hex string.
    pub fn kid_hex(&self) -> String {
        hex::encode(self.kid)
    }

    /// Decrypted key as a lowercase hex string.
    pub fn key_hex(&self) -> String {
        hex::encode(&self.key)
    }
}

impl fmt::Display for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", hex::encode(self.kid), hex::encode(&self.key))
    }
}

impl fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}:{}",
            self.key_type,
            hex::encode(self.kid),
            hex::encode(&self.key),
        )
    }
}

/// The three derived keys from a session key.
pub(crate) struct DerivedKeys {
    /// 16 bytes. AES-CMAC(session_key, 0x01 || enc_context).
    /// Used to decrypt KeyContainer.key fields.
    pub(crate) enc_key: [u8; 16],
    /// 32 bytes. CMAC(session_key, 0x01 || mac_context) || CMAC(session_key, 0x02 || mac_context).
    /// Used to verify license response signature via HMAC-SHA256.
    pub(crate) mac_key_server: [u8; 32],
    /// 32 bytes. CMAC(session_key, 0x03 || mac_context) || CMAC(session_key, 0x04 || mac_context).
    /// Used for license renewal requests.
    #[allow(dead_code)]
    pub(crate) mac_key_client: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    fn sample_key() -> ContentKey {
        ContentKey {
            kid: hex!("00000000000000000000000000000001"),
            key: vec![0xab, 0xcd, 0xef, 0x01],
            key_type: KeyType::Content,
        }
    }

    #[test]
    fn content_key_display() {
        let key = sample_key();
        let s = format!("{key}");
        assert_eq!(s, "00000000000000000000000000000001:abcdef01");
    }

    #[test]
    fn content_key_debug() {
        let key = sample_key();
        let s = format!("{key:?}");
        assert_eq!(s, "[CONTENT] 00000000000000000000000000000001:abcdef01");
    }

    #[test]
    fn content_key_debug_signing() {
        let key = ContentKey {
            kid: [0xFF; 16],
            key: vec![0x00],
            key_type: KeyType::Signing,
        };
        let s = format!("{key:?}");
        assert!(s.starts_with("[SIGNING]"));
    }

    #[test]
    fn content_key_hex_accessors() {
        let key = sample_key();
        assert_eq!(key.kid_hex(), "00000000000000000000000000000001");
        assert_eq!(key.key_hex(), "abcdef01");
    }

    #[test]
    fn key_type_display() {
        assert_eq!(format!("{}", KeyType::Content), "CONTENT");
        assert_eq!(format!("{}", KeyType::Signing), "SIGNING");
        assert_eq!(format!("{}", KeyType::KeyControl), "KEY_CONTROL");
        assert_eq!(format!("{}", KeyType::OperatorSession), "OPERATOR_SESSION");
        assert_eq!(format!("{}", KeyType::Entitlement), "ENTITLEMENT");
        assert_eq!(format!("{}", KeyType::OemContent), "OEM_CONTENT");
    }

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
    fn key_type_name_round_trip() {
        let variants = [
            KeyType::Signing,
            KeyType::Content,
            KeyType::KeyControl,
            KeyType::OperatorSession,
            KeyType::Entitlement,
            KeyType::OemContent,
        ];
        for kt in variants {
            let name = kt.to_name();
            let parsed = KeyType::from_name(name.as_bytes()).unwrap();
            assert_eq!(parsed, kt);
        }
    }

    #[test]
    fn key_type_from_name_case_insensitive() {
        assert_eq!(KeyType::from_name(b"signing"), Some(KeyType::Signing));
        assert_eq!(KeyType::from_name(b"CONTENT"), Some(KeyType::Content));
        assert_eq!(
            KeyType::from_name(b"key_control"),
            Some(KeyType::KeyControl)
        );
        assert_eq!(
            KeyType::from_name(b"Operator_Session"),
            Some(KeyType::OperatorSession)
        );
        assert_eq!(
            KeyType::from_name(b"oem_content"),
            Some(KeyType::OemContent)
        );
        assert_eq!(KeyType::from_name(b"nope"), None);
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

    #[test]
    fn system_id_from_known_bytes() {
        use crate::constants::*;
        assert_eq!(SystemId::from_bytes(WIDEVINE_SYSTEM_ID), SystemId::Widevine);
        assert_eq!(
            SystemId::from_bytes(PLAYREADY_SYSTEM_ID),
            SystemId::PlayReady
        );
        assert_eq!(SystemId::from_bytes(FAIRPLAY_SYSTEM_ID), SystemId::FairPlay);
        assert_eq!(SystemId::from_bytes(CLEARKEY_SYSTEM_ID), SystemId::ClearKey);
    }

    #[test]
    fn system_id_to_bytes_round_trip() {
        use crate::constants::*;
        for (id, expected) in [
            (SystemId::Widevine, WIDEVINE_SYSTEM_ID),
            (SystemId::PlayReady, PLAYREADY_SYSTEM_ID),
            (SystemId::FairPlay, FAIRPLAY_SYSTEM_ID),
            (SystemId::ClearKey, CLEARKEY_SYSTEM_ID),
        ] {
            assert_eq!(id.to_bytes(), expected);
            assert_eq!(SystemId::from_bytes(id.to_bytes()), id);
        }
    }

    #[test]
    fn system_id_unknown_preserves_bytes() {
        let bytes: [u8; 16] = hex!("00112233445566778899aabbccddeeff");
        let id = SystemId::from_bytes(bytes);
        assert_eq!(id, SystemId::Unknown(bytes));
        assert_eq!(id.to_bytes(), bytes);
    }

    #[test]
    fn system_id_is_known() {
        assert!(SystemId::Widevine.is_known());
        assert!(SystemId::PlayReady.is_known());
        assert!(SystemId::FairPlay.is_known());
        assert!(SystemId::ClearKey.is_known());
        assert!(!SystemId::Unknown([0; 16]).is_known());

        assert!(!SystemId::Widevine.is_unknown());
        assert!(SystemId::Unknown([0; 16]).is_unknown());
    }

    #[test]
    fn system_id_to_name() {
        assert_eq!(SystemId::Widevine.to_name(), "Widevine");
        assert_eq!(SystemId::PlayReady.to_name(), "PlayReady");
        assert_eq!(SystemId::FairPlay.to_name(), "FairPlay");
        assert_eq!(SystemId::ClearKey.to_name(), "ClearKey");
        assert_eq!(SystemId::Unknown([0; 16]).to_name(), "Unknown");
    }

    #[test]
    fn system_id_display() {
        assert_eq!(
            format!("{}", SystemId::Widevine),
            "Widevine (edef8ba9-79d6-4ace-a3c8-27dcd51d21ed)"
        );
        assert_eq!(
            format!("{}", SystemId::PlayReady),
            "PlayReady (9a04f079-9840-4286-ab92-e65be0885f95)"
        );
        assert_eq!(
            format!("{}", SystemId::FairPlay),
            "FairPlay (94ce86fb-07ff-4f43-adb8-93d2fa968ca2)"
        );
        assert_eq!(
            format!("{}", SystemId::ClearKey),
            "ClearKey (1077efec-c0b2-4d02-ace3-3c1e52e2fb4b)"
        );
        assert_eq!(
            format!("{}", SystemId::Unknown([0; 16])),
            "Unknown (00000000-0000-0000-0000-000000000000)"
        );
    }

    #[test]
    fn system_id_to_uuid() {
        assert_eq!(
            SystemId::Widevine.to_uuid(),
            "edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"
        );
        assert_eq!(
            SystemId::PlayReady.to_uuid(),
            "9a04f079-9840-4286-ab92-e65be0885f95"
        );
        assert_eq!(
            SystemId::Unknown([0; 16]).to_uuid(),
            "00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn system_id_from_uuid_hyphenated() {
        assert_eq!(
            SystemId::from_uuid(b"edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"),
            Some(SystemId::Widevine)
        );
        assert_eq!(
            SystemId::from_uuid(b"9a04f079-9840-4286-ab92-e65be0885f95"),
            Some(SystemId::PlayReady)
        );
        assert_eq!(
            SystemId::from_uuid(b"94ce86fb-07ff-4f43-adb8-93d2fa968ca2"),
            Some(SystemId::FairPlay)
        );
        assert_eq!(
            SystemId::from_uuid(b"1077efec-c0b2-4d02-ace3-3c1e52e2fb4b"),
            Some(SystemId::ClearKey)
        );
    }

    #[test]
    fn system_id_from_uuid_plain() {
        assert_eq!(
            SystemId::from_uuid(b"edef8ba979d64acea3c827dcd51d21ed"),
            Some(SystemId::Widevine)
        );
    }

    #[test]
    fn system_id_from_uuid_case_insensitive() {
        assert_eq!(
            SystemId::from_uuid(b"EDEF8BA9-79D6-4ACE-A3C8-27DCD51D21ED"),
            Some(SystemId::Widevine)
        );
        assert_eq!(
            SystemId::from_uuid(b"Edef8BA9-79d6-4Ace-a3c8-27dcd51d21ED"),
            Some(SystemId::Widevine)
        );
    }

    #[test]
    fn system_id_from_uuid_round_trip() {
        for id in [
            SystemId::Widevine,
            SystemId::PlayReady,
            SystemId::FairPlay,
            SystemId::ClearKey,
            SystemId::Unknown(hex!("00112233445566778899aabbccddeeff")),
        ] {
            let uuid = id.to_uuid();
            assert_eq!(SystemId::from_uuid(uuid.as_bytes()), Some(id));
        }
    }

    #[test]
    fn system_id_from_uuid_invalid() {
        assert_eq!(SystemId::from_uuid(b""), None);
        assert_eq!(SystemId::from_uuid(b"not-a-uuid"), None);
        assert_eq!(
            SystemId::from_uuid(b"edef8ba9-79d6-4ace-a3c8-27dcd51d21"),
            None
        ); // too short
        assert_eq!(
            SystemId::from_uuid(b"edef8ba9-79d6-4ace-a3c8-27dcd51d21edff"),
            None
        ); // too long
        assert_eq!(
            SystemId::from_uuid(b"zdef8ba9-79d6-4ace-a3c8-27dcd51d21ed"),
            None
        ); // invalid hex
    }
}
