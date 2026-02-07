/*!
    BCert (Binary Certificate) chain format parsing.
*/

use core::fmt;
use core::str::FromStr;

use drm_core::{ParseError, Reader, eq_ignore_ascii_case, trim_ascii};

use crate::error::FormatError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CHAIN_MAGIC: &[u8; 4] = b"CHAI";
pub const CERT_MAGIC: &[u8; 4] = b"CERT";

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/**
    BCert attribute tag.
*/
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttributeTag {
    Basic = 0x0001,
    Domain = 0x0002,
    Pc = 0x0003,
    Device = 0x0004,
    Feature = 0x0005,
    Key = 0x0006,
    Manufacturer = 0x0007,
    Signature = 0x0008,
    Silverlight = 0x0009,
    Metering = 0x000A,
    ExtDataSignKey = 0x000B,
    ExtDataContainer = 0x000C,
    ExtDataSignature = 0x000D,
    ExtDataHwid = 0x000E,
    Server = 0x000F,
    SecurityVersion = 0x0010,
    SecurityVersion2 = 0x0011,
}

impl AttributeTag {
    pub const fn from_u16(u: u16) -> Option<Self> {
        match u {
            0x0001 => Some(Self::Basic),
            0x0002 => Some(Self::Domain),
            0x0003 => Some(Self::Pc),
            0x0004 => Some(Self::Device),
            0x0005 => Some(Self::Feature),
            0x0006 => Some(Self::Key),
            0x0007 => Some(Self::Manufacturer),
            0x0008 => Some(Self::Signature),
            0x0009 => Some(Self::Silverlight),
            0x000A => Some(Self::Metering),
            0x000B => Some(Self::ExtDataSignKey),
            0x000C => Some(Self::ExtDataContainer),
            0x000D => Some(Self::ExtDataSignature),
            0x000E => Some(Self::ExtDataHwid),
            0x000F => Some(Self::Server),
            0x0010 => Some(Self::SecurityVersion),
            0x0011 => Some(Self::SecurityVersion2),
            _ => None,
        }
    }

    pub const fn to_u16(self) -> u16 {
        self as u16
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            5 if eq_ignore_ascii_case(name, b"Basic") => Some(Self::Basic),
            6 if eq_ignore_ascii_case(name, b"Domain") => Some(Self::Domain),
            2 if eq_ignore_ascii_case(name, b"Pc") => Some(Self::Pc),
            6 if eq_ignore_ascii_case(name, b"Device") => Some(Self::Device),
            7 if eq_ignore_ascii_case(name, b"Feature") => Some(Self::Feature),
            3 if eq_ignore_ascii_case(name, b"Key") => Some(Self::Key),
            12 if eq_ignore_ascii_case(name, b"Manufacturer") => Some(Self::Manufacturer),
            9 if eq_ignore_ascii_case(name, b"Signature") => Some(Self::Signature),
            11 if eq_ignore_ascii_case(name, b"Silverlight") => Some(Self::Silverlight),
            8 if eq_ignore_ascii_case(name, b"Metering") => Some(Self::Metering),
            14 if eq_ignore_ascii_case(name, b"ExtDataSignKey") => Some(Self::ExtDataSignKey),
            16 if eq_ignore_ascii_case(name, b"ExtDataContainer") => Some(Self::ExtDataContainer),
            17 if eq_ignore_ascii_case(name, b"ExtDataSignature") => Some(Self::ExtDataSignature),
            11 if eq_ignore_ascii_case(name, b"ExtDataHwid") => Some(Self::ExtDataHwid),
            6 if eq_ignore_ascii_case(name, b"Server") => Some(Self::Server),
            15 if eq_ignore_ascii_case(name, b"SecurityVersion") => Some(Self::SecurityVersion),
            16 if eq_ignore_ascii_case(name, b"SecurityVersion2") => Some(Self::SecurityVersion2),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Domain => "Domain",
            Self::Pc => "Pc",
            Self::Device => "Device",
            Self::Feature => "Feature",
            Self::Key => "Key",
            Self::Manufacturer => "Manufacturer",
            Self::Signature => "Signature",
            Self::Silverlight => "Silverlight",
            Self::Metering => "Metering",
            Self::ExtDataSignKey => "ExtDataSignKey",
            Self::ExtDataContainer => "ExtDataContainer",
            Self::ExtDataSignature => "ExtDataSignature",
            Self::ExtDataHwid => "ExtDataHwid",
            Self::Server => "Server",
            Self::SecurityVersion => "SecurityVersion",
            Self::SecurityVersion2 => "SecurityVersion2",
        }
    }
}

