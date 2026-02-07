use p256::{
    ProjectivePoint, Scalar,
    elliptic_curve::{PrimeField, sec1::ToEncodedPoint},
};

use drm_core::Reader;
use drm_playready_format::bcert::BCertChain;

use crate::error::{CdmError, CdmResult};
use crate::types::SecurityLevel;

const MAGIC: &[u8] = b"PRD";

/**
    An ECC P-256 keypair (32-byte private scalar + 64-byte uncompressed public point).
*/
#[derive(Debug, Clone)]
pub(crate) struct EccKeyPair {
    pub private_key: [u8; 32],
    pub public_key: [u8; 64],
}

/**
    Represents a PlayReady device loaded from a `.prd` file.

    The device holds three ECC P-256 keypairs and a group certificate chain.
    The encryption key is used for ElGamal decryption of content keys,
    the signing key for ECDSA-SHA256 challenge signing, and the group key
    (v3 only) for provisioning.
*/
#[derive(Debug, Clone)]
pub struct Device {
    /// Security level extracted from the leaf BCert's BasicInfo.
    pub security_level: SecurityLevel,
    /// Group key — signs new leaf BCerts during provisioning. Only present in PRD v3.
    pub(crate) group_key: Option<EccKeyPair>,
    /// Encryption key — ElGamal decryption of content keys in license responses.
    pub(crate) encryption_key: EccKeyPair,
    /// Signing key — ECDSA-SHA256 signing of license challenge XML.
    pub(crate) signing_key: EccKeyPair,
    /// Raw group certificate chain bytes.
    pub(crate) group_certificate: Vec<u8>,
}

impl Device {
    /**
        Create a new device from raw components.

        Each ECC keypair is 96 bytes: 32-byte private scalar + 64-byte uncompressed
        public point (X || Y). The group key is optional (only present in PRD v3 devices).
        The group certificate is the raw BCert chain bytes (starts with `CHAI`).
    */
    pub fn new(
        encryption_key: [u8; 96],
        signing_key: [u8; 96],
        group_key: Option<[u8; 96]>,
        group_certificate: Vec<u8>,
    ) -> CdmResult<Self> {
        let encryption_key = split_keypair(encryption_key);
        let signing_key = split_keypair(signing_key);
        let group_key = group_key.map(split_keypair);
        let security_level = extract_security_level(&group_certificate)?;

        Ok(Self {
            security_level,
            group_key,
            encryption_key,
            signing_key,
            group_certificate,
        })
    }

    /**
        Create a new device from 32-byte ECC private keys, deriving public keys via P-256.

        This is the common case when assembling a device from provisioning data
        (e.g. raw key files). The public keys are computed from the private scalars
        using P-256 scalar-basepoint multiplication.
    */
    pub fn from_private_keys(
        encryption_key: [u8; 32],
        signing_key: [u8; 32],
        group_key: Option<[u8; 32]>,
        group_certificate: Vec<u8>,
    ) -> CdmResult<Self> {
        let encryption_key = derive_keypair(encryption_key)?;
        let signing_key = derive_keypair(signing_key)?;
        let group_key = group_key.map(derive_keypair).transpose()?;
        let security_level = extract_security_level(&group_certificate)?;

        Ok(Self {
            security_level,
            group_key,
            encryption_key,
            signing_key,
            group_certificate,
        })
    }

    /**
        Parse a PRD file from raw bytes.

        Supports PRD v2 (no group key) and v3 (with group key).
    */
    pub fn from_bytes(data: impl AsRef<[u8]>) -> CdmResult<Self> {
        let data = data.as_ref();
        let mut r = Reader::new(data);

        // Check magic bytes
        let magic = r.read_bytes(3).map_err(|_| CdmError::PrdTruncated)?;
        if magic != MAGIC {
            return Err(CdmError::PrdBadMagic);
        }

        // Read version
        let version = r.read_array::<1>().map_err(|_| CdmError::PrdTruncated)?[0];

        match version {
            2 => Self::parse_v2(&mut r),
            3 => Self::parse_v3(&mut r),
            _ => Err(CdmError::PrdUnsupportedVersion(version)),
        }
    }

