use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ::rsa::BigUint;
use ::rsa::pkcs1::EncodeRsaPublicKey;
use prost::Message;
use rand::Rng;

use wdv3_proto::signed_message::MessageType;
use wdv3_proto::{DrmCertificate, License, LicenseRequest, SignedDrmCertificate, SignedMessage};

use crate::constants::{
    LICENSE_PRODUCTION_E, LICENSE_PRODUCTION_N, LICENSE_PRODUCTION_PROVIDER_ID,
    LICENSE_PRODUCTION_SERIAL, LICENSE_STAGING_E, LICENSE_STAGING_N, LICENSE_STAGING_PROVIDER_ID,
    LICENSE_STAGING_SERIAL, ROOT_PUBLIC_KEY_E, ROOT_PUBLIC_KEY_N,
};
use crate::crypto::{aes, hmac, padding, privacy, rsa};
use crate::device::Device;
use crate::error::{CdmError, CdmResult};
use crate::pssh::PsshBox;
use crate::types::{ContentKey, DeviceType, KeyType, LicenseType};

/// Global session counter for monotonically-increasing session numbers.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A Widevine CDM session that builds license challenges and parses license responses.
///
/// Typical usage:
/// ```ignore
/// let device = WvdDevice::from_bytes(&wvd_bytes)?;
/// let mut session = Session::new(device);
///
/// // Optional: enable privacy mode with a service certificate
/// session.set_service_certificate(&cert_bytes)?;
///
/// // Build the challenge bytes to POST to a license server
/// let challenge = session.build_license_challenge(&pssh, LicenseType::Streaming)?;
///
/// // ... user sends challenge via HTTP, gets response bytes ...
///
/// // Parse the response and extract content keys
/// let keys = session.parse_license_response(&response_bytes)?;
/// ```
pub struct Session {
    /// Monotonically-increasing session number (for display/logging).
    number: u64,
    /// Parsed WVD device credentials.
    device: Device,
    /// Verified service certificate for privacy mode. None = no privacy.
    service_certificate: Option<SignedDrmCertificate>,
    /// Map from request_id -> (enc_context, mac_context).
    /// Built during build_license_challenge(), consumed during parse_license_response().
    contexts: HashMap<Vec<u8>, (Vec<u8>, Vec<u8>)>,
    /// Extracted content keys after a successful parse_license_response().
    content_keys: Vec<ContentKey>,
}

impl Session {
    /// Create a new session for the given device.
    pub fn new(device: Device) -> Self {
        Session {
            number: SESSION_COUNTER.fetch_add(1, Ordering::Relaxed),
            device,
            service_certificate: None,
            contexts: HashMap::new(),
            content_keys: Vec::new(),
        }
    }

    /// Session number (monotonically increasing across all sessions in the process).
    pub fn number(&self) -> u64 {
        self.number
    }

    /// Set (and verify) a service certificate for privacy mode.
    ///
    /// The certificate is verified against the Widevine root public key using
    /// RSA-PSS-SHA1 signature verification. Once set, subsequent calls to
    /// `build_license_challenge` will encrypt the ClientIdentification.
    pub fn set_service_certificate(&mut self, raw: &[u8]) -> CdmResult<()> {
        let root_der = build_root_public_key_der()?;
        let signed_cert = privacy::verify_service_certificate(raw, &root_der)?;
        self.service_certificate = Some(signed_cert);
        Ok(())
    }

    /// Use the hardcoded privacy certificate for Google's production license server
    /// (license.widevine.com).
    ///
    /// Skips signature verification since the certificate data is compiled-in.
    pub fn set_service_certificate_common(&mut self) -> CdmResult<()> {
        self.service_certificate = Some(build_hardcoded_service_certificate(
            LICENSE_PRODUCTION_PROVIDER_ID,
            &LICENSE_PRODUCTION_SERIAL,
            &LICENSE_PRODUCTION_N,
            &LICENSE_PRODUCTION_E,
        )?);
        Ok(())
    }