impl fmt::Display for AttributeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for AttributeTag {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "attribute tag",
            value: s.to_owned(),
        })
    }
}

/**
    Certificate type from BasicInfo.
*/
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CertType {
    Unknown = 0,
    Pc = 1,
    Device = 2,
    Domain = 3,
    Issuer = 4,
    CrlSigner = 5,
    Service = 6,
    Silverlight = 7,
    Application = 8,
    Metering = 9,
    KeyFileSigner = 10,
    Server = 11,
    LicenseSigner = 12,
    SecureTimeServer = 13,
    RprovModelAuth = 14,
}

impl CertType {
    pub const fn from_u32(u: u32) -> Option<Self> {
        match u {
            0 => Some(Self::Unknown),
            1 => Some(Self::Pc),
            2 => Some(Self::Device),
            3 => Some(Self::Domain),
            4 => Some(Self::Issuer),
            5 => Some(Self::CrlSigner),
            6 => Some(Self::Service),
            7 => Some(Self::Silverlight),
            8 => Some(Self::Application),
            9 => Some(Self::Metering),
            10 => Some(Self::KeyFileSigner),
            11 => Some(Self::Server),
            12 => Some(Self::LicenseSigner),
            13 => Some(Self::SecureTimeServer),
            14 => Some(Self::RprovModelAuth),
            _ => None,
        }
    }

    pub const fn to_u32(self) -> u32 {
        self as u32
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            7 if eq_ignore_ascii_case(name, b"Unknown") => Some(Self::Unknown),
            2 if eq_ignore_ascii_case(name, b"Pc") => Some(Self::Pc),
            6 if eq_ignore_ascii_case(name, b"Device") => Some(Self::Device),
            6 if eq_ignore_ascii_case(name, b"Domain") => Some(Self::Domain),
            6 if eq_ignore_ascii_case(name, b"Issuer") => Some(Self::Issuer),
            9 if eq_ignore_ascii_case(name, b"CrlSigner") => Some(Self::CrlSigner),
            7 if eq_ignore_ascii_case(name, b"Service") => Some(Self::Service),
            11 if eq_ignore_ascii_case(name, b"Silverlight") => Some(Self::Silverlight),
            11 if eq_ignore_ascii_case(name, b"Application") => Some(Self::Application),
            8 if eq_ignore_ascii_case(name, b"Metering") => Some(Self::Metering),
            13 if eq_ignore_ascii_case(name, b"KeyFileSigner") => Some(Self::KeyFileSigner),
            6 if eq_ignore_ascii_case(name, b"Server") => Some(Self::Server),
            13 if eq_ignore_ascii_case(name, b"LicenseSigner") => Some(Self::LicenseSigner),
            16 if eq_ignore_ascii_case(name, b"SecureTimeServer") => Some(Self::SecureTimeServer),
            14 if eq_ignore_ascii_case(name, b"RprovModelAuth") => Some(Self::RprovModelAuth),
            _ => None,
        }
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Pc => "Pc",
            Self::Device => "Device",
            Self::Domain => "Domain",
            Self::Issuer => "Issuer",
            Self::CrlSigner => "CrlSigner",
            Self::Service => "Service",
            Self::Silverlight => "Silverlight",
            Self::Application => "Application",
            Self::Metering => "Metering",
            Self::KeyFileSigner => "KeyFileSigner",
            Self::Server => "Server",
            Self::LicenseSigner => "LicenseSigner",
            Self::SecureTimeServer => "SecureTimeServer",
            Self::RprovModelAuth => "RprovModelAuth",
        }
    }
}

impl fmt::Display for CertType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for CertType {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "cert type",
            value: s.to_owned(),
        })
    }
}

/**
    Key usage values.
*/
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyUsage {
    Unknown = 0,
    Sign = 1,
    EncryptKey = 2,
    SignCrl = 3,
    IssuerAll = 4,
    IssuerIndiv = 5,
    IssuerDevice = 6,
    IssuerLink = 7,
    IssuerDomain = 8,
    IssuerSilverlight = 9,
    IssuerApplication = 10,
    IssuerCrl = 11,
    IssuerMetering = 12,
    IssuerSignKeyfile = 13,
    SignKeyfile = 14,
    IssuerServer = 15,
    EncryptKeySampleProtectionRc4 = 16,
    Reserved2 = 17,
    IssuerSignLicense = 18,
    SignLicense = 19,
    SignResponse = 20,
    PrndEncryptKeyDeprecated = 21,
    EncryptKeySampleProtectionAes128Ctr = 22,
    IssuerSecureTimeServer = 23,
    IssuerRprovModelAuth = 24,
}

