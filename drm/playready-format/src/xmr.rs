/*!
    XMR (eXtensible Media Rights) binary license format parsing.
*/

use drm_core::Reader;

use crate::error::FormatError;
use crate::key::{CipherType, KeyType};

// ---------------------------------------------------------------------------
// XMR object type constants
// ---------------------------------------------------------------------------

pub mod object_type {
    pub const OUTER_CONTAINER: u16 = 0x0001;
    pub const GLOBAL_POLICY_CONTAINER: u16 = 0x0002;
    pub const MINIMUM_ENVIRONMENT: u16 = 0x0003;
    pub const PLAYBACK_POLICY_CONTAINER: u16 = 0x0004;
    pub const OUTPUT_PROTECTION: u16 = 0x0005;
    pub const UPLINK_KID: u16 = 0x0006;
    pub const EXPLICIT_ANALOG_VIDEO_CONTAINER: u16 = 0x0007;
    pub const ANALOG_VIDEO_OUTPUT_CONFIG: u16 = 0x0008;
    pub const KEY_MATERIAL_CONTAINER: u16 = 0x0009;
    pub const CONTENT_KEY: u16 = 0x000A;
    pub const SIGNATURE: u16 = 0x000B;
    pub const SERIAL_NUMBER: u16 = 0x000C;
    pub const SETTINGS: u16 = 0x000D;
    pub const COPY_POLICY_CONTAINER: u16 = 0x000E;
    pub const ALLOW_PLAYLISTBURN_CONTAINER: u16 = 0x000F;
    pub const INCLUSION_LIST: u16 = 0x0010;
    pub const PRIORITY: u16 = 0x0011;
    pub const EXPIRATION: u16 = 0x0012;
    pub const ISSUEDATE: u16 = 0x0013;
    pub const EXPIRATION_AFTER_FIRSTUSE: u16 = 0x0014;
    pub const EXPIRATION_AFTER_FIRSTSTORE: u16 = 0x0015;
    pub const METERING: u16 = 0x0016;
    pub const PLAYCOUNT: u16 = 0x0017;
    pub const GRACE_PERIOD: u16 = 0x001A;
    pub const COPYCOUNT: u16 = 0x001B;
    pub const COPY_PROTECTION: u16 = 0x001C;
    pub const REVOCATION_INFO_VERSION: u16 = 0x0020;
    pub const RSA_DEVICE_KEY: u16 = 0x0021;
    pub const SOURCEID: u16 = 0x0022;
    pub const REVOCATION_CONTAINER: u16 = 0x0025;
    pub const RSA_LICENSE_GRANTER_KEY: u16 = 0x0026;
    pub const USERID: u16 = 0x0027;
    pub const RESTRICTED_SOURCEID: u16 = 0x0028;
    pub const DOMAIN_ID: u16 = 0x0029;
    pub const ECC_DEVICE_KEY: u16 = 0x002A;
    pub const GENERATION_NUMBER: u16 = 0x002B;
    pub const POLICY_METADATA: u16 = 0x002C;
    pub const OPTIMIZED_CONTENT_KEY: u16 = 0x002D;
    pub const EXPLICIT_DIGITAL_AUDIO_CONTAINER: u16 = 0x002E;
    pub const RINGTONE_POLICY_CONTAINER: u16 = 0x002F;
    pub const EXPIRATION_AFTER_FIRSTPLAY: u16 = 0x0030;
    pub const DIGITAL_AUDIO_OUTPUT_CONFIG: u16 = 0x0031;
    pub const REVOCATION_INFO_VERSION_2: u16 = 0x0032;
    pub const EMBEDDING_BEHAVIOR: u16 = 0x0033;
    pub const SECURITY_LEVEL: u16 = 0x0034;
    pub const COPY_TO_PC_CONTAINER: u16 = 0x0035;
    pub const PLAY_ENABLER_CONTAINER: u16 = 0x0036;
    pub const MOVE_ENABLER: u16 = 0x0037;
    pub const COPY_ENABLER_CONTAINER: u16 = 0x0038;
    pub const PLAY_ENABLER: u16 = 0x0039;
    pub const COPY_ENABLER: u16 = 0x003A;
    pub const UPLINK_KID_2: u16 = 0x003B;
    pub const COPY_POLICY_2_CONTAINER: u16 = 0x003C;
    pub const COPYCOUNT_2: u16 = 0x003D;
    pub const REMOVAL_DATE: u16 = 0x0050;
    pub const AUX_KEY: u16 = 0x0051;
    pub const UPLINKX: u16 = 0x0052;
    pub const DIGITAL_VIDEO_OUTPUT_CONFIG: u16 = 0x0059;
    pub const SECURESTOP: u16 = 0x005A;
    pub const SECURESTOP2: u16 = 0x005C;
    pub const OPTIMIZED_CONTENT_KEY_2: u16 = 0x005D;
}