    /**
        Parse a base64-encoded PRD file.
    */
    pub fn from_base64(prd: impl AsRef<[u8]>) -> CdmResult<Self> {
        let bytes = data_encoding::BASE64
            .decode(prd.as_ref())
            .map_err(|e| CdmError::InvalidBase64(format!("PRD: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /**
        Serialize to PRD v3 format bytes.

        Always writes v3 format regardless of the version originally loaded.
        If the device was loaded from v2 (no group key), the group key
        field is written as all zeros.
    */
    pub fn to_bytes(&self) -> Vec<u8> {
        let cert_len = self.group_certificate.len() as u32;
        let total = 3 + 1 + 96 + 96 + 96 + 4 + self.group_certificate.len();
        let mut buf = Vec::with_capacity(total);

        // Header
        buf.extend(MAGIC);
        buf.push(3u8);

        // Group key (96 bytes) — zeros if absent
        match &self.group_key {
            Some(kp) => {
                buf.extend(&kp.private_key);
                buf.extend(&kp.public_key);
            }
            None => buf.extend(&[0u8; 96]),
        }

        // Encryption key (96 bytes)
        buf.extend(&self.encryption_key.private_key);
        buf.extend(&self.encryption_key.public_key);

        // Signing key (96 bytes)
        buf.extend(&self.signing_key.private_key);
        buf.extend(&self.signing_key.public_key);

        // Certificate chain
        buf.extend(&cert_len.to_be_bytes());
        buf.extend(&self.group_certificate);

        buf
    }

    /**
        Serialize to a base64-encoded PRD string.
    */
    pub fn to_base64(&self) -> String {
        data_encoding::BASE64.encode(&self.to_bytes())
    }

    /**
        Parse the group certificate chain from the stored raw bytes.
    */
    pub fn group_certificate_chain(&self) -> CdmResult<BCertChain> {
        BCertChain::from_bytes(&self.group_certificate).map_err(CdmError::from)
    }

    /**
        Returns the encryption private key (32 bytes).
    */
    pub fn encryption_private_key(&self) -> &[u8; 32] {
        &self.encryption_key.private_key
    }

    /**
        Returns the encryption public key (64 bytes, X || Y).
    */
    pub fn encryption_public_key(&self) -> &[u8; 64] {
        &self.encryption_key.public_key
    }

    /**
        Returns the signing private key (32 bytes).
    */
    pub fn signing_private_key(&self) -> &[u8; 32] {
        &self.signing_key.private_key
    }

    /**
        Returns the signing public key (64 bytes, X || Y).
    */
    pub fn signing_public_key(&self) -> &[u8; 64] {
        &self.signing_key.public_key
    }

    /**
        Returns the group private key (32 bytes), if present.
    */
    pub fn group_private_key(&self) -> Option<&[u8; 32]> {
        self.group_key.as_ref().map(|kp| &kp.private_key)
    }

    /**
        Returns the raw group certificate chain bytes.
    */
    pub fn group_certificate_bytes(&self) -> &[u8] {
        &self.group_certificate
    }

    /**
        Returns `true` if this device has a group key (PRD v3 with non-zero group key).
    */
    pub fn has_group_key(&self) -> bool {
        self.group_key.is_some()
    }

    /**
        PRD v2: cert_len(4) + cert + enc_key(96) + sign_key(96)
    */
    fn parse_v2(r: &mut Reader<'_>) -> CdmResult<Self> {
        let cert_len = r.read_u32be().map_err(|_| CdmError::PrdTruncated)? as usize;
        let cert_bytes = r.read_bytes(cert_len).map_err(|_| CdmError::PrdTruncated)?;
        let encryption_key = read_ecc_keypair(r)?;
        let signing_key = read_ecc_keypair(r)?;

        let security_level = extract_security_level(cert_bytes)?;

        Ok(Self {
            security_level,
            group_key: None,
            encryption_key,
            signing_key,
            group_certificate: cert_bytes.to_vec(),
        })
    }

    /**
        PRD v3: group_key(96) + enc_key(96) + sign_key(96) + cert_len(4) + cert
    */
    fn parse_v3(r: &mut Reader<'_>) -> CdmResult<Self> {
        let group_key = read_ecc_keypair(r)?;
        let encryption_key = read_ecc_keypair(r)?;
        let signing_key = read_ecc_keypair(r)?;

        let cert_len = r.read_u32be().map_err(|_| CdmError::PrdTruncated)? as usize;
        let cert_bytes = r.read_bytes(cert_len).map_err(|_| CdmError::PrdTruncated)?;

        let security_level = extract_security_level(cert_bytes)?;

        // Check if group key is all zeros (absent)
        let has_group_key = group_key.private_key != [0u8; 32];

        Ok(Self {
            security_level,
            group_key: if has_group_key { Some(group_key) } else { None },
            encryption_key,
            signing_key,
            group_certificate: cert_bytes.to_vec(),
        })
    }
}

/**
    Derive a full ECC keypair from a 32-byte private scalar via P-256 basepoint multiplication.
*/
fn derive_keypair(private_key: [u8; 32]) -> CdmResult<EccKeyPair> {
    // Reject zero scalar (identity point is not a valid public key)
    if private_key == [0u8; 32] {
        return Err(CdmError::EccKeyParse("private key scalar is zero".into()));
    }

    let scalar = Option::<Scalar>::from(Scalar::from_repr(*p256::FieldBytes::from_slice(
        &private_key,
    )))
    .ok_or_else(|| CdmError::EccKeyParse("invalid private key scalar".into()))?;

    let point = (ProjectivePoint::GENERATOR * scalar).to_affine();
    let encoded = point.to_encoded_point(false);
    let bytes = encoded.as_bytes();

    if bytes.len() < 65 {
        return Err(CdmError::EccKeyParse("derived point is identity".into()));
    }

    let mut public_key = [0u8; 64];
    public_key.copy_from_slice(&bytes[1..65]);

    Ok(EccKeyPair {
        private_key,
        public_key,
    })
}

/**
    Split a 96-byte buffer into an ECC keypair (32 private + 64 public).
*/
fn split_keypair(buf: [u8; 96]) -> EccKeyPair {
    let mut private_key = [0u8; 32];
    let mut public_key = [0u8; 64];
    private_key.copy_from_slice(&buf[..32]);
    public_key.copy_from_slice(&buf[32..]);
    EccKeyPair {
        private_key,
        public_key,
    }
}

/**
    Read a 96-byte ECC keypair (32 private + 64 public) from the reader.
*/
fn read_ecc_keypair(r: &mut Reader<'_>) -> CdmResult<EccKeyPair> {
    let private_key = r.read_array::<32>().map_err(|_| CdmError::PrdTruncated)?;
    let public_key = r.read_array::<64>().map_err(|_| CdmError::PrdTruncated)?;
    Ok(EccKeyPair {
        private_key,
        public_key,
    })
}

/**
    Extract the security level from a raw BCertChain by parsing and reading the leaf cert.
*/
fn extract_security_level(cert_bytes: &[u8]) -> CdmResult<SecurityLevel> {
    let chain = BCertChain::from_bytes(cert_bytes).map_err(CdmError::from)?;
    let leaf = chain
        .leaf()
        .ok_or_else(|| CdmError::Format("BCert chain has no certificates".into()))?;
    let basic_info = leaf
        .basic_info()
        .ok_or_else(|| CdmError::Format("leaf BCert has no BasicInfo attribute".into()))?;
    Ok(SecurityLevel::from_u32(basic_info.security_level))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PRD_V3: &[u8] = include_bytes!("../testfiles/device.prd");
    const TEST_PRD_V2: &[u8] = include_bytes!("../testfiles/device_v2.prd");

    fn test_device() -> Device {
        Device::from_bytes(TEST_PRD_V3).expect("failed to parse test PRD v3")
    }

    // ── Successful parsing ────────────────────────────────────────────

    #[test]
    fn parse_test_device_v3() {
        let device = Device::from_bytes(TEST_PRD_V3).unwrap();
        assert_eq!(device.security_level, SecurityLevel::SL3000);
        assert!(device.group_key.is_some());
    }

    #[test]
    fn parse_test_device_v2() {
        let device = Device::from_bytes(TEST_PRD_V2).unwrap();
        assert_eq!(device.security_level, SecurityLevel::SL3000);
        assert!(device.group_key.is_none());
    }

    #[test]
    fn round_trip() {
        let device = test_device();
        let serialized = device.to_bytes();
        let device2 = Device::from_bytes(&serialized).unwrap();
        assert_eq!(device2.security_level, device.security_level);
        assert_eq!(
            device2.encryption_key.private_key,
            device.encryption_key.private_key
        );
        assert_eq!(
            device2.signing_key.private_key,
            device.signing_key.private_key
        );
        assert_eq!(device2.group_certificate, device.group_certificate);
    }

    #[test]
    fn base64_round_trip() {
        let device = test_device();
        let b64 = device.to_base64();
        let device2 = Device::from_base64(&b64).unwrap();
        assert_eq!(device2.security_level, device.security_level);
        assert_eq!(
            device2.encryption_key.private_key,
            device.encryption_key.private_key
        );
    }

    #[test]
    fn v2_round_trips_as_v3() {
        let device = Device::from_bytes(TEST_PRD_V2).unwrap();
        // to_bytes always writes v3
        let serialized = device.to_bytes();
        assert_eq!(&serialized[..3], b"PRD");
        assert_eq!(serialized[3], 3);
        let device2 = Device::from_bytes(&serialized).unwrap();
        assert_eq!(device2.security_level, device.security_level);
        assert!(device2.group_key.is_none());
    }

    #[test]
    fn group_certificate_chain_parses() {
        let device = test_device();
        let chain = device.group_certificate_chain().unwrap();
        assert!(chain.leaf().is_some());
        assert!(chain.root().is_some());
    }

    #[test]
    fn public_keys_are_nonzero() {
        let device = test_device();
        assert_ne!(device.encryption_public_key(), &[0u8; 64]);
        assert_ne!(device.signing_public_key(), &[0u8; 64]);
    }

    // ── Error cases ──────────────────────────────────────────────────

    #[test]
    fn bad_magic() {
        let err = Device::from_bytes(b"XYZ\x03").unwrap_err();
        assert!(matches!(err, CdmError::PrdBadMagic));
    }

    #[test]
    fn unsupported_version() {
        let err = Device::from_bytes(b"PRD\x01").unwrap_err();
        assert!(matches!(err, CdmError::PrdUnsupportedVersion(1)));
    }

    #[test]
    fn truncated_input() {
        let err = Device::from_bytes(b"PR").unwrap_err();
        assert!(matches!(err, CdmError::PrdTruncated));
    }

    #[test]
    fn empty_input() {
        let err = Device::from_bytes(b"").unwrap_err();
        assert!(matches!(err, CdmError::PrdTruncated));
    }

    #[test]
    fn v2_truncated_cert_len() {
        let err = Device::from_bytes(b"PRD\x02\x00\x00").unwrap_err();
        assert!(matches!(err, CdmError::PrdTruncated));
    }

    #[test]
    fn v3_truncated_keys() {
        let mut data = b"PRD\x03".to_vec();
        data.extend(&[0u8; 50]);
        let err = Device::from_bytes(&data).unwrap_err();
        assert!(matches!(err, CdmError::PrdTruncated));
    }

    // ── Device::new() tests ──────────────────────────────────────────

    /// Extract 96-byte keypair from EccKeyPair.
    fn keypair_to_array(kp: &EccKeyPair) -> [u8; 96] {
        let mut buf = [0u8; 96];
        buf[..32].copy_from_slice(&kp.private_key);
        buf[32..].copy_from_slice(&kp.public_key);
        buf
    }

    #[test]
    fn new_reconstructs_from_parsed_device() {
        let original = test_device();
        let device = Device::new(
            keypair_to_array(&original.encryption_key),
            keypair_to_array(&original.signing_key),
            original.group_key.as_ref().map(keypair_to_array),
            original.group_certificate.clone(),
        )
        .unwrap();

        assert_eq!(device.security_level, original.security_level);
        assert_eq!(
            device.encryption_key.private_key,
            original.encryption_key.private_key
        );
        assert_eq!(
            device.encryption_key.public_key,
            original.encryption_key.public_key
        );
        assert_eq!(
            device.signing_key.private_key,
            original.signing_key.private_key
        );
        assert_eq!(
            device.signing_key.public_key,
            original.signing_key.public_key
        );
        assert!(device.has_group_key());
        // Serialization matches
        assert_eq!(device.to_bytes(), original.to_bytes());
    }

    #[test]
    fn new_without_group_key() {
        let original = test_device();
        let device = Device::new(
            keypair_to_array(&original.encryption_key),
            keypair_to_array(&original.signing_key),
            None,
            original.group_certificate.clone(),
        )
        .unwrap();

        assert!(!device.has_group_key());
        assert_eq!(device.security_level, original.security_level);
    }

    #[test]
    fn new_bad_certificate_fails() {
        let original = test_device();
        let err = Device::new(
            keypair_to_array(&original.encryption_key),
            keypair_to_array(&original.signing_key),
            None,
            b"not a valid cert".to_vec(),
        )
        .unwrap_err();
        assert!(matches!(err, CdmError::Format(_)));
    }

    // ── Device::from_private_keys() tests ────────────────────────────

    #[test]
    fn from_private_keys_derives_correct_public_keys() {
        let original = test_device();
        let device = Device::from_private_keys(
            original.encryption_key.private_key,
            original.signing_key.private_key,
            original.group_key.as_ref().map(|k| k.private_key),
            original.group_certificate.clone(),
        )
        .unwrap();

        assert_eq!(device.security_level, original.security_level);
        // Public keys must match those derived from the private keys in the original
        assert_eq!(
            device.encryption_key.public_key,
            original.encryption_key.public_key
        );
        assert_eq!(
            device.signing_key.public_key,
            original.signing_key.public_key
        );
        if let Some(ref gk) = device.group_key {
            assert_eq!(
                gk.public_key,
                original.group_key.as_ref().unwrap().public_key
            );
        }
        // Serialization matches since public keys are correct
        assert_eq!(device.to_bytes(), original.to_bytes());
    }

    #[test]
    fn from_private_keys_without_group_key() {
        let original = Device::from_bytes(TEST_PRD_V2).unwrap();
        let device = Device::from_private_keys(
            original.encryption_key.private_key,
            original.signing_key.private_key,
            None,
            original.group_certificate.clone(),
        )
        .unwrap();

        assert!(!device.has_group_key());
        assert_eq!(device.security_level, original.security_level);
        assert_eq!(
            device.encryption_key.public_key,
            original.encryption_key.public_key
        );
    }

    #[test]
    fn from_private_keys_zero_scalar_fails() {
        let original = test_device();
        let err = Device::from_private_keys(
            [0u8; 32], // zero is not a valid P-256 scalar
            original.signing_key.private_key,
            None,
            original.group_certificate.clone(),
        )
        .unwrap_err();
        assert!(matches!(err, CdmError::EccKeyParse(_)));
    }

    #[test]
    fn from_private_keys_bad_certificate_fails() {
        let original = test_device();
        let err = Device::from_private_keys(
            original.encryption_key.private_key,
            original.signing_key.private_key,
            None,
            vec![0xFF; 10],
        )
        .unwrap_err();
        assert!(matches!(err, CdmError::Format(_)));
    }
}