impl KeyUsage {
    pub const fn from_u32(u: u32) -> Option<Self> {
        match u {
            0 => Some(Self::Unknown),
            1 => Some(Self::Sign),
            2 => Some(Self::EncryptKey),
            3 => Some(Self::SignCrl),
            4 => Some(Self::IssuerAll),
            5 => Some(Self::IssuerIndiv),
            6 => Some(Self::IssuerDevice),
            7 => Some(Self::IssuerLink),
            8 => Some(Self::IssuerDomain),
            9 => Some(Self::IssuerSilverlight),
            10 => Some(Self::IssuerApplication),
            11 => Some(Self::IssuerCrl),
            12 => Some(Self::IssuerMetering),
            13 => Some(Self::IssuerSignKeyfile),
            14 => Some(Self::SignKeyfile),
            15 => Some(Self::IssuerServer),
            16 => Some(Self::EncryptKeySampleProtectionRc4),
            17 => Some(Self::Reserved2),
            18 => Some(Self::IssuerSignLicense),
            19 => Some(Self::SignLicense),
            20 => Some(Self::SignResponse),
            21 => Some(Self::PrndEncryptKeyDeprecated),
            22 => Some(Self::EncryptKeySampleProtectionAes128Ctr),
            23 => Some(Self::IssuerSecureTimeServer),
            24 => Some(Self::IssuerRprovModelAuth),
            _ => None,
        }
    }

    pub const fn from_name(name: &[u8]) -> Option<Self> {
        let name = trim_ascii(name);
        match name.len() {
            7 if eq_ignore_ascii_case(name, b"Unknown") => Some(Self::Unknown),
            4 if eq_ignore_ascii_case(name, b"Sign") => Some(Self::Sign),
            10 if eq_ignore_ascii_case(name, b"EncryptKey") => Some(Self::EncryptKey),
            7 if eq_ignore_ascii_case(name, b"SignCrl") => Some(Self::SignCrl),
            9 if eq_ignore_ascii_case(name, b"IssuerAll") => Some(Self::IssuerAll),
            11 if eq_ignore_ascii_case(name, b"IssuerIndiv") => Some(Self::IssuerIndiv),
            12 if eq_ignore_ascii_case(name, b"IssuerDevice") => Some(Self::IssuerDevice),
            10 if eq_ignore_ascii_case(name, b"IssuerLink") => Some(Self::IssuerLink),
            12 if eq_ignore_ascii_case(name, b"IssuerDomain") => Some(Self::IssuerDomain),
            17 if eq_ignore_ascii_case(name, b"IssuerSilverlight") => Some(Self::IssuerSilverlight),
            17 if eq_ignore_ascii_case(name, b"IssuerApplication") => Some(Self::IssuerApplication),
            9 if eq_ignore_ascii_case(name, b"IssuerCrl") => Some(Self::IssuerCrl),
            14 if eq_ignore_ascii_case(name, b"IssuerMetering") => Some(Self::IssuerMetering),
            17 if eq_ignore_ascii_case(name, b"IssuerSignKeyfile") => Some(Self::IssuerSignKeyfile),
            11 if eq_ignore_ascii_case(name, b"SignKeyfile") => Some(Self::SignKeyfile),
            12 if eq_ignore_ascii_case(name, b"IssuerServer") => Some(Self::IssuerServer),
            29 if eq_ignore_ascii_case(name, b"EncryptKeySampleProtectionRc4") => {
                Some(Self::EncryptKeySampleProtectionRc4)
            }
            9 if eq_ignore_ascii_case(name, b"Reserved2") => Some(Self::Reserved2),
            17 if eq_ignore_ascii_case(name, b"IssuerSignLicense") => Some(Self::IssuerSignLicense),
            11 if eq_ignore_ascii_case(name, b"SignLicense") => Some(Self::SignLicense),
            12 if eq_ignore_ascii_case(name, b"SignResponse") => Some(Self::SignResponse),
            24 if eq_ignore_ascii_case(name, b"PrndEncryptKeyDeprecated") => {
                Some(Self::PrndEncryptKeyDeprecated)
            }
            35 if eq_ignore_ascii_case(name, b"EncryptKeySampleProtectionAes128Ctr") => {
                Some(Self::EncryptKeySampleProtectionAes128Ctr)
            }
            22 if eq_ignore_ascii_case(name, b"IssuerSecureTimeServer") => {
                Some(Self::IssuerSecureTimeServer)
            }
            20 if eq_ignore_ascii_case(name, b"IssuerRprovModelAuth") => {
                Some(Self::IssuerRprovModelAuth)
            }
            _ => None,
        }
    }