// ---------------------------------------------------------------------------
// Magic
// ---------------------------------------------------------------------------

pub const XMR_MAGIC: &[u8; 4] = b"XMR\x00";

// ---------------------------------------------------------------------------
// Structures
// ---------------------------------------------------------------------------

/**
    Parsed XMR license.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmrLicense {
    pub version: u32,
    pub rights_id: [u8; 16],
    pub containers: Vec<XmrObject>,
    /// Original raw bytes for signature verification.
    raw: Vec<u8>,
}

/**
    A single XMR TLV object.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmrObject {
    pub flags: u16,
    pub obj_type: u16,
    pub data: XmrObjectData,
}

/**
    Parsed XMR object data.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmrObjectData {
    Container(Vec<XmrObject>),
    ContentKey(ContentKeyObject),
    Signature(SignatureObject),
    EccKey(EccKeyObject),
    AuxiliaryKeys(AuxiliaryKeysObject),
    OutputProtection(OutputProtectionObject),
    Expiration(ExpirationObject),
    IssueDate(IssueDateObject),
    MeteringRestriction(MeteringRestrictionObject),
    GracePeriod(GracePeriodObject),
    SourceId(SourceIdObject),
    DomainRestriction(DomainRestrictionObject),
    RightsSettings(RightsSettingsObject),
    ExpirationAfterFirstPlay(ExpirationAfterFirstPlayObject),
    RevInfoVersion(RevInfoVersionObject),
    EmbeddedLicenseSettings(EmbeddedLicenseSettingsObject),
    SecurityLevel(SecurityLevelObject),
    MoveEnabler(MoveEnablerObject),
    PlayEnabler(PlayEnablerObject),
    CopyEnabler(CopyEnablerObject),
    UplinkKid(UplinkKidObject),
    CopyCount(CopyCountObject),
    RemovalDate(RemovalDateObject),
    SecureStop(SecureStopObject),
    PolicyMetadata(PolicyMetadataObject),
    UplinkKey3(UplinkKey3Object),
    AnalogVideoOutput(AnalogVideoOutputObject),
    DigitalAudioOutput(DigitalAudioOutputObject),
    DigitalVideoOutput(DigitalVideoOutputObject),
    Unknown(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Leaf object structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentKeyObject {
    pub key_id: [u8; 16],
    pub key_type: KeyType,
    pub cipher_type: CipherType,
    pub encrypted_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureObject {
    pub signature_type: u16,
    pub signature_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EccKeyObject {
    pub curve_type: u16,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxiliaryKeysObject {
    pub keys: Vec<AuxiliaryKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxiliaryKey {
    pub location: u32,
    pub key: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputProtectionObject {
    pub compressed_digital_video: u16,
    pub uncompressed_digital_video: u16,
    pub analog_video: u16,
    pub compressed_digital_audio: u16,
    pub uncompressed_digital_audio: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpirationObject {
    pub begin_date: u32,
    pub end_date: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDateObject {
    pub issue_date: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteringRestrictionObject {
    pub metering_id: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GracePeriodObject {
    pub grace_period: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdObject {
    pub source_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainRestrictionObject {
    pub account_id: [u8; 16],
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RightsSettingsObject {
    pub rights: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpirationAfterFirstPlayObject {
    pub seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevInfoVersionObject {
    pub sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedLicenseSettingsObject {
    pub indicator: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityLevelObject {
    pub minimum_security_level: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveEnablerObject {
    pub minimum_move_protection_level: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayEnablerObject {
    pub play_enabler_type: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyEnablerObject {
    pub copy_enabler_type: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplinkKidObject {
    pub uplink_kid: [u8; 16],
    pub chained_checksum_type: u16,
    pub chained_checksum: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyCountObject {
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalDateObject {
    pub removal_date: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureStopObject {
    pub metering_id: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyMetadataObject {
    pub metadata_type: [u8; 16],
    pub policy_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplinkKey3Object {
    pub uplink_key_id: [u8; 16],
    pub checksum: Vec<u8>,
    pub entries: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalogVideoOutputObject {
    pub video_output_protection_id: [u8; 16],
    pub config_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalAudioOutputObject {
    pub audio_output_protection_id: [u8; 16],
    pub config_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalVideoOutputObject {
    pub video_output_protection_id: [u8; 16],
    pub config_data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl XmrLicense {
    /**
        Parse an XMR license from raw bytes.
    */
    pub fn from_bytes(data: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(data);

        let magic = r.read_bytes(4)?;
        if magic != XMR_MAGIC {
            return Err(FormatError::InvalidMagic {
                expected: "XMR\\x00",
                got: format!("{magic:02x?}"),
            });
        }

        let version = r.read_u32be()?;
        let rights_id = r.read_array::<16>()?;

        let containers = parse_objects(&mut r)?;

        Ok(Self {
            version,
            rights_id,
            containers,
            raw: data.to_vec(),
        })
    }

    /**
        Find all objects of a given type (recursively searches containers).
    */
    pub fn find_objects(&self, obj_type: u16) -> Vec<&XmrObject> {
        let mut result = Vec::new();
        find_objects_recursive(&self.containers, obj_type, &mut result);
        result
    }

    /**
        Find all content key objects.
    */
    pub fn find_content_keys(&self) -> Vec<&ContentKeyObject> {
        self.find_objects(object_type::CONTENT_KEY)
            .into_iter()
            .filter_map(|o| match &o.data {
                XmrObjectData::ContentKey(ck) => Some(ck),
                _ => None,
            })
            .collect()
    }

    /**
        Find the signature object.
    */
    pub fn find_signature(&self) -> Option<&SignatureObject> {
        self.find_objects(object_type::SIGNATURE)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::Signature(s) => Some(s),
                _ => None,
            })
    }

    /**
        Find the ECC device key object.
    */
    pub fn find_ecc_key(&self) -> Option<&EccKeyObject> {
        self.find_objects(object_type::ECC_DEVICE_KEY)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::EccKey(k) => Some(k),
                _ => None,
            })
    }

    /**
        Find the auxiliary keys object.
    */
    pub fn find_auxiliary_keys(&self) -> Option<&AuxiliaryKeysObject> {
        self.find_objects(object_type::AUX_KEY)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::AuxiliaryKeys(ak) => Some(ak),
                _ => None,
            })
    }

    /**
        Returns true if this is a scalable license (has auxiliary keys).
    */
    pub fn is_scalable(&self) -> bool {
        self.find_auxiliary_keys().is_some()
    }

    /**
        Find the expiration restriction object.
    */
    pub fn find_expiration(&self) -> Option<&ExpirationObject> {
        self.find_objects(object_type::EXPIRATION)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::Expiration(e) => Some(e),
                _ => None,
            })
    }

    /**
        Find the output protection level restriction object.
    */
    pub fn find_output_protection(&self) -> Option<&OutputProtectionObject> {
        self.find_objects(object_type::OUTPUT_PROTECTION)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::OutputProtection(op) => Some(op),
                _ => None,
            })
    }

    /**
        Find the minimum security level object.
    */
    pub fn find_security_level(&self) -> Option<&SecurityLevelObject> {
        self.find_objects(object_type::SECURITY_LEVEL)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::SecurityLevel(sl) => Some(sl),
                _ => None,
            })
    }

    /**
        Find the issue date object.
    */
    pub fn find_issue_date(&self) -> Option<&IssueDateObject> {
        self.find_objects(object_type::ISSUEDATE)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::IssueDate(id) => Some(id),
                _ => None,
            })
    }

    /**
        Find the domain restriction object.
    */
    pub fn find_domain_restriction(&self) -> Option<&DomainRestrictionObject> {
        self.find_objects(object_type::DOMAIN_ID)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::DomainRestriction(dr) => Some(dr),
                _ => None,
            })
    }

    /**
        Find the expiration-after-first-play object.
    */
    pub fn find_expiration_after_first_play(&self) -> Option<&ExpirationAfterFirstPlayObject> {
        self.find_objects(object_type::EXPIRATION_AFTER_FIRSTPLAY)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::ExpirationAfterFirstPlay(e) => Some(e),
                _ => None,
            })
    }

    /**
        Find the grace period object.
    */
    pub fn find_grace_period(&self) -> Option<&GracePeriodObject> {
        self.find_objects(object_type::GRACE_PERIOD)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::GracePeriod(gp) => Some(gp),
                _ => None,
            })
    }

    /**
        Find the removal date object.
    */
    pub fn find_removal_date(&self) -> Option<&RemovalDateObject> {
        self.find_objects(object_type::REMOVAL_DATE)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::RemovalDate(rd) => Some(rd),
                _ => None,
            })
    }

    /**
        Find the metering restriction object.
    */
    pub fn find_metering(&self) -> Option<&MeteringRestrictionObject> {
        self.find_objects(object_type::METERING)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::MeteringRestriction(m) => Some(m),
                _ => None,
            })
    }

    /**
        Find the embedded license settings object.
    */
    pub fn find_embedded_license_settings(&self) -> Option<&EmbeddedLicenseSettingsObject> {
        self.find_objects(object_type::EMBEDDING_BEHAVIOR)
            .into_iter()
            .find_map(|o| match &o.data {
                XmrObjectData::EmbeddedLicenseSettings(e) => Some(e),
                _ => None,
            })
    }

    /**
        Get the bytes that should be CMAC-verified.

        This is all raw license bytes except the last `(signature_data_length + 12)` bytes,
        where 12 = 8 (TLV header) + 4 (signature_type + signature_data_length fields).
    */
    pub fn signature_message_bytes(&self) -> Option<&[u8]> {
        let sig = self.find_signature()?;
        let tail_len = sig.signature_data.len() + 12;
        if self.raw.len() >= tail_len {
            Some(&self.raw[..self.raw.len() - tail_len])
        } else {
            None
        }
    }

    /**
        The full raw bytes of this license.
    */
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }
}