    /// Use the hardcoded privacy certificate for Google's staging license server
    /// (staging.google.com).
    ///
    /// Skips signature verification since the certificate data is compiled-in.
    pub fn set_service_certificate_staging(&mut self) -> CdmResult<()> {
        self.service_certificate = Some(build_hardcoded_service_certificate(
            LICENSE_STAGING_PROVIDER_ID,
            &LICENSE_STAGING_SERIAL,
            &LICENSE_STAGING_N,
            &LICENSE_STAGING_E,
        )?);
        Ok(())
    }

    /// Build a license challenge (serialized SignedMessage) for the given PSSH box.
    ///
    /// Returns the raw bytes that should be POSTed to a license server.
    /// The derivation contexts are stored internally for use by `parse_license_response`.
    pub fn build_license_challenge(
        &mut self,
        pssh: &PsshBox,
        license_type: LicenseType,
    ) -> CdmResult<Vec<u8>> {
        let request_id = generate_request_id(self.device.device_type, self.number);

        // Build ContentIdentification with WidevinePsshData
        use wdv3_proto::license_request::ContentIdentification;
        use wdv3_proto::license_request::RequestType;
        use wdv3_proto::license_request::content_identification::ContentIdVariant;
        use wdv3_proto::license_request::content_identification::WidevinePsshData as PsshContentId;

        let proto_license_type: wdv3_proto::LicenseType = license_type.into();

        let content_id = ContentIdentification {
            content_id_variant: Some(ContentIdVariant::WidevinePsshData(PsshContentId {
                pssh_data: vec![pssh.init_data().to_vec()],
                license_type: Some(proto_license_type as i32),
                request_id: Some(request_id.clone()),
            })),
        };

        // Build LicenseRequest — privacy mode determines client_id vs encrypted_client_id
        let (client_id, encrypted_client_id) =
            if let Some(ref signed_cert) = self.service_certificate {
                let drm_cert_bytes = signed_cert.drm_certificate.as_deref().ok_or_else(|| {
                    CdmError::CertificateDecode("missing drm_certificate in service cert".into())
                })?;
                let drm_cert = DrmCertificate::decode(drm_cert_bytes)
                    .map_err(|e: prost::DecodeError| CdmError::ProtobufDecode(e.to_string()))?;
                let client_id_bytes = self.device.client_id.encode_to_vec();
                let encrypted = privacy::encrypt_client_id(&client_id_bytes, &drm_cert)?;
                (None, Some(encrypted))
            } else {
                (Some(self.device.client_id.clone()), None)
            };

        let request_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Range [1, 2^31) — upper bound ensures the value fits in a signed int32
        // (Java/JNI compatibility in the Android CDM). Lower bound avoids protobuf default 0.
        let key_control_nonce: u32 = rand::thread_rng().gen_range(1..2_147_483_648);

        let license_request = LicenseRequest {
            client_id,
            content_id: Some(content_id),
            r#type: Some(RequestType::New as i32),
            request_time: Some(request_time),
            key_control_nonce_deprecated: None,
            protocol_version: Some(21), // VERSION_2_1
            key_control_nonce: Some(key_control_nonce),
            encrypted_client_id,
        };

        let license_request_bytes = license_request.encode_to_vec();

        // Store derivation contexts keyed by request_id
        let enc_ctx = aes::build_enc_context(&license_request_bytes);
        let mac_ctx = aes::build_mac_context(&license_request_bytes);
        self.contexts.insert(request_id, (enc_ctx, mac_ctx));

        // Sign the serialized LicenseRequest with RSA-PSS-SHA1
        let signature = rsa::rsa_pss_sha1_sign(&self.device.private_key, &license_request_bytes)?;

        // Wrap in SignedMessage
        let signed_message = SignedMessage {
            r#type: Some(MessageType::LicenseRequest as i32),
            msg: Some(license_request_bytes),
            signature: Some(signature),
            ..Default::default()
        };

        Ok(signed_message.encode_to_vec())
    }