    pub const fn to_u32(self) -> u32 {
        self as u32
    }

    pub const fn to_name(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Sign => "Sign",
            Self::EncryptKey => "EncryptKey",
            Self::SignCrl => "SignCrl",
            Self::IssuerAll => "IssuerAll",
            Self::IssuerIndiv => "IssuerIndiv",
            Self::IssuerDevice => "IssuerDevice",
            Self::IssuerLink => "IssuerLink",
            Self::IssuerDomain => "IssuerDomain",
            Self::IssuerSilverlight => "IssuerSilverlight",
            Self::IssuerApplication => "IssuerApplication",
            Self::IssuerCrl => "IssuerCrl",
            Self::IssuerMetering => "IssuerMetering",
            Self::IssuerSignKeyfile => "IssuerSignKeyfile",
            Self::SignKeyfile => "SignKeyfile",
            Self::IssuerServer => "IssuerServer",
            Self::EncryptKeySampleProtectionRc4 => "EncryptKeySampleProtectionRc4",
            Self::Reserved2 => "Reserved2",
            Self::IssuerSignLicense => "IssuerSignLicense",
            Self::SignLicense => "SignLicense",
            Self::SignResponse => "SignResponse",
            Self::PrndEncryptKeyDeprecated => "PrndEncryptKeyDeprecated",
            Self::EncryptKeySampleProtectionAes128Ctr => "EncryptKeySampleProtectionAes128Ctr",
            Self::IssuerSecureTimeServer => "IssuerSecureTimeServer",
            Self::IssuerRprovModelAuth => "IssuerRprovModelAuth",
        }
    }
}

impl fmt::Display for KeyUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_name())
    }
}