fn find_objects_recursive<'a>(
    objects: &'a [XmrObject],
    obj_type: u16,
    result: &mut Vec<&'a XmrObject>,
) {
    for obj in objects {
        if obj.obj_type == obj_type {
            result.push(obj);
        }
        if let XmrObjectData::Container(children) = &obj.data {
            find_objects_recursive(children, obj_type, result);
        }
    }
}

/**
    Parse a sequence of XMR objects from a reader (greedy until exhausted).
*/
fn parse_objects(r: &mut Reader<'_>) -> Result<Vec<XmrObject>, FormatError> {
    let mut objects = Vec::new();
    while r.remaining() >= 8 {
        let obj = parse_object(r)?;
        objects.push(obj);
    }
    Ok(objects)
}

/**
    Parse a single XMR TLV object.
*/
fn parse_object(r: &mut Reader<'_>) -> Result<XmrObject, FormatError> {
    let flags = r.read_u16be()?;
    let obj_type = r.read_u16be()?;
    let raw_length = r.read_u32be()? as usize;

    // The length field includes the 8-byte TLV header (flags + type + length).
    let data_length = raw_length.saturating_sub(8);
    let data_bytes = r.read_bytes(data_length)?;

    // Container if flags bit 1 is set (flags == 2 or 3)
    let is_container = flags & 0x02 != 0;

    let data = if is_container {
        let mut sub_reader = Reader::new(data_bytes);
        let children = parse_objects(&mut sub_reader)?;
        XmrObjectData::Container(children)
    } else {
        parse_leaf(obj_type, data_bytes)?
    };

    Ok(XmrObject {
        flags,
        obj_type,
        data,
    })
}