    /// Parse a license response and extract content keys.
    ///
    /// Takes the raw bytes received from the license server. Returns the
    /// extracted content keys on success.
    pub fn parse_license_response(&mut self, raw: &[u8]) -> CdmResult<&[ContentKey]> {
        // Step 1: Decode the SignedMessage wrapper
        let signed_message = SignedMessage::decode(raw)
            .map_err(|e: prost::DecodeError| CdmError::ProtobufDecode(e.to_string()))?;

        // Verify this is a LICENSE message, not something else
        let msg_type = signed_message.r#type.unwrap_or(0);
        if msg_type != MessageType::License as i32 {
            return Err(CdmError::ProtobufDecode(format!(
                "expected LICENSE message (type {}), got type {msg_type}",
                MessageType::License as i32,
            )));
        }

        let msg = signed_message
            .msg
            .as_deref()
            .ok_or_else(|| CdmError::ProtobufDecode("missing msg in SignedMessage".into()))?;
        let signature = signed_message
            .signature
            .as_deref()
            .ok_or_else(|| CdmError::ProtobufDecode("missing signature in SignedMessage".into()))?;
        let session_key_enc = signed_message.session_key.as_deref().ok_or_else(|| {
            CdmError::ProtobufDecode("missing session_key in SignedMessage".into())
        })?;

        // Step 2: Decode the License from msg
        let license = License::decode(msg)
            .map_err(|e: prost::DecodeError| CdmError::ProtobufDecode(e.to_string()))?;

        // Step 3: Extract request_id from the license identification
        let license_id = license
            .id
            .as_ref()
            .ok_or_else(|| CdmError::ProtobufDecode("missing id in License".into()))?;
        let request_id = license_id.request_id.as_deref().ok_or_else(|| {
            CdmError::ProtobufDecode("missing request_id in LicenseIdentification".into())
        })?;

        // Step 4: Look up stored derivation contexts
        let (enc_context, mac_context) = self
            .contexts
            .remove(request_id)
            .ok_or(CdmError::ContextNotFound)?;

        // Step 5: Decrypt the session key with RSA-OAEP-SHA1
        let session_key_vec =
            rsa::rsa_oaep_sha1_decrypt(&self.device.private_key, session_key_enc)?;
        let session_key: [u8; 16] = session_key_vec.try_into().map_err(|v: Vec<u8>| {
            CdmError::RsaOperation(format!("session key is {} bytes, expected 16", v.len()))
        })?;

        // Step 6: Derive encryption and MAC keys
        let derived = aes::derive_keys(&enc_context, &mac_context, &session_key)?;

        // Step 7: Verify the license response HMAC signature
        hmac::verify_license_signature(
            &derived.mac_key_server,
            signed_message.oemcrypto_core_message.as_deref(),
            msg,
            signature,
        )?;

        // Step 8: Extract and decrypt content keys from each KeyContainer
        let mut keys = Vec::new();
        for container in &license.key {
            let iv = match container.iv.as_deref() {
                Some(iv) => iv,
                None => continue,
            };
            let encrypted_key = match container.key.as_deref() {
                Some(k) => k,
                None => continue,
            };

            // Decrypt and unpad the content key
            let decrypted = aes::aes_cbc_decrypt_key(&derived.enc_key, iv, encrypted_key)?;
            let key_bytes = padding::pkcs7_unpad(&decrypted, 16)?;

            // Map the proto key type to our KeyType; skip unrecognized (value 0)
            let proto_type = container.r#type.unwrap_or(0);
            let key_type = match wdv3_proto::license::key_container::KeyType::try_from(proto_type) {
                Ok(kt) => KeyType::from(kt),
                Err(_) => continue,
            };

            // Normalize the key ID to 16 bytes
            let kid_raw = container.id.as_deref().unwrap_or_default();
            let kid = kid_to_uuid(kid_raw);

            keys.push(ContentKey {
                kid,
                key: key_bytes,
                key_type,
            });
        }

        if keys.is_empty() {
            return Err(CdmError::NoContentKeys);
        }

        self.content_keys = keys;
        Ok(&self.content_keys)
    }

    /// Returns the extracted content keys (empty until `parse_license_response` succeeds).
    pub fn keys(&self) -> &[ContentKey] {
        &self.content_keys
    }