impl FromStr for KeyUsage {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s.as_bytes()).ok_or_else(|| ParseError {
            kind: "key usage",
            value: s.to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/**
    Parsed BCert certificate chain.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BCertChain {
    pub version: u32,
    pub flags: u32,
    pub certificates: Vec<BCert>,
}

/**
    A single BCert certificate.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BCert {
    pub version: u32,
    pub total_length: u32,
    pub certificate_length: u32,
    pub attributes: Vec<BCertAttribute>,
    /// Raw bytes of this certificate (for signature verification).
    raw: Vec<u8>,
}

/**
    A BCert attribute (TLV).
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BCertAttribute {
    pub flags: u16,
    pub tag: u16,
    pub data: AttributeData,
}

/**
    Parsed attribute data variants.
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeData {
    Basic(BasicInfo),
    Domain(DomainInfo),
    Pc(PcInfo),
    Device(DeviceInfo),
    Feature(FeatureInfo),
    Key(KeyInfo),
    Manufacturer(ManufacturerInfo),
    Signature(SignatureInfo),
    Silverlight(SilverlightInfo),
    Metering(MeteringInfo),
    ExtDataSignKey(ExtDataSignKeyInfo),
    Server(ServerInfo),
    SecurityVersion(SecurityVersionInfo),
    Unknown(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicInfo {
    pub cert_id: [u8; 16],
    pub security_level: u32,
    pub flags: u32,
    pub cert_type: u32,
    pub public_key_digest: [u8; 32],
    pub expiration_date: u32,
    pub client_id: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainInfo {
    pub service_id: [u8; 16],
    pub account_id: [u8; 16],
    pub revision_timestamp: u32,
    pub domain_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcInfo {
    pub security_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub max_license: u32,
    pub max_header: u32,
    pub max_chain_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureInfo {
    pub features: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub keys: Vec<CertKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertKey {
    pub key_type: u16,
    /// Raw public key bytes (X || Y for ECC-256, 64 bytes).
    pub key: Vec<u8>,
    pub flags: u32,
    pub usages: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManufacturerInfo {
    pub flags: u32,
    pub name: String,
    pub model_name: String,
    pub model_number: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInfo {
    pub signature_type: u16,
    pub signature: Vec<u8>,
    /// Issuer's public key that signed this certificate.
    pub signing_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilverlightInfo {
    pub security_version: u32,
    pub platform_identifier: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteringInfo {
    pub metering_id: [u8; 16],
    pub metering_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtDataSignKeyInfo {
    pub key_type: u16,
    pub flags: u32,
    pub key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub warning_days: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityVersionInfo {
    pub security_version: u32,
    pub platform_identifier: u32,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl BCertChain {
    /**
        Parse a BCert chain from raw bytes.
    */
    pub fn from_bytes(data: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(data);

        let magic = r.read_bytes(4)?;
        if magic != CHAIN_MAGIC {
            return Err(FormatError::InvalidMagic {
                expected: "CHAI",
                got: String::from_utf8_lossy(magic).into_owned(),
            });
        }

        let version = r.read_u32be()?;
        let _total_length = r.read_u32be()?;
        let flags = r.read_u32be()?;
        let cert_count = r.read_u32be()? as usize;

        let mut certificates = Vec::with_capacity(cert_count);
        for _ in 0..cert_count {
            let cert = parse_cert(&mut r)?;
            certificates.push(cert);
        }

        Ok(Self {
            version,
            flags,
            certificates,
        })
    }

    /**
        First certificate (leaf / device certificate).
    */
    pub fn leaf(&self) -> Option<&BCert> {
        self.certificates.first()
    }

    /**
        Last certificate (root / issuer certificate).
    */
    pub fn root(&self) -> Option<&BCert> {
        self.certificates.last()
    }
}

impl BCert {
    /**
        Get the BasicInfo attribute if present.
    */
    pub fn basic_info(&self) -> Option<&BasicInfo> {
        self.attributes.iter().find_map(|a| match &a.data {
            AttributeData::Basic(info) => Some(info),
            _ => None,
        })
    }

    /**
        Get the KeyInfo attribute if present.
    */
    pub fn key_info(&self) -> Option<&KeyInfo> {
        self.attributes.iter().find_map(|a| match &a.data {
            AttributeData::Key(info) => Some(info),
            _ => None,
        })
    }

    /**
        Get the SignatureInfo attribute if present.
    */
    pub fn signature_info(&self) -> Option<&SignatureInfo> {
        self.attributes.iter().find_map(|a| match &a.data {
            AttributeData::Signature(info) => Some(info),
            _ => None,
        })
    }

    /**
        Get the first key matching the given usage value.
    */
    pub fn key_by_usage(&self, usage: u32) -> Option<&[u8]> {
        self.key_info().and_then(|ki| {
            ki.keys
                .iter()
                .find(|k| k.usages.contains(&usage))
                .map(|k| k.key.as_slice())
        })
    }

    /**
        Get the first key with `Sign` (1) usage.
    */
    pub fn signing_key(&self) -> Option<&[u8]> {
        self.key_info().and_then(|ki| {
            ki.keys
                .iter()
                .find(|k| k.usages.contains(&KeyUsage::Sign.to_u32()))
                .map(|k| k.key.as_slice())
        })
    }

    /**
        Get the first key with `EncryptKey` (2) usage.
    */
    pub fn encryption_key(&self) -> Option<&[u8]> {
        self.key_info().and_then(|ki| {
            ki.keys
                .iter()
                .find(|k| k.usages.contains(&KeyUsage::EncryptKey.to_u32()))
                .map(|k| k.key.as_slice())
        })
    }

    /**
        Raw bytes covered by the signature: \[0..certificate_length\] from the cert start.
    */
    pub fn signed_bytes(&self) -> &[u8] {
        let end = (self.certificate_length as usize).min(self.raw.len());
        &self.raw[..end]
    }
}

// ---------------------------------------------------------------------------
// Internal parsing
// ---------------------------------------------------------------------------

fn parse_cert(r: &mut Reader<'_>) -> Result<BCert, FormatError> {
    let cert_start = r.position();

    let magic = r.read_bytes(4)?;
    if magic != CERT_MAGIC {
        return Err(FormatError::InvalidMagic {
            expected: "CERT",
            got: String::from_utf8_lossy(magic).into_owned(),
        });
    }

    let version = r.read_u32be()?;
    let total_length = r.read_u32be()?;
    let certificate_length = r.read_u32be()?;

    let cert_end = cert_start + total_length as usize;
    let mut attributes = Vec::new();

    while r.position() < cert_end && r.remaining() >= 8 {
        let attr = parse_attribute(r)?;
        attributes.push(attr);
    }

    let raw_end = cert_end.min(r.data().len());
    let raw = r.data()[cert_start..raw_end].to_vec();

    // Advance past any remaining bytes in this cert
    let skip = cert_end.min(r.data().len()).saturating_sub(r.position());
    if skip > 0 {
        r.read_bytes(skip)?;
    }

    Ok(BCert {
        version,
        total_length,
        certificate_length,
        attributes,
        raw,
    })
}

fn parse_attribute(r: &mut Reader<'_>) -> Result<BCertAttribute, FormatError> {
    let flags = r.read_u16be()?;
    let tag = r.read_u16be()?;
    let length = r.read_u32be()? as usize; // includes 8-byte header

    let data_len = length.saturating_sub(8);
    let data_bytes = r.read_bytes(data_len)?;

    let data = match AttributeTag::from_u16(tag) {
        Some(AttributeTag::Basic) => parse_basic(data_bytes)?,
        Some(AttributeTag::Domain) => parse_domain(data_bytes)?,
        Some(AttributeTag::Pc) => parse_pc(data_bytes)?,
        Some(AttributeTag::Device) => parse_device(data_bytes)?,
        Some(AttributeTag::Feature) => parse_feature(data_bytes)?,
        Some(AttributeTag::Key) => parse_key(data_bytes)?,
        Some(AttributeTag::Manufacturer) => parse_manufacturer(data_bytes)?,
        Some(AttributeTag::Signature) => parse_signature(data_bytes)?,
        Some(AttributeTag::Silverlight) => parse_silverlight(data_bytes)?,
        Some(AttributeTag::Metering) => parse_metering(data_bytes)?,
        Some(AttributeTag::ExtDataSignKey) => parse_ext_data_sign_key(data_bytes)?,
        Some(AttributeTag::Server) => parse_server(data_bytes)?,
        Some(AttributeTag::SecurityVersion | AttributeTag::SecurityVersion2) => {
            parse_security_version(data_bytes)?
        }
        // Unknown or container tags — store raw bytes
        _ => AttributeData::Unknown(data_bytes.to_vec()),
    };

    Ok(BCertAttribute { flags, tag, data })
}

// ---------------------------------------------------------------------------
// Attribute data parsers
// ---------------------------------------------------------------------------

fn parse_basic(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let cert_id = r.read_array::<16>()?;
    let security_level = r.read_u32be()?;
    let flags = r.read_u32be()?;
    let cert_type = r.read_u32be()?;
    let public_key_digest = r.read_array::<32>()?;
    let expiration_date = r.read_u32be()?;
    let client_id = r.read_array::<16>()?;
    Ok(AttributeData::Basic(BasicInfo {
        cert_id,
        security_level,
        flags,
        cert_type,
        public_key_digest,
        expiration_date,
        client_id,
    }))
}

fn parse_domain(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let service_id = r.read_array::<16>()?;
    let account_id = r.read_array::<16>()?;
    let revision_timestamp = r.read_u32be()?;
    let url_len = r.read_u32be()? as usize;
    let domain_url = r.read_padded_string(url_len)?;
    Ok(AttributeData::Domain(DomainInfo {
        service_id,
        account_id,
        revision_timestamp,
        domain_url,
    }))
}

fn parse_pc(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let security_version = r.read_u32be()?;
    Ok(AttributeData::Pc(PcInfo { security_version }))
}

fn parse_device(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let max_license = r.read_u32be()?;
    let max_header = r.read_u32be()?;
    let max_chain_depth = r.read_u32be()?;
    Ok(AttributeData::Device(DeviceInfo {
        max_license,
        max_header,
        max_chain_depth,
    }))
}

fn parse_feature(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let count = r.read_u32be()? as usize;
    let mut features = Vec::with_capacity(count.min(32));
    for _ in 0..count.min(32) {
        features.push(r.read_u32be()?);
    }
    Ok(AttributeData::Feature(FeatureInfo { features }))
}

fn parse_key(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let key_count = r.read_u32be()? as usize;
    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        let key_type = r.read_u16be()?;
        let key_length_bits = r.read_u16be()? as usize;
        let key_length_bytes = key_length_bits / 8;
        let flags = r.read_u32be()?;
        let key = r.read_bytes(key_length_bytes)?.to_vec();
        let usages_count = r.read_u32be()? as usize;
        let mut usages = Vec::with_capacity(usages_count);
        for _ in 0..usages_count {
            usages.push(r.read_u32be()?);
        }
        keys.push(CertKey {
            key_type,
            key,
            flags,
            usages,
        });
    }
    Ok(AttributeData::Key(KeyInfo { keys }))
}