/**
    Parse a leaf object's data based on its type.
*/
fn parse_leaf(obj_type: u16, data: &[u8]) -> Result<XmrObjectData, FormatError> {
    let mut r = Reader::new(data);
    match obj_type {
        object_type::CONTENT_KEY => {
            let key_id = r.read_array::<16>()?;
            let key_type_raw = r.read_u16be()?;
            let key_type =
                KeyType::from_u16(key_type_raw).ok_or(FormatError::InvalidEnumValue {
                    kind: "KeyType",
                    value: key_type_raw as u32,
                })?;
            let cipher_type_raw = r.read_u16be()?;
            let cipher_type =
                CipherType::from_u16(cipher_type_raw).ok_or(FormatError::InvalidEnumValue {
                    kind: "CipherType",
                    value: cipher_type_raw as u32,
                })?;
            let key_length = r.read_u16be()? as usize;
            let encrypted_key = r.read_bytes(key_length)?.to_vec();
            Ok(XmrObjectData::ContentKey(ContentKeyObject {
                key_id,
                key_type,
                cipher_type,
                encrypted_key,
            }))
        }
        object_type::SIGNATURE => {
            let signature_type = r.read_u16be()?;
            let sig_len = r.read_u16be()? as usize;
            let signature_data = r.read_bytes(sig_len)?.to_vec();
            Ok(XmrObjectData::Signature(SignatureObject {
                signature_type,
                signature_data,
            }))
        }
        object_type::ECC_DEVICE_KEY => {
            let curve_type = r.read_u16be()?;
            let key_len = r.read_u16be()? as usize;
            let key = r.read_bytes(key_len)?.to_vec();
            Ok(XmrObjectData::EccKey(EccKeyObject { curve_type, key }))
        }
        object_type::AUX_KEY => {
            let count = r.read_u16be()? as usize;
            let mut keys = Vec::with_capacity(count);
            for _ in 0..count {
                let location = r.read_u32be()?;
                let key = r.read_array::<16>()?;
                keys.push(AuxiliaryKey { location, key });
            }
            Ok(XmrObjectData::AuxiliaryKeys(AuxiliaryKeysObject { keys }))
        }
        object_type::OUTPUT_PROTECTION => {
            let compressed_digital_video = r.read_u16be()?;
            let uncompressed_digital_video = r.read_u16be()?;
            let analog_video = r.read_u16be()?;
            let compressed_digital_audio = r.read_u16be()?;
            let uncompressed_digital_audio = r.read_u16be()?;
            Ok(XmrObjectData::OutputProtection(OutputProtectionObject {
                compressed_digital_video,
                uncompressed_digital_video,
                analog_video,
                compressed_digital_audio,
                uncompressed_digital_audio,
            }))
        }
        object_type::EXPIRATION => {
            let begin_date = r.read_u32be()?;
            let end_date = r.read_u32be()?;
            Ok(XmrObjectData::Expiration(ExpirationObject {
                begin_date,
                end_date,
            }))
        }
        object_type::ISSUEDATE => {
            let issue_date = r.read_u32be()?;
            Ok(XmrObjectData::IssueDate(IssueDateObject { issue_date }))
        }
        object_type::METERING => {
            let metering_id = r.read_array::<16>()?;
            Ok(XmrObjectData::MeteringRestriction(
                MeteringRestrictionObject { metering_id },
            ))
        }
        object_type::GRACE_PERIOD => {
            let grace_period = r.read_u32be()?;
            Ok(XmrObjectData::GracePeriod(GracePeriodObject {
                grace_period,
            }))
        }
        object_type::SOURCEID => {
            let source_id = r.read_u32be()?;
            Ok(XmrObjectData::SourceId(SourceIdObject { source_id }))
        }
        object_type::DOMAIN_ID => {
            let account_id = r.read_array::<16>()?;
            let revision = r.read_u32be()?;
            Ok(XmrObjectData::DomainRestriction(DomainRestrictionObject {
                account_id,
                revision,
            }))
        }
        object_type::SETTINGS => {
            let rights = r.read_u16be()?;
            Ok(XmrObjectData::RightsSettings(RightsSettingsObject {
                rights,
            }))
        }
        object_type::EXPIRATION_AFTER_FIRSTPLAY => {
            let seconds = r.read_u32be()?;
            Ok(XmrObjectData::ExpirationAfterFirstPlay(
                ExpirationAfterFirstPlayObject { seconds },
            ))
        }
        object_type::REVOCATION_INFO_VERSION | object_type::REVOCATION_INFO_VERSION_2 => {
            let sequence = r.read_u32be()?;
            Ok(XmrObjectData::RevInfoVersion(RevInfoVersionObject {
                sequence,
            }))
        }
        object_type::EMBEDDING_BEHAVIOR => {
            let indicator = r.read_u16be()?;
            Ok(XmrObjectData::EmbeddedLicenseSettings(
                EmbeddedLicenseSettingsObject { indicator },
            ))
        }
        object_type::SECURITY_LEVEL => {
            let minimum_security_level = r.read_u16be()?;
            Ok(XmrObjectData::SecurityLevel(SecurityLevelObject {
                minimum_security_level,
            }))
        }
        object_type::MOVE_ENABLER => {
            let minimum_move_protection_level = r.read_u32be()?;
            Ok(XmrObjectData::MoveEnabler(MoveEnablerObject {
                minimum_move_protection_level,
            }))
        }
        object_type::PLAY_ENABLER => {
            let play_enabler_type = r.read_array::<16>()?;
            Ok(XmrObjectData::PlayEnabler(PlayEnablerObject {
                play_enabler_type,
            }))
        }
        object_type::COPY_ENABLER => {
            let copy_enabler_type = r.read_array::<16>()?;
            Ok(XmrObjectData::CopyEnabler(CopyEnablerObject {
                copy_enabler_type,
            }))
        }
        object_type::UPLINK_KID_2 => {
            let uplink_kid = r.read_array::<16>()?;
            let chained_checksum_type = r.read_u16be()?;
            let chained_checksum_len = r.read_u16be()? as usize;
            let chained_checksum = r.read_bytes(chained_checksum_len)?.to_vec();
            Ok(XmrObjectData::UplinkKid(UplinkKidObject {
                uplink_kid,
                chained_checksum_type,
                chained_checksum,
            }))
        }
        object_type::COPYCOUNT | object_type::COPYCOUNT_2 => {
            let count = r.read_u32be()?;
            Ok(XmrObjectData::CopyCount(CopyCountObject { count }))
        }
        object_type::REMOVAL_DATE => {
            let removal_date = r.read_u32be()?;
            Ok(XmrObjectData::RemovalDate(RemovalDateObject {
                removal_date,
            }))
        }
        object_type::SECURESTOP | object_type::SECURESTOP2 => {
            let metering_id = r.read_array::<16>()?;
            Ok(XmrObjectData::SecureStop(SecureStopObject { metering_id }))
        }
        object_type::POLICY_METADATA => {
            let metadata_type = r.read_array::<16>()?;
            let policy_data = r.read_bytes(r.remaining())?.to_vec();
            Ok(XmrObjectData::PolicyMetadata(PolicyMetadataObject {
                metadata_type,
                policy_data,
            }))
        }
        object_type::UPLINKX => {
            let uplink_key_id = r.read_array::<16>()?;
            let chained_len = r.read_u16be()? as usize;
            let checksum = r.read_bytes(chained_len)?.to_vec();
            let count = r.read_u16be()? as usize;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(r.read_u32be()?);
            }
            Ok(XmrObjectData::UplinkKey3(UplinkKey3Object {
                uplink_key_id,
                checksum,
                entries,
            }))
        }
        object_type::ANALOG_VIDEO_OUTPUT_CONFIG => {
            let video_output_protection_id = r.read_array::<16>()?;
            let config_data = r.read_bytes(r.remaining())?.to_vec();
            Ok(XmrObjectData::AnalogVideoOutput(AnalogVideoOutputObject {
                video_output_protection_id,
                config_data,
            }))
        }
        object_type::DIGITAL_AUDIO_OUTPUT_CONFIG => {
            let audio_output_protection_id = r.read_array::<16>()?;
            let config_data = r.read_bytes(r.remaining())?.to_vec();
            Ok(XmrObjectData::DigitalAudioOutput(
                DigitalAudioOutputObject {
                    audio_output_protection_id,
                    config_data,
                },
            ))
        }
        object_type::DIGITAL_VIDEO_OUTPUT_CONFIG => {
            let video_output_protection_id = r.read_array::<16>()?;
            let config_data = r.read_bytes(r.remaining())?.to_vec();
            Ok(XmrObjectData::DigitalVideoOutput(
                DigitalVideoOutputObject {
                    video_output_protection_id,
                    config_data,
                },
            ))
        }
        _ => Ok(XmrObjectData::Unknown(data.to_vec())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a TLV header: flags(2) + type(2) + length(4), where length includes
    /// the 8-byte header itself.
    fn write_tlv_header(buf: &mut Vec<u8>, flags: u16, obj_type: u16, data_len: usize) {
        buf.extend_from_slice(&flags.to_be_bytes());
        buf.extend_from_slice(&obj_type.to_be_bytes());
        buf.extend_from_slice(&((data_len + 8) as u32).to_be_bytes());
    }

    /// Build a minimal XMR license with a content key and signature.
    fn build_test_xmr() -> Vec<u8> {
        let mut buf = Vec::new();

        // XMR header
        buf.extend_from_slice(XMR_MAGIC);
        buf.extend_from_slice(&1u32.to_be_bytes()); // version
        buf.extend_from_slice(&[0xAA; 16]); // rights_id

        // Outer container (type 0x0001, flags=0x0002 container)
        let mut container_data = Vec::new();

        // Content key object (leaf, type 0x000A)
        let mut ck_data = Vec::new();
        ck_data.extend_from_slice(&[0xBB; 16]); // key_id
        ck_data.extend_from_slice(&1u16.to_be_bytes()); // key_type = Aes128Ctr
        ck_data.extend_from_slice(&3u16.to_be_bytes()); // cipher_type = Ecc256
        let fake_key = [0xCC; 128];
        ck_data.extend_from_slice(&(fake_key.len() as u16).to_be_bytes());
        ck_data.extend_from_slice(&fake_key);

        write_tlv_header(&mut container_data, 0x0000, 0x000A, ck_data.len());
        container_data.extend_from_slice(&ck_data);

        // ECC key object (leaf, type 0x002A)
        let mut ecc_data = Vec::new();
        ecc_data.extend_from_slice(&1u16.to_be_bytes()); // curve_type
        let ecc_key = [0xDD; 64];
        ecc_data.extend_from_slice(&(ecc_key.len() as u16).to_be_bytes());
        ecc_data.extend_from_slice(&ecc_key);

        write_tlv_header(&mut container_data, 0x0000, 0x002A, ecc_data.len());
        container_data.extend_from_slice(&ecc_data);

        // Write outer container
        write_tlv_header(&mut buf, 0x0002, 0x0001, container_data.len());
        buf.extend_from_slice(&container_data);

        // Signature object (leaf, type 0x000B) — outside the container
        let mut sig_data = Vec::new();
        sig_data.extend_from_slice(&1u16.to_be_bytes()); // signature_type
        let sig_bytes = [0xEE; 16];
        sig_data.extend_from_slice(&(sig_bytes.len() as u16).to_be_bytes());
        sig_data.extend_from_slice(&sig_bytes);

        write_tlv_header(&mut buf, 0x0000, 0x000B, sig_data.len());
        buf.extend_from_slice(&sig_data);

        buf
    }

    #[test]
    fn parse_xmr_license() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();

        assert_eq!(license.version, 1);
        assert_eq!(license.rights_id, [0xAA; 16]);
        assert_eq!(license.containers.len(), 2); // outer container + signature
    }

    #[test]
    fn find_content_keys() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();

        let keys = license.find_content_keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_id, [0xBB; 16]);
        assert_eq!(keys[0].key_type, KeyType::Aes128Ctr);
        assert_eq!(keys[0].cipher_type, CipherType::Ecc256);
        assert_eq!(keys[0].encrypted_key.len(), 128);
    }

    #[test]
    fn find_ecc_key() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();

        let ecc = license.find_ecc_key().unwrap();
        assert_eq!(ecc.curve_type, 1);
        assert_eq!(ecc.key.len(), 64);
        assert_eq!(ecc.key, vec![0xDD; 64]);
    }

    #[test]
    fn find_signature() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();

        let sig = license.find_signature().unwrap();
        assert_eq!(sig.signature_type, 1);
        assert_eq!(sig.signature_data.len(), 16);
    }

    #[test]
    fn signature_message_bytes() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();

        let msg = license.signature_message_bytes().unwrap();
        // Total = data.len(), signature tail = 16 (sig data) + 12 = 28
        assert_eq!(msg.len(), data.len() - 28);
    }

    #[test]
    fn not_scalable() {
        let data = build_test_xmr();
        let license = XmrLicense::from_bytes(&data).unwrap();
        assert!(!license.is_scalable());
    }

    #[test]
    fn bad_magic() {
        let data = b"BAD\x00\x00\x00\x00\x01rest";
        let err = XmrLicense::from_bytes(data).unwrap_err();
        assert!(matches!(err, FormatError::InvalidMagic { .. }));
    }
}