    /// Returns content keys filtered to `KeyType::Content`, formatted as `(hex_kid, hex_key)` pairs.
    pub fn content_keys_as_hex(&self) -> Vec<(String, String)> {
        self.content_keys
            .iter()
            .filter(|k| k.key_type == KeyType::Content)
            .map(|k| (hex::encode(k.kid), hex::encode(&k.key)))
            .collect()
    }
}

/// Build the Widevine root RSA public key in PKCS#1 DER format from the raw N/E constants.
fn build_root_public_key_der() -> CdmResult<Vec<u8>> {
    let n = BigUint::from_bytes_be(&ROOT_PUBLIC_KEY_N);
    let e = BigUint::from_bytes_be(&ROOT_PUBLIC_KEY_E);
    let pubkey =
        ::rsa::RsaPublicKey::new(n, e).map_err(|e| CdmError::RsaKeyParse(format!("{e}")))?;
    let der = pubkey
        .to_pkcs1_der()
        .map_err(|e| CdmError::RsaKeyParse(format!("{e}")))?;
    Ok(der.as_bytes().to_vec())
}

/// Build a `SignedDrmCertificate` from hardcoded provider constants.
///
/// This constructs a DrmCertificate protobuf containing the provider_id,
/// serial_number, and public_key, then wraps it in a SignedDrmCertificate
/// (with an empty signature, since we trust the compiled-in data).
fn build_hardcoded_service_certificate(
    provider_id: &str,
    serial_number: &[u8],
    n: &[u8],
    e: &[u8],
) -> CdmResult<SignedDrmCertificate> {
    // Build the RSA public key DER from N/E
    let pubkey = ::rsa::RsaPublicKey::new(BigUint::from_bytes_be(n), BigUint::from_bytes_be(e))
        .map_err(|e| CdmError::RsaKeyParse(format!("{e}")))?;
    let pub_der = pubkey
        .to_pkcs1_der()
        .map_err(|e| CdmError::RsaKeyParse(format!("{e}")))?;

    // Build DrmCertificate protobuf
    let drm_cert = DrmCertificate {
        provider_id: Some(provider_id.to_owned()),
        serial_number: Some(serial_number.to_vec()),
        public_key: Some(pub_der.as_bytes().to_vec()),
        ..Default::default()
    };
    let drm_cert_bytes = drm_cert.encode_to_vec();

    Ok(SignedDrmCertificate {
        drm_certificate: Some(drm_cert_bytes),
        // No signature — we trust the hardcoded data
        ..Default::default()
    })
}

/// Generate a random request_id.
///
/// - Android devices: mimics OEMCrypto CTR counter block format —
///   4 random bytes + 4 zero bytes + 8-byte little-endian session number.
/// - Chrome devices: 16 raw random bytes.
fn generate_request_id(device_type: DeviceType, session_number: u64) -> Vec<u8> {
    let mut rng = rand::thread_rng();
    match device_type {
        DeviceType::Android => {
            let mut id = vec![0u8; 16];
            rand::RngCore::fill_bytes(&mut rng, &mut id[..4]);
            // bytes 4..8 stay zero
            id[8..16].copy_from_slice(&session_number.to_le_bytes());
            id
        }
        DeviceType::Chrome => {
            let mut id = vec![0u8; 16];
            rand::RngCore::fill_bytes(&mut rng, &mut id);
            id
        }
    }
}

/// Normalize a key ID to exactly 16 bytes (UUID size).
///
/// 1. If the bytes are valid UTF-8 and parse as a decimal integer, convert
///    that integer to 16 big-endian bytes.
/// 2. Otherwise, pad with trailing zeros or truncate to 16 bytes.
fn kid_to_uuid(kid: &[u8]) -> [u8; 16] {
    // Try decimal string parse
    if let Ok(s) = std::str::from_utf8(kid)
        && let Ok(n) = s.parse::<u128>()
    {
        return n.to_be_bytes();
    }

    // Raw bytes: pad or truncate to 16
    let mut uuid = [0u8; 16];
    let len = kid.len().min(16);
    uuid[..len].copy_from_slice(&kid[..len]);
    uuid
}