fn parse_manufacturer(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let flags = r.read_u32be()?;
    let name_len = r.read_u32be()? as usize;
    let name = r.read_padded_string(name_len)?;
    let model_name_len = r.read_u32be()? as usize;
    let model_name = r.read_padded_string(model_name_len)?;
    let model_number_len = r.read_u32be()? as usize;
    let model_number = r.read_padded_string(model_number_len)?;
    Ok(AttributeData::Manufacturer(ManufacturerInfo {
        flags,
        name,
        model_name,
        model_number,
    }))
}

fn parse_signature(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let signature_type = r.read_u16be()?;
    let signature_size = r.read_u16be()? as usize;
    let signature = r.read_bytes(signature_size)?.to_vec();
    let signing_key_size_bits = r.read_u32be()? as usize;
    let signing_key_size_bytes = signing_key_size_bits / 8;
    let signing_key = r.read_bytes(signing_key_size_bytes)?.to_vec();
    Ok(AttributeData::Signature(SignatureInfo {
        signature_type,
        signature,
        signing_key,
    }))
}

fn parse_silverlight(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let security_version = r.read_u32be()?;
    let platform_identifier = r.read_u32be()?;
    Ok(AttributeData::Silverlight(SilverlightInfo {
        security_version,
        platform_identifier,
    }))
}

fn parse_metering(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let metering_id = r.read_array::<16>()?;
    let url_len = r.read_u32be()? as usize;
    let metering_url = r.read_padded_string(url_len)?;
    Ok(AttributeData::Metering(MeteringInfo {
        metering_id,
        metering_url,
    }))
}

fn parse_ext_data_sign_key(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let key_type = r.read_u16be()?;
    let key_length_bits = r.read_u16be()? as usize;
    let flags = r.read_u32be()?;
    let key = r.read_bytes(key_length_bits / 8)?.to_vec();
    Ok(AttributeData::ExtDataSignKey(ExtDataSignKeyInfo {
        key_type,
        flags,
        key,
    }))
}

fn parse_server(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let warning_days = r.read_u32be()?;
    Ok(AttributeData::Server(ServerInfo { warning_days }))
}

fn parse_security_version(data: &[u8]) -> Result<AttributeData, FormatError> {
    let mut r = Reader::new(data);
    let security_version = r.read_u32be()?;
    let platform_identifier = r.read_u32be()?;
    Ok(AttributeData::SecurityVersion(SecurityVersionInfo {
        security_version,
        platform_identifier,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_chain() -> Vec<u8> {
        let mut cert_body = Vec::new();

        // BasicInfo attribute: flags(2) + tag(2) + length(4) + data(80)
        let basic_data_len = 16 + 4 + 4 + 4 + 32 + 4 + 16;
        let attr_total_len = 8 + basic_data_len;
        cert_body.extend_from_slice(&0x0001u16.to_be_bytes());
        cert_body.extend_from_slice(&0x0001u16.to_be_bytes());
        cert_body.extend_from_slice(&(attr_total_len as u32).to_be_bytes());
        cert_body.extend_from_slice(&[0x01; 16]); // cert_id
        cert_body.extend_from_slice(&3000u32.to_be_bytes()); // security_level
        cert_body.extend_from_slice(&0u32.to_be_bytes()); // flags
        cert_body.extend_from_slice(&2u32.to_be_bytes()); // cert_type = Device
        cert_body.extend_from_slice(&[0x02; 32]); // public_key_digest
        cert_body.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes()); // expiration
        cert_body.extend_from_slice(&[0x03; 16]); // client_id

        let total_length = 16 + cert_body.len() as u32;
        let mut cert = Vec::new();
        cert.extend_from_slice(CERT_MAGIC);
        cert.extend_from_slice(&1u32.to_be_bytes());
        cert.extend_from_slice(&total_length.to_be_bytes());
        cert.extend_from_slice(&(cert_body.len() as u32).to_be_bytes());
        cert.extend_from_slice(&cert_body);

        let chain_total = 20 + cert.len();
        let mut chain = Vec::new();
        chain.extend_from_slice(CHAIN_MAGIC);
        chain.extend_from_slice(&1u32.to_be_bytes());
        chain.extend_from_slice(&(chain_total as u32).to_be_bytes());
        chain.extend_from_slice(&0u32.to_be_bytes());
        chain.extend_from_slice(&1u32.to_be_bytes());
        chain.extend_from_slice(&cert);

        chain
    }

    #[test]
    fn parse_basic_chain() {
        let data = build_test_chain();
        let chain = BCertChain::from_bytes(&data).unwrap();

        assert_eq!(chain.version, 1);
        assert_eq!(chain.certificates.len(), 1);

        let cert = &chain.certificates[0];
        let basic = cert.basic_info().unwrap();
        assert_eq!(basic.cert_id, [0x01; 16]);
        assert_eq!(basic.security_level, 3000);
        assert_eq!(basic.cert_type, CertType::Device.to_u32());
        assert_eq!(basic.public_key_digest, [0x02; 32]);
        assert_eq!(basic.expiration_date, 0xFFFFFFFF);
        assert_eq!(basic.client_id, [0x03; 16]);
    }

    #[test]
    fn leaf_and_root() {
        let data = build_test_chain();
        let chain = BCertChain::from_bytes(&data).unwrap();

        assert!(chain.leaf().is_some());
        assert!(chain.root().is_some());
        assert_eq!(
            chain.leaf().unwrap().basic_info().unwrap().cert_id,
            chain.root().unwrap().basic_info().unwrap().cert_id,
        );
    }

    #[test]
    fn bad_chain_magic() {
        let data = b"XXXX\x00\x00\x00\x01";
        let err = BCertChain::from_bytes(data).unwrap_err();
        assert!(matches!(err, FormatError::InvalidMagic { .. }));
    }

    #[test]
    fn unknown_attribute_tag() {
        let mut cert_body = Vec::new();
        cert_body.extend_from_slice(&0x0000u16.to_be_bytes());
        cert_body.extend_from_slice(&0xFFFDu16.to_be_bytes());
        cert_body.extend_from_slice(&12u32.to_be_bytes());
        cert_body.extend_from_slice(&[0xAA; 4]);

        let total_length = 16 + cert_body.len() as u32;
        let mut cert = Vec::new();
        cert.extend_from_slice(CERT_MAGIC);
        cert.extend_from_slice(&1u32.to_be_bytes());
        cert.extend_from_slice(&total_length.to_be_bytes());
        cert.extend_from_slice(&(cert_body.len() as u32).to_be_bytes());
        cert.extend_from_slice(&cert_body);

        let chain_total = 20 + cert.len();
        let mut chain = Vec::new();
        chain.extend_from_slice(CHAIN_MAGIC);
        chain.extend_from_slice(&1u32.to_be_bytes());
        chain.extend_from_slice(&(chain_total as u32).to_be_bytes());
        chain.extend_from_slice(&0u32.to_be_bytes());
        chain.extend_from_slice(&1u32.to_be_bytes());
        chain.extend_from_slice(&cert);

        let parsed = BCertChain::from_bytes(&chain).unwrap();
        assert_eq!(parsed.certificates[0].attributes.len(), 1);
        assert!(matches!(
            parsed.certificates[0].attributes[0].data,
            AttributeData::Unknown(_)
        ));
    }

    #[test]
    fn attribute_tag_round_trip() {
        for tag in [
            AttributeTag::Basic,
            AttributeTag::Key,
            AttributeTag::Signature,
            AttributeTag::SecurityVersion2,
        ] {
            assert_eq!(AttributeTag::from_u16(tag.to_u16()), Some(tag));
            let name = tag.to_name();
            let parsed: AttributeTag = name.parse().unwrap();
            assert_eq!(parsed, tag);
        }
    }

    #[test]
    fn cert_type_round_trip() {
        for ct in [CertType::Device, CertType::Issuer, CertType::LicenseSigner] {
            assert_eq!(CertType::from_u32(ct.to_u32()), Some(ct));
            let name = ct.to_name();
            let parsed: CertType = name.parse().unwrap();
            assert_eq!(parsed, ct);
        }
    }

    #[test]
    fn key_usage_round_trip() {
        for ku in [KeyUsage::Sign, KeyUsage::EncryptKey, KeyUsage::SignLicense] {
            assert_eq!(KeyUsage::from_u32(ku.to_u32()), Some(ku));
            let name = ku.to_name();
            let parsed: KeyUsage = name.parse().unwrap();
            assert_eq!(parsed, ku);
        }
    }
}
