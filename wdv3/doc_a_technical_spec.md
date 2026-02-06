# TECHNICAL SPECIFICATION

## Widevine L3 CDM — Rust Reimplementation Architecture

### Type Definitions, Module Layout, and Protocol Documentation

---

**Document Classification: CONFIDENTIAL**

Purpose: Provide a complete structural blueprint for a Rust reimplementation of the Widevine L3 license acquisition protocol. Every type, function signature, and data flow is documented with its corresponding protocol phase. This document does not contain runnable code.

---

## 1. Module Architecture

The reimplementation is organized into modules that mirror the protocol phases.

```
widevine_l3/
├── main.rs                 // Entry point: orchestrates full license acquisition
├── wvd.rs                  // Phase 2: WVD file parsing and credential loading
├── protobuf/
│   ├── mod.rs              // Protobuf wire format primitives (varint, tags, length-delimited)
│   ├── serialize.rs        // Manual serialization of LicenseRequest, SignedMessage
│   ├── deserialize.rs      // Manual deserialization of License, KeyContainer, SignedMessage
│   └── messages.rs         // Struct definitions for all protobuf messages
├── pssh.rs                 // Phase 1: PSSH box parsing and WidevinePsshData extraction
├── crypto/
│   ├── mod.rs              // Crypto module root
│   ├── rsa.rs              // RSA-PSS-SHA1 signing, RSA-OAEP-SHA1 encrypt/decrypt
│   ├── aes.rs              // AES-CBC encrypt/decrypt, AES-CMAC (key derivation)
│   ├── hmac.rs             // HMAC-SHA256 (license response verification)
│   ├── padding.rs          // PKCS#7 padding (removal for key decryption, addition for privacy mode)
│   └── privacy.rs          // Privacy mode: client ID encryption, service certificate verification
├── license.rs              // Phases 3-5: Request construction, response parsing, key extraction
└── types.rs                // Shared types: ContentKey, DeviceType, SecurityLevel, DerivedKeys, errors
```

---

## 2. Shared Types (`types.rs`)

These types are used across module boundaries. Each encodes a protocol-specific concept.

```rust
/// Device type as encoded in WVD file byte offset 4.
/// Values: Chrome=1, Android=2. These are defined by the WVD file format specification,
/// not by Google's license_protocol.proto (the closest proto enum,
/// ClientIdentification.TokenType, has unrelated values).
#[repr(u8)]
pub enum DeviceType {
    Chrome = 1,
    Android = 2,
}

/// Widevine security level.
/// Ref: license_protocol.proto.
#[repr(u8)]
pub enum SecurityLevel {
    L1 = 1,
    L2 = 2,
    L3 = 3,
}

/// A content decryption key extracted from a license response.
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
    /// All types are decrypted and stored; format_keys() filters to CONTENT (2) for output.
    pub key_type: KeyType,
}

/// Key type enumeration from License.KeyContainer.KeyType.
/// Ref: license_protocol.proto, License.KeyContainer.KeyType enum.
///
/// Note: Protobuf default value 0 has no named variant in the proto definition.
/// If a KeyContainer has key_type == 0, it should be treated as an unknown type
/// and processed (decrypted, stored) but not included in the CONTENT key output.
#[repr(u32)]
pub enum KeyType {
    Signing = 1,
    Content = 2,
    KeyControl = 3,
    OperatorSession = 4,
    Entitlement = 5,
    OemContent = 6,
}

/// The three derived keys from a session key.
pub struct DerivedKeys {
    /// 16 bytes. AES-CMAC(session_key, 0x01 || enc_context).
    /// Used to decrypt KeyContainer.key fields.
    pub enc_key: [u8; 16],
    /// 32 bytes. CMAC(session_key, 0x01 || mac_context) || CMAC(session_key, 0x02 || mac_context).
    /// Used to verify license response signature via HMAC-SHA256.
    pub mac_key_server: [u8; 32],
    /// 32 bytes. CMAC(session_key, 0x03 || mac_context) || CMAC(session_key, 0x04 || mac_context).
    /// Used for license renewal requests.
    pub mac_key_client: [u8; 32],
}

/// Errors specific to the protocol exchange.
pub enum CdmError {
    /// WVD file has wrong magic bytes (expected [0x57, 0x56, 0x44]).
    InvalidWvdMagic,
    /// WVD version is not 2.
    UnsupportedWvdVersion(u8),
    /// PSSH box not found or system ID mismatch.
    PsshNotFound,
    /// Widevine system ID in PSSH does not match EDEF8BA9-79D6-4ACE-A3C8-27DCD51D21ED.
    SystemIdMismatch,
    /// Protobuf deserialization failed.
    ProtobufDecode(String),
    /// RSA operation failed.
    RsaError(String),
    /// HMAC-SHA256 signature mismatch on license response.
    SignatureMismatch,
    /// No CONTENT-type keys found in license.
    NoContentKeys,
    /// PKCS#7 padding is invalid after AES-CBC decryption.
    InvalidPadding,
    /// HTTP request to license server failed.
    LicenseServerError(u16),
    /// Service certificate signature verification failed (RSA-PSS-SHA1 vs root cert).
    CertificateSignatureMismatch,
    /// Could not parse certificate as SignedDrmCertificate or SignedMessage.
    CertificateDecodeError(String),
    /// License response request_id does not match any stored session context.
    /// Indicates a mismatched or replayed response.
    ContextNotFound,
}
```

---

## 3. WVD File Parsing (`wvd.rs`)

Loads device credentials from the WVD binary format.

```rust
/// Represents a parsed WVD (Widevine Device) file.
pub struct WvdDevice {
    /// DeviceType: Chrome (1) or Android (2). Offset 4.
    pub device_type: DeviceType,
    /// Security level. Offset 5.
    pub security_level: SecurityLevel,
    /// DER-encoded RSA private key (PKCS#1 RSAPrivateKey).
    /// Starts at offset 9, length from uint16be at offset 7.
    pub private_key_der: Vec<u8>,
    /// Raw serialized ClientIdentification protobuf (license_protocol.proto, lines 435-526).
    /// Starts at offset 11 + private_key_len, length from uint16be at offset 9 + private_key_len.
    /// Sent as-is (or encrypted via privacy mode) in the license request.
    /// Treated as opaque bytes — internal fields are not parsed.
    /// The ClientIdentification struct definition (containing device token, DRM certificates,
    /// and client capabilities) is not required for reimplementation; only the serialized
    /// bytes are needed for the license request.
    pub client_id_blob: Vec<u8>,
}

/// Parse a WVD v2 file from raw bytes.
///
/// Binary layout:
///   [0..3]    magic: "WVD" (0x57, 0x56, 0x44)
///   [3]       version: u8 (must be 2)
///   [4]       type: u8 (1=CHROME, 2=ANDROID)
///   [5]       security_level: u8
///   [6]       flags: u8 (reserved, currently 0x00. Parse but discard.
///             Non-zero flags should not cause a parse error.)
///   [7..9]    private_key_len: u16 big-endian
///   [9..9+N]  private_key: N bytes of DER-encoded RSA private key
///   [9+N..11+N] client_id_len: u16 big-endian
///   [11+N..11+N+M] client_id: M bytes of serialized ClientIdentification protobuf
pub fn parse_wvd(data: &[u8]) -> Result<WvdDevice, CdmError>;
```

---

## 4. PSSH Box Parsing (`pssh.rs`)

Extracts Widevine-specific initialization data from ISOBMFF PSSH boxes.

```rust
/// The Widevine system ID as raw bytes (big-endian per RFC 4122).
/// UUID: EDEF8BA9-79D6-4ACE-A3C8-27DCD51D21ED
pub const WIDEVINE_SYSTEM_ID: [u8; 16] = [
    0xED, 0xEF, 0x8B, 0xA9, 0x79, 0xD6, 0x4A, 0xCE,
    0xA3, 0xC8, 0x27, 0xDC, 0xD5, 0x1D, 0x21, 0xED,
];

/// Parsed PSSH box contents relevant to license acquisition.
pub struct PsshData {
    /// Raw init_data bytes (the PSSH box's Data field).
    /// This is a serialized WidevinePsshData protobuf.
    /// Passed directly into LicenseRequest.ContentIdentification.WidevinePsshData.pssh_data.
    pub init_data: Vec<u8>,
    /// Key IDs extracted from the WidevinePsshData.key_ids field (proto field 2, repeated).
    /// Each is 16 bytes. Used for final KID:KEY output correlation.
    pub key_ids: Vec<[u8; 16]>,
}

/// Parse a PSSH box from raw bytes (full ISOBMFF box starting with size field).
///
/// ISOBMFF PSSH box layout:
///   [0..4]    box_size: u32 big-endian (total box size including header)
///   [4..8]    box_type: "pssh" = 0x70737368
///   [8]       version: u8 (0 or 1)
///   [9..12]   flags: u24 (typically 0)
///   [12..28]  system_id: 16 bytes (must match WIDEVINE_SYSTEM_ID)
///   if version == 1:
///     [28..32]  key_id_count: u32 big-endian
///     [32..]    key_ids: key_id_count × 16 bytes
///   [..]      data_size: u32 big-endian (ISOBMFF convention: all multi-byte integers are big-endian)
///   [..]      data: data_size bytes (WidevinePsshData protobuf)
///
/// Key ID extraction by version:
///   v0: No key_id_count in box header. data_size is at offset 28.
///       Key IDs must be extracted from the WidevinePsshData protobuf's
///       key_ids field (proto field 2, repeated bytes, each 16 bytes).
///   v1: key_id_count at offset 28, key_ids at offset 32 (each 16 bytes).
///       data_size at offset 32 + (key_id_count × 16).
///       Key IDs are available from both the box header AND the protobuf.
///       Prefer box-header KIDs (they're already parsed as raw bytes).
pub fn parse_pssh_box(data: &[u8]) -> Result<PsshData, CdmError>;

/// Extract PSSH box(es) from a DASH MPD XML manifest.
///
/// Looks for <ContentProtection> elements with schemeIdUri="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed"
/// and extracts the child <cenc:pssh> element's base64-encoded content.
///
/// Namespace URIs:
///   MPD: urn:mpeg:dash:schema:mpd:2011
///   CENC: urn:mpeg:cenc:2013
pub fn extract_pssh_from_mpd(mpd_xml: &str) -> Result<Vec<PsshData>, CdmError>;
```

---

## 5. Protobuf Wire Format (`protobuf/`)

### 5.1 Primitives (`protobuf/mod.rs`)

```rust
/// Protobuf wire types.
#[repr(u8)]
pub enum WireType {
    Varint = 0,            // int32, int64, uint32, uint64, sint32, sint64, bool, enum
    Fixed64 = 1,           // fixed64, sfixed64, double
    LengthDelimited = 2,   // string, bytes, embedded messages, repeated fields
    Fixed32 = 5,           // fixed32, sfixed32, float
    // Wire types 3 (StartGroup) and 4 (EndGroup) are deprecated in proto3 but
    // valid in proto2. license_protocol.proto uses proto2 syntax. Widevine's
    // messages do not use groups, but a forward-compatible deserializer should
    // handle them: read fields until a matching EndGroup (wire type 4) with the
    // same field number is encountered. If encountered unexpectedly (without a
    // matching start/end), treat as CdmError::ProtobufDecode.
}

/// Skip an unknown field in a protobuf message based on its wire type.
/// Required for forward-compatible deserialization.
///
/// Skip strategy per wire type:
///   Varint (0):      read bytes until a byte with MSB=0 (the terminator)
///   Fixed64 (1):     skip exactly 8 bytes
///   LengthDelimited (2): read varint length N, then skip N bytes
///   Fixed32 (5):     skip exactly 4 bytes
fn skip_field(wire_type: u8, data: &[u8], offset: usize) -> Result<usize, CdmError>;

/// Encode a u64 as a protobuf varint (LEB128).
/// Returns the number of bytes written.
///
/// Algorithm:
///   while value > 0x7F:
///     emit (value & 0x7F) | 0x80
///     value >>= 7
///   emit value
pub fn encode_varint(value: u64, buf: &mut Vec<u8>) -> usize;

/// Decode a protobuf varint from a byte slice.
/// Returns (decoded_value, bytes_consumed).
pub fn decode_varint(data: &[u8]) -> Result<(u64, usize), CdmError>;

/// Encode a protobuf field tag.
/// tag_byte = (field_number << 3) | wire_type
pub fn encode_tag(field_number: u32, wire_type: WireType, buf: &mut Vec<u8>);

/// Encode a length-delimited field: tag + varint(length) + payload.
pub fn encode_length_delimited(field_number: u32, payload: &[u8], buf: &mut Vec<u8>);

/// Encode a varint field: tag + varint(value).
pub fn encode_varint_field(field_number: u32, value: u64, buf: &mut Vec<u8>);
```

### 5.2 Message Structures (`protobuf/messages.rs`)

All proto field numbers are from `license_protocol.proto`. Field numbers are stable across versions.

```rust
/// SignedMessage — the outermost protocol envelope.
/// Ref: license_protocol.proto, message SignedMessage.
///
/// Used for both license requests (sent to server) and license responses (received).
pub struct SignedMessage {
    /// Field 1, varint. MessageType enum.
    /// Request: LICENSE_REQUEST (1). Response: LICENSE (2).
    pub msg_type: u32,
    /// Field 2, length-delimited. Serialized inner message (LicenseRequest or License).
    pub msg: Vec<u8>,
    /// Field 3, length-delimited.
    /// Request: RSA-PSS-SHA1 signature over msg bytes.
    /// Response: HMAC-SHA256(mac_key_server, oemcrypto_core_message || msg).
    pub signature: Vec<u8>,
    /// Field 4, length-delimited. Present in responses only.
    /// RSA-OAEP-SHA1 encrypted session key (encrypted to the device's public key).
    pub session_key: Option<Vec<u8>>,
    // Fields 5-8 exist in the proto but are NOT used:
    //   Field 5: remote_attestation (bytes)
    //   Field 6: metric_data (MetricData repeated)
    //   Field 7: service_version_info (VersionInfo)
    //   Field 8: session_key_type (SessionKeyType enum)
    // A deserializer must skip these fields by wire type if encountered.
    /// Field 9, length-delimited. Present in some responses (OEMCrypto API v16+).
    /// If present, prepended to msg when computing HMAC for signature verification.
    pub oemcrypto_core_message: Option<Vec<u8>>,
}

/// LicenseRequest — the inner message of a license request SignedMessage.
/// Ref: license_protocol.proto, message LicenseRequest.
pub struct LicenseRequest {
    /// Field 1, length-delimited. Serialized ClientIdentification protobuf.
    /// Sent in cleartext if privacy mode is not used.
    /// Mutually exclusive with encrypted_client_id.
    pub client_id: Option<Vec<u8>>,
    /// Field 2, length-delimited. ContentIdentification wrapper.
    /// Contains a oneof with WidevinePsshData (most common).
    pub content_id: ContentIdentification,
    /// Field 3, varint. RequestType: NEW=1, RENEWAL=2, RELEASE=3.
    pub request_type: u32,
    /// Field 4, varint. POSIX timestamp (seconds since epoch, UTC).
    pub request_time: i64,
    /// Field 5, length-delimited. DEPRECATED old-style decimal-encoded string key control nonce.
    /// Superseded by field 7. Not populated.
    /// Included for completeness — a deserializer must skip this field if encountered
    /// in messages from older CDM implementations.
    pub key_control_nonce_deprecated: Option<Vec<u8>>,
    /// Field 6, varint. ProtocolVersion: VERSION_2_1 = 21.
    pub protocol_version: u32,
    /// Field 7, varint. Random uint32 nonce for key control.
    /// Generated in range [1, 2^31). The upper bound of 2^31 (not 2^32)
    /// ensures the value fits in a signed int32 (Java/JNI compatibility
    /// in the Android CDM). The lower bound of 1 avoids the protobuf default value 0.
    /// The active replacement for deprecated field 5.
    pub key_control_nonce: u32,
    /// Field 8, length-delimited. EncryptedClientIdentification protobuf.
    /// Used when privacy mode is active. Mutually exclusive with client_id.
    pub encrypted_client_id: Option<Vec<u8>>,
}

/// ContentIdentification wrapping the PSSH data.
/// Ref: license_protocol.proto, LicenseRequest.ContentIdentification.
///
/// The proto defines a oneof with four variants:
///   1. WidevinePsshData  — the only variant constructed in this implementation
///   2. WebmKeyId         — NOT USED
///   3. ExistingLicense    — NOT USED
///   4. InitData           — NOT USED
/// This spec documents only the WidevinePsshData path.
pub struct ContentIdentification {
    /// Oneof variant 1 (field number 1, encoded tag byte 0x0A). WidevinePsshData.
    pub widevine_pssh_data: WidevinePsshDataWrapper,
}

/// The WidevinePsshData message INSIDE ContentIdentification (not the standalone one).
/// Ref: license_protocol.proto, LicenseRequest.ContentIdentification.WidevinePsshData.
///
/// CRITICAL: Do not confuse with the standalone WidevinePsshData message
/// which is found inside PSSH boxes. This is the wrapper that carries the PSSH data
/// as an opaque blob within the license request.
pub struct WidevinePsshDataWrapper {
    /// Field 1, length-delimited, REPEATED.
    /// Each entry is a raw PSSH data payload.
    /// Typically contains a single entry: the init_data extracted from the PSSH box.
    pub pssh_data: Vec<Vec<u8>>,
    /// Field 2, varint. LicenseType: STREAMING=1, OFFLINE=2, AUTOMATIC=3.
    pub license_type: u32,
    /// Field 3, length-delimited. Opaque request identifier.
    /// For Android devices: 32-byte uppercase hex-encoded string derived from
    ///   random(4) || 0x00000000 || session_number(u32 zero-extended to 8 bytes LE),
    ///   then hex().upper().encode().
    /// For Chrome devices: 16 random bytes.
    pub request_id: Vec<u8>,
}

/// A key container from a License response.
/// Ref: license_protocol.proto, License.KeyContainer (12 top-level fields).
///
/// Fields 1, 2, 3, 4, and 9 are used. Fields 5-8 and 10-12 are defined in the
/// proto but not used. A manual deserializer must still handle or skip all fields
/// by wire type to avoid misinterpreting subsequent data.
pub struct KeyContainer {
    /// Field 1, length-delimited. Key ID (typically 16 bytes).
    /// USED: content key identifier (KID).
    pub id: Vec<u8>,
    /// Field 2, length-delimited. IV for AES-CBC decryption of the key field.
    /// USED: passed to AES-CBC decrypt as iv parameter.
    pub iv: Vec<u8>,
    /// Field 3, length-delimited. Encrypted key bytes.
    /// USED: decrypted via AES-128-CBC(enc_key, iv, ciphertext).
    pub key: Vec<u8>,
    /// Field 4, varint. KeyType enum value.
    /// USED: determines key type and checks for OPERATOR_SESSION type (triggers field 9 access).
    pub key_type: u32,
    /// Field 5, varint. SecurityLevel enum value (SW_SECURE_CRYPTO=1 through HW_SECURE_ALL=5).
    /// NOT USED. Parse and skip.
    pub level: Option<u32>,
    /// Field 6, length-delimited. OutputProtection message (HDCP/CGMS requirements).
    /// NOT USED. Parse and skip.
    pub required_protection: Option<Vec<u8>>,
    /// Field 7, length-delimited. OutputProtection message (requested, not required).
    /// NOT USED. Parse and skip.
    pub requested_protection: Option<Vec<u8>>,
    /// Field 8, length-delimited. KeyControl message (key_control_block + iv).
    /// NOT USED. Parse and skip.
    pub key_control: Option<Vec<u8>>,
    /// Field 9, length-delimited. OperatorSessionKeyPermissions message.
    /// USED only when key_type == OPERATOR_SESSION (4).
    /// Contains boolean permission flags: allow_encrypt (1), allow_decrypt (2),
    /// allow_sign (3), allow_signature_verify (4).
    pub operator_session_key_permissions: Option<Vec<u8>>,
    /// Field 10, length-delimited, REPEATED. VideoResolutionConstraint messages.
    /// NOT USED. Parse and skip.
    pub video_resolution_constraints: Vec<Vec<u8>>,
    /// Field 11, varint (bool). Anti-rollback usage table flag.
    /// NOT USED. Parse and skip.
    pub anti_rollback_usage_table: Option<bool>,
    /// Field 12, length-delimited (string). Provider-defined track label.
    /// NOT USED. Parse and skip.
    pub track_label: Option<String>,
}

/// LicenseIdentification — embedded in License.id.
/// Ref: license_protocol.proto, message LicenseIdentification (6 fields).
///
/// Only request_id (field 1) is required for key extraction. The remaining fields
/// are documented for completeness and to ensure a deserializer correctly
/// skips them rather than misinterpreting their wire data.
pub struct LicenseIdentification {
    /// Field 1, length-delimited. Must match the request_id from the original request.
    /// Used to look up the correct derivation context from Session.contexts.
    /// THIS IS THE ONLY FIELD REQUIRED FOR KEY EXTRACTION.
    pub request_id: Vec<u8>,
    /// Field 2, length-delimited. Session identifier assigned by the server.
    /// Not used for key extraction. Skipped during deserialization.
    pub session_id: Option<Vec<u8>>,
    /// Field 3, length-delimited. Purchase identifier for persistent licenses.
    /// Not used for key extraction.
    pub purchase_id: Option<Vec<u8>>,
    /// Field 4, varint. LicenseType enum (STREAMING=1, OFFLINE=2, AUTOMATIC=3).
    /// Echoed from the original request. Not used for key extraction.
    pub license_type: Option<u32>,
    /// Field 5, varint (int32). License version, incremented on renewal.
    /// Not used for key extraction.
    pub version: Option<i32>,
    /// Field 6, length-delimited. Provider-assigned session token.
    /// Not used for key extraction.
    pub provider_session_token: Option<Vec<u8>>,
}

/// Parsed License message from a license response.
/// Ref: license_protocol.proto, message License (11 top-level fields).
///
/// Only fields 1 (id) and 3 (keys) are required for key extraction.
/// All other fields are documented for completeness. A deserializer must
/// handle or skip all fields to avoid misinterpreting the wire format.
pub struct License {
    /// Field 1, length-delimited. LicenseIdentification — contains request_id
    /// used to look up derivation contexts from the session.
    /// REQUIRED FOR KEY EXTRACTION.
    pub id: LicenseIdentification,
    /// Field 2, length-delimited. Policy message containing playback enforcement:
    /// can_play, can_persist, can_renew, rental/playback/license duration seconds,
    /// renewal parameters, HDCP enforcement, etc. (15 sub-fields total).
    /// Not used for key extraction.
    pub policy: Option<Vec<u8>>,
    /// Field 3, length-delimited, REPEATED. KeyContainer entries.
    /// REQUIRED FOR KEY EXTRACTION. Each container holds one encrypted key.
    pub keys: Vec<KeyContainer>,
    /// Field 4, varint (int64). License start time in seconds UTC.
    /// Not used for key extraction.
    pub license_start_time: Option<i64>,
    /// Field 5, varint (bool). Whether remote attestation was verified.
    /// Not used for key extraction.
    pub remote_attestation_verified: Option<bool>,
    /// Field 6, length-delimited. Provider-defined client token.
    /// Not used for key extraction.
    pub provider_client_token: Option<Vec<u8>>,
    /// Field 7, varint (uint32). CENC protection scheme FourCC.
    /// Values are FourCC codes stored as u32 big-endian
    /// (first ASCII character in MSB): 'cenc' (0x63656E63), 'cbc1' (0x63626331),
    /// 'cens' (0x63656E73), 'cbcs' (0x63626373).
    /// In Rust: `u32::from_be_bytes(*b"cenc")` = 0x63656E63.
    /// Relevant to content decryption (determines AES mode for media segments)
    /// but not to key extraction itself.
    pub protection_scheme: Option<u32>,
    /// Field 8, length-delimited. SRM requirement. Not used for key extraction.
    pub srm_requirement: Option<Vec<u8>>,
    /// Field 9, length-delimited. Signed SRM update. Not used for key extraction.
    pub srm_update: Option<Vec<u8>>,
    /// Field 10, varint. PlatformVerificationStatus enum. Not used for key extraction.
    pub platform_verification_status: Option<u32>,
    /// Field 11, length-delimited, REPEATED. Group IDs. Not used for key extraction.
    pub group_ids: Vec<Vec<u8>>,
}

/// EncryptedClientIdentification — privacy-mode wrapper for ClientIdentification.
/// Ref: license_protocol.proto, message EncryptedClientIdentification (5 fields).
///
/// When privacy mode is active, the plaintext ClientIdentification is encrypted
/// with a random AES-128 key, and that key is RSA-OAEP encrypted to the service
/// certificate's public key.
///
/// Populated by encrypt_client_id() and placed in LicenseRequest.encrypted_client_id
/// (field 8) instead of the plaintext client_id (field 1).
pub struct EncryptedClientIdentification {
    /// Field 1, length-delimited (string). Provider ID from the service certificate
    /// (DrmCertificate.provider_id, field 7).
    pub provider_id: String,
    /// Field 2, length-delimited (bytes). Serial number of the service certificate
    /// (DrmCertificate.serial_number, field 2).
    pub service_certificate_serial_number: Vec<u8>,
    /// Field 3, length-delimited (bytes). AES-128-CBC ciphertext of the
    /// PKCS#7-padded serialized ClientIdentification protobuf.
    pub encrypted_client_id: Vec<u8>,
    /// Field 4, length-delimited (bytes). 16-byte IV used for the AES-128-CBC encryption.
    pub encrypted_client_id_iv: Vec<u8>,
    /// Field 5, length-delimited (bytes). RSA-OAEP-SHA1 ciphertext of the 16-byte
    /// AES privacy key, encrypted to the service certificate's RSA public key
    /// (DrmCertificate.public_key, field 4).
    pub encrypted_privacy_key: Vec<u8>,
}

/// SignedDrmCertificate — signed wrapper around a DrmCertificate.
/// Ref: license_protocol.proto, message SignedDrmCertificate (4 fields).
///
/// Used to deliver service privacy certificates. The signature is verified
/// against the Widevine root certificate's RSA public key using RSA-PSS-SHA1.
pub struct SignedDrmCertificate {
    /// Field 1, length-delimited (bytes). Serialized DrmCertificate protobuf.
    pub drm_certificate: Vec<u8>,
    /// Field 2, length-delimited (bytes). RSA-PSS-SHA1 signature over field 1.
    pub signature: Vec<u8>,
    /// Field 3, length-delimited. Signer certificate (recursive SignedDrmCertificate).
    /// Forms a certificate chain. Not used for service certificates.
    pub signer: Option<Vec<u8>>,
    /// Field 4, varint. HashAlgorithmProto enum identifying the hash used for signing.
    /// Not used (assumes SHA-1).
    pub hash_algorithm: Option<u32>,
}

/// DrmCertificate — the inner certificate payload.
/// Ref: license_protocol.proto, message DrmCertificate (12 fields).
///
/// Fields 2 (serial_number), 4 (public_key), and 7 (provider_id) are used
/// during privacy mode operations.
pub struct DrmCertificate {
    /// Field 1, varint. Certificate type: ROOT=0, DEVICE_MODEL=1, DEVICE=2,
    /// SERVICE=3, PROVISIONER=4. Service certificates have type SERVICE (3).
    /// NOT USED during privacy operations.
    pub cert_type: Option<u32>,
    /// Field 2, length-delimited (bytes). 128-bit globally unique serial number.
    /// USED: copied into EncryptedClientIdentification.service_certificate_serial_number.
    pub serial_number: Vec<u8>,
    /// Field 3, varint (uint32). Creation timestamp in seconds UTC.
    /// NOT USED.
    pub creation_time_seconds: Option<u32>,
    /// Field 4, length-delimited (bytes). PKCS#1 ASN.1 DER-encoded RSA public key.
    /// USED: RSA-OAEP encryption key for privacy mode, RSA-PSS verification key for
    /// root certificate signature checks.
    pub public_key: Vec<u8>,
    /// Field 5, varint (uint32). Widevine system ID. NOT USED.
    pub system_id: Option<u32>,
    /// Field 6, varint (bool). DEPRECATED. Test device indicator. NOT USED.
    pub test_device_deprecated: Option<bool>,
    /// Field 7, length-delimited (string). Provider identifier.
    /// USED: copied into EncryptedClientIdentification.provider_id.
    pub provider_id: Option<String>,
    /// Field 8, varint, REPEATED. ServiceType enum values. NOT USED.
    pub service_types: Vec<u32>,
    /// Field 9, varint. Algorithm enum: UNKNOWN=0, RSA=1, ECC_SECP256R1=2,
    /// ECC_SECP384R1=3, ECC_SECP521R1=4. Default: RSA. NOT USED.
    pub algorithm: Option<u32>,
    /// Field 10, length-delimited (bytes). Root of trust identifier. NOT USED.
    pub rot_id: Option<Vec<u8>>,
    /// Field 11, length-delimited. EncryptionKey nested message. NOT USED.
    pub encryption_key: Option<Vec<u8>>,
    /// Field 12, varint (uint32). Expiration timestamp in seconds UTC. NOT USED.
    pub expiration_time_seconds: Option<u32>,
}
```

### 5.3 Serialization (`protobuf/serialize.rs`)

```rust
/// Serialize a LicenseRequest into protobuf wire format bytes.
///
/// Field serialization order follows protobuf convention (by field number).
/// Each field is serialized as: encode_tag() + encode_varint()/encode_length_delimited().
///
/// The output bytes of this function serve three purposes:
///   1. They become SignedMessage.msg (the payload to sign and send).
///   2. They are passed to rsa_pss_sha1_sign as the raw message input (hashing is internal).
///   3. They are used to build the derivation contexts (see build_enc_context / build_mac_context).
pub fn serialize_license_request(req: &LicenseRequest) -> Vec<u8>;

/// Serialize an EncryptedClientIdentification into protobuf wire format bytes.
/// Used when privacy mode is active: the output becomes LicenseRequest.encrypted_client_id
/// (field 8, tag 0x42, length-delimited).
///
/// Fields serialized (all 5, in order):
///   tag 0x0A: provider_id (string)
///   tag 0x12: service_certificate_serial_number (bytes)
///   tag 0x1A: encrypted_client_id (bytes)
///   tag 0x22: encrypted_client_id_iv (bytes)
///   tag 0x2A: encrypted_privacy_key (bytes)
pub fn serialize_encrypted_client_id(ecid: &EncryptedClientIdentification) -> Vec<u8>;

/// Serialize a SignedMessage for the license request direction (client → server).
///
/// Only 3 of SignedMessage's fields are serialized for outgoing messages. Fields 4
/// (session_key) and 9 (oemcrypto_core_message) are server-to-client only.
///
/// Fields serialized:
///   tag 0x08, varint: msg_type (1 = LICENSE_REQUEST)
///   tag 0x12, length-delimited: msg (serialized LicenseRequest)
///   tag 0x1A, length-delimited: signature (RSA-PSS-SHA1 output)
pub fn serialize_signed_message(msg: &SignedMessage) -> Vec<u8>;
```

### 5.4 Deserialization (`protobuf/deserialize.rs`)

```rust
/// Deserialize a SignedMessage from wire format bytes (license response).
///
/// Must handle fields in any order (protobuf does not guarantee field ordering).
/// Critical fields: type (1), msg (2), signature (3), session_key (4),
/// oemcrypto_core_message (9).
/// Unknown fields must be skipped (read tag, determine wire type, skip appropriate bytes).
pub fn deserialize_signed_message(data: &[u8]) -> Result<SignedMessage, CdmError>;

/// Deserialize a License from wire format bytes (the msg field of a LICENSE-type SignedMessage).
///
/// Must extract: id (field 1) containing request_id, and all key containers (field 3, repeated).
pub fn deserialize_license(data: &[u8]) -> Result<License, CdmError>;

/// Deserialize a single KeyContainer from wire format bytes.
///
/// 12 proto fields total; see KeyContainer struct in Section 5.2 for full listing.
/// Used fields: 1 (id), 2 (iv), 3 (key), 4 (type), 9 (operator_session_key_permissions,
/// only for OPERATOR_SESSION keys). Remaining fields must be parsed and skipped by wire type.
/// The key field (3) is still encrypted at this point.
pub fn deserialize_key_container(data: &[u8]) -> Result<KeyContainer, CdmError>;

/// Deserialize a SignedDrmCertificate from wire format bytes.
///
/// Fields: drm_certificate (1), signature (2), signer (3), hash_algorithm (4).
/// Only fields 1 and 2 are required for verification.
pub fn deserialize_signed_drm_certificate(data: &[u8]) -> Result<SignedDrmCertificate, CdmError>;

/// Deserialize a DrmCertificate from wire format bytes.
///
/// 12 proto fields total; see DrmCertificate struct in Section 5.2 for full listing.
pub fn deserialize_drm_certificate(data: &[u8]) -> Result<DrmCertificate, CdmError>;
```

---

## 6. Cryptographic Operations (`crypto/`)

### 6.1 RSA Operations (`crypto/rsa.rs`)

```rust
/// RSA-PSS-SHA1 signing for license request authentication.
///
/// Parameters (all protocol-mandated, not implementation choices):
///   Hash: SHA-1 (NOT SHA-256)
///   MGF: MGF1-SHA-1
///   Salt length: 20 bytes (SHA-1 digest length)
///   Trailer: 0xBC (standard)
///
/// Input: Raw serialized LicenseRequest bytes (NOT pre-hashed).
/// This function must compute SHA-1(message) exactly once, then apply EMSA-PSS-ENCODE
/// using that digest.
///
/// IMPORTANT — hash ownership:
///   The reference Python implementation passes a pre-computed hash *object* to
///   PyCryptodome's pss.sign(), which uses the digest value directly — it does NOT
///   hash again internally.
///   In Rust, most RSA crate signing APIs (e.g., rsa::pss::SigningKey::sign()) accept
///   raw message bytes and hash internally. If using such an API, pass the raw
///   license_request_bytes directly. Do NOT pre-hash and then pass to an API that
///   also hashes — this would produce a double-hash and an invalid signature.
pub fn rsa_pss_sha1_sign(
    private_key_der: &[u8],  // PKCS#1 DER-encoded RSA private key from WVD
    message: &[u8],          // Raw serialized LicenseRequest bytes — NOT pre-hashed
) -> Result<Vec<u8>, CdmError>;

/// RSA-OAEP-SHA1 decryption for session key recovery.
///
/// Parameters (protocol-mandated):
///   Hash: SHA-1
///   MGF: MGF1-SHA-1
///   Label: empty (b"")
///
/// Input: SignedMessage.session_key (field 4) from the license response.
/// Key: Same RSA private key from the WVD file.
/// Output: Session key bytes (expected 16 bytes for AES-128-CMAC derivation).
///   The reference implementation does not validate the decrypted length. In this
///   Rust spec, the caller must convert the Vec<u8> to [u8; 16] before passing to
///   derive_keys(), which serves as an explicit length validation. If decryption
///   yields non-16-byte output, the conversion fails and should produce
///   CdmError::RsaError. In all observed implementations, the session key is
///   exactly 16 bytes.
pub fn rsa_oaep_sha1_decrypt(
    private_key_der: &[u8],  // PKCS#1 DER-encoded RSA private key from WVD
    ciphertext: &[u8],       // SignedMessage.session_key bytes
) -> Result<Vec<u8>, CdmError>;

/// RSA-OAEP-SHA1 encryption for privacy mode (wrapping the AES privacy key).
///
/// Parameters (same as decrypt):
///   Hash: SHA-1
///   MGF: MGF1-SHA-1
///   Label: empty (b"")
///
/// Input: 16-byte privacy_key (random AES key generated for this request).
/// Key: DrmCertificate.public_key from the verified service certificate (DER-encoded RSA public key).
/// Output: RSA-OAEP ciphertext (size = RSA modulus size, typically 256 bytes for 2048-bit keys).
///
/// Used only by crypto::privacy::encrypt_client_id().
pub fn rsa_oaep_sha1_encrypt(
    public_key_der: &[u8],   // DrmCertificate.public_key from service certificate
    plaintext: &[u8],         // 16-byte privacy_key
) -> Result<Vec<u8>, CdmError>;
```

### 6.2 AES Operations (`crypto/aes.rs`)

```rust
/// AES-CMAC key derivation (RFC 4493).
///
/// Produces three derived keys from the decrypted session key and pre-built
/// derivation contexts. The contexts are built at license *request* time
/// (via build_enc_context / build_mac_context) and stored in the Session.
/// They are then passed into this function at license *response* time,
/// after the session key has been recovered via RSA-OAEP.
///
/// Derive keys using AES-128-CMAC with the session key:
///   enc_key         = CMAC(session_key, [0x01] || enc_context)                   → 16 bytes
///   mac_key_server  = CMAC(session_key, [0x01] || mac_context)
///                   || CMAC(session_key, [0x02] || mac_context)                   → 32 bytes
///   mac_key_client  = CMAC(session_key, [0x03] || mac_context)
///                   || CMAC(session_key, [0x04] || mac_context)                   → 32 bytes
pub fn derive_keys(
    enc_context: &[u8],              // Pre-built at request time, stored in Session
    mac_context: &[u8],              // Pre-built at request time, stored in Session
    session_key: &[u8; 16],          // Decrypted via RSA-OAEP, then validated to 16 bytes
    // NOTE: rsa_oaep_sha1_decrypt returns Vec<u8>. The caller must convert to [u8; 16]
    // before calling derive_keys. This conversion IS the length validation — if OAEP
    // decryption yields non-16-byte output, the try_into() / TryFrom conversion fails
    // and the implementation should return CdmError::RsaError. The [u8; 16] type here
    // is intentional: AES-CMAC requires exactly a 128-bit key.
) -> Result<DerivedKeys, CdmError>;

/// Build the encryption derivation context from serialized LicenseRequest bytes.
/// Called at request time, output stored in Session.contexts.
///
/// Returns: b"ENCRYPTION" || 0x00 || license_request_bytes || [0x00, 0x00, 0x00, 0x80]
///
/// CRITICAL: The license_request_bytes embedded in the context MUST be the exact same
/// bytes that are signed and sent as SignedMessage.msg. If serialization is
/// non-deterministic (producing different bytes for the same logical message), the
/// derived keys will not match the server's derivation. Widevine's proto messages
/// do not use maps (which have non-deterministic serialization in protobuf), so
/// a correct serializer will always produce identical bytes for the same input.
pub fn build_enc_context(license_request_bytes: &[u8]) -> Vec<u8>;

/// Build the authentication derivation context from serialized LicenseRequest bytes.
/// Called at request time, output stored in Session.contexts.
///
/// Returns: b"AUTHENTICATION" || 0x00 || license_request_bytes || [0x00, 0x00, 0x02, 0x00]
pub fn build_mac_context(license_request_bytes: &[u8]) -> Vec<u8>;

/// Single AES-128-CMAC computation (RFC 4493).
/// Key: 16-byte AES key (the session key).
/// Message: counter_byte || context_bytes.
/// Output: 16 bytes (one AES block).
fn aes_cmac(key: &[u8; 16], message: &[u8]) -> [u8; 16];

/// AES-128-CBC decryption of an encrypted content key.
///
/// Key: enc_key (16 bytes, from derive_keys).
/// IV: KeyContainer.iv (proto field 2, 16 bytes).
/// Ciphertext: KeyContainer.key (proto field 3).
/// Output: Decrypted key bytes, still PKCS#7-padded. Caller must unpad via pkcs7_unpad.
pub fn aes_cbc_decrypt_key(
    enc_key: &[u8; 16],
    iv: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CdmError>;

/// AES-128-CBC encryption for privacy mode (ClientIdentification encryption).
///
/// Key: random 16-byte privacy_key (generated per-request).
/// IV: random 16-byte privacy_iv (generated per-request).
/// Plaintext: PKCS#7-padded serialized ClientIdentification bytes.
/// Output: Ciphertext bytes.
///
/// Used only by crypto::privacy::encrypt_client_id().
pub fn aes_cbc_encrypt(
    key: &[u8; 16],
    iv: &[u8; 16],
    plaintext: &[u8],  // Must be pre-padded to AES block size (16 bytes)
) -> Vec<u8>;
```

### 6.3 HMAC Verification (`crypto/hmac.rs`)

```rust
/// HMAC-SHA256 verification of license response signature.
///
/// Key: mac_key_server (32 bytes, from derive_keys).
/// Message: oemcrypto_core_message (if present) || msg
///   — where msg is SignedMessage.msg (the serialized License protobuf)
///   — and oemcrypto_core_message is SignedMessage.oemcrypto_core_message (field 9)
///
/// Returns: true if computed HMAC matches SignedMessage.signature (field 3).
pub fn verify_license_signature(
    mac_key_server: &[u8; 32],
    oemcrypto_core_message: Option<&[u8]>,
    msg: &[u8],
    expected_signature: &[u8],
) -> bool;
```

### 6.4 PKCS#7 Padding (`crypto/padding.rs`)

```rust
/// Remove PKCS#7 padding from a decrypted AES-CBC plaintext.
///
/// The last byte indicates the number of padding bytes (1-16).
/// All padding bytes must have the same value as the last byte.
/// Returns the unpadded data, or CdmError::InvalidPadding if the padding is malformed.
pub fn pkcs7_unpad(data: &[u8], block_size: usize) -> Result<Vec<u8>, CdmError>;

/// Apply PKCS#7 padding to plaintext before AES-CBC encryption.
/// Used by encrypt_client_id() to pad the serialized ClientIdentification
/// before AES-128-CBC encryption.
///
/// Appends 1–16 bytes, each with the value of the padding length.
pub fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8>;
```

### 6.5 Privacy Mode — Client ID Encryption (`crypto/privacy.rs`)

```rust
/// Encrypt the device's ClientIdentification for privacy mode.
///
/// Privacy mode prevents the plaintext device identity from being visible to network
/// observers or intermediary proxy servers.
///
/// Cryptographic operations (two-step hybrid encryption):
///
///   Step 1 — AES-128-CBC encryption of the ClientIdentification:
///     privacy_key = random 16 bytes
///     privacy_iv  = random 16 bytes
///     plaintext   = pkcs7_pad(ClientIdentification.SerializeToString(), block_size=16)
///     ciphertext  = AES-128-CBC-encrypt(privacy_key, privacy_iv, plaintext)
///
///   Step 2 — RSA-OAEP-SHA1 wrapping of the AES key:
///     encrypted_privacy_key = RSA-OAEP-SHA1-encrypt(
///         public_key = DrmCertificate.public_key (from service certificate),
///         plaintext  = privacy_key (16 bytes)
///     )
///     Parameters: Hash=SHA-1, MGF=MGF1-SHA-1, Label=empty.
///
/// The results are packed into an EncryptedClientIdentification protobuf:
///   provider_id                       ← DrmCertificate.provider_id
///   service_certificate_serial_number ← DrmCertificate.serial_number
///   encrypted_client_id               ← ciphertext from Step 1
///   encrypted_client_id_iv            ← privacy_iv
///   encrypted_privacy_key             ← encrypted key from Step 2
///
/// Implementation note: This function takes &DrmCertificate (already parsed).
/// verify_service_certificate() handles unwrapping and verification;
/// encrypt_client_id() handles encryption.
pub fn encrypt_client_id(
    client_id_blob: &[u8],          // Serialized ClientIdentification protobuf (from WvdDevice)
    service_certificate: &DrmCertificate,  // Parsed from verified SignedDrmCertificate
) -> Result<EncryptedClientIdentification, CdmError>;

/// Verify and parse a service privacy certificate.
///
/// The service certificate arrives as either:
///   (a) A SignedMessage with type=SERVICE_CERTIFICATE, whose msg field contains
///       a serialized SignedDrmCertificate, or
///   (b) A direct serialized SignedDrmCertificate.
/// Attempt (a) first, then fall back to (b).
///
/// Verification:
///   1. Parse the outer container to extract a SignedDrmCertificate.
///   2. Verify the signature: RSA-PSS-SHA1-verify(
///          public_key = WIDEVINE_ROOT_PUBLIC_KEY,
///          message    = signed_drm_certificate.drm_certificate,
///          signature  = signed_drm_certificate.signature
///      )
///      Parameters: Hash=SHA-1, MGF=MGF1-SHA-1, Salt=20 bytes.
///   3. Parse signed_drm_certificate.drm_certificate as a DrmCertificate.
///   4. Return the verified SignedDrmCertificate for storage on the Session.
pub fn verify_service_certificate(
    certificate_bytes: &[u8],        // Raw bytes: SignedMessage or SignedDrmCertificate
    root_public_key: &[u8],          // DER-encoded RSA public key from WIDEVINE_ROOT_CERT
) -> Result<SignedDrmCertificate, CdmError>;

/// The Widevine root DrmCertificate, used to verify service certificate signatures.
///
/// This is a hardcoded constant extracted from a SignedDrmCertificate blob that is
/// embedded in every Widevine CDM binary. Only the public_key field (field 4) is
/// used — for RSA-PSS-SHA1 signature verification of service certificates.
///
/// To obtain the actual bytes: locate the root SignedDrmCertificate blob in a CDM binary,
/// base64-decode it, parse as SignedDrmCertificate, parse .drm_certificate as DrmCertificate,
/// read .public_key (field 4). See Verification Cross-Reference (Document B) for the
/// specific source location.
pub const WIDEVINE_ROOT_CERT: &[u8] = &[/* DER-encoded RSA public key bytes */];
```

---

## 7. License Exchange Orchestration (`license.rs`)

This module ties all protocol phases together.

```rust
/// Represents an active license acquisition session.
pub struct Session {
    /// Random 16-byte session identifier, generated via CSPRNG at session creation.
    /// This is a local identifier only — it is NOT sent over the wire (the server
    /// assigns its own session_id in LicenseIdentification.session_id). Used as a
    /// handle for the caller to reference this session across API calls.
    pub id: [u8; 16],
    /// Session number (incrementing counter).
    pub number: u32,
    /// Maps request_id → (enc_context, mac_context) for derivation after response.
    /// The contexts are computed from the serialized LicenseRequest bytes at request time
    /// and stored here until the corresponding license response arrives.
    pub contexts: HashMap<Vec<u8>, (Vec<u8>, Vec<u8>)>,
    /// Verified service privacy certificate, set via verify_service_certificate().
    /// When Some, build_license_challenge() encrypts the client ID (privacy mode).
    /// When None, the plaintext client_id_blob is sent in LicenseRequest.client_id (field 1).
    pub service_certificate: Option<SignedDrmCertificate>,
}

/// Generate request_id based on device type.
///
/// Android: random(4) || 0x00000000 || session_number_as_u64_le → hex().upper().as_bytes()
///          session_number is u32 zero-extended to 8 bytes little-endian
///          Raw: 16 bytes → hex-encoded: 32-byte uppercase ASCII string
/// Chrome:  random(16) — raw bytes, no hex encoding
pub fn generate_request_id(device_type: DeviceType, session_number: u32) -> Vec<u8>;

/// Construct a license challenge (Phase 3).
///
/// Steps:
///   1. Generate request_id based on device type.
///   2. Build LicenseRequest with:
///      - If session.service_certificate is Some and privacy_mode is true:
///        encrypt client_id_blob via crypto::privacy::encrypt_client_id() →
///        set encrypted_client_id (field 8), omit client_id (field 1).
///      - Else: set client_id (field 1) to WvdDevice.client_id_blob, omit encrypted_client_id.
///      - ContentIdentification wrapping PSSH init_data
///      - type = NEW (1)
///      - request_time = current POSIX timestamp
///      - protocol_version = VERSION_2_1 (21)
///      - key_control_nonce = random u32 in [1, 2^31)
///   3. Serialize LicenseRequest to bytes (manual protobuf serialization).
///   4. Build derivation contexts from the serialized bytes:
///      enc_context  = crypto::aes::build_enc_context(&license_request_bytes)
///      mac_context  = crypto::aes::build_mac_context(&license_request_bytes)
///      Store (enc_context, mac_context) in session.contexts[request_id].
///   5. Sign serialized bytes with RSA-PSS-SHA1 using device private key.
///      (rsa_pss_sha1_sign hashes internally — do NOT pre-hash. See Section 6.1.)
///   6. Build SignedMessage(type=LICENSE_REQUEST, msg=serialized, signature=sig).
///   7. Serialize SignedMessage to bytes.
///   8. Return serialized SignedMessage bytes (the "challenge" to POST to the license server).
///
/// Data flow note: The serialized LicenseRequest bytes from step 3 are used in
/// step 4 (context building) and step 5 (signing input). The contexts are stored
/// in the session and will be consumed later by parse_license_response.
pub fn build_license_challenge(
    device: &WvdDevice,
    pssh: &PsshData,
    session: &mut Session,
    privacy_mode: bool,           // Default: true. If true AND session.service_certificate
                                  // is Some, encrypts client ID.
) -> Result<Vec<u8>, CdmError>;

/// Parse a license response and extract content keys (Phases 4-5).
///
/// Steps:
///   1. Deserialize response bytes as SignedMessage.
///   2. Verify SignedMessage.type == LICENSE (2).
///   3. Deserialize SignedMessage.msg as License.
///   4. Look up (enc_context, mac_context) from session.contexts using License.id.request_id.
///   5. Decrypt SignedMessage.session_key via RSA-OAEP-SHA1 → 16-byte session key.
///   6. Derive keys using pre-built contexts from step 4:
///      let keys = crypto::aes::derive_keys(&enc_context, &mac_context, &session_key)
///   7. Verify response signature: HMAC-SHA256(keys.mac_key_server, oemcrypto_core_message || msg).
///   8. For each KeyContainer in License.keys (ALL types):
///      a. Decrypt KeyContainer.key via AES-128-CBC(keys.enc_key, KeyContainer.iv, KeyContainer.key).
///      b. PKCS#7 unpad the result.
///      c. Convert KeyContainer.id to a 16-byte KID (kid_to_uuid algorithm):
///           1. If id is a string (not expected in binary proto): base64-decode to bytes.
///           2. If id is empty: use 16 zero bytes.
///           3. If id bytes, interpreted as ASCII text, consist entirely of digit characters:
///              parse the string as a decimal integer, construct UUID from that
///              integer (big-endian 128-bit). Example: b"12345" → UUID(int=12345).
///              NOTE: For Rust, use .bytes().all(|b| b.is_ascii_digit()) — only ASCII
///              digits 0-9 are relevant in practice (KIDs are binary protobuf data).
///           4. If id is <16 bytes: right-pad with zero bytes to 16 bytes.
///           5. If id is ≥16 bytes: passed directly as 16-byte UUID.
///              The reference implementation calls UUID(bytes=kid) with no slicing.
///              Python's UUID(bytes=) requires exactly 16 bytes and raises ValueError
///              for any other length. In practice, KIDs reaching this branch are
///              exactly 16 bytes.
///         This conversion never fails for well-formed protocol data.
///      d. Produce ContentKey { kid, key, key_type }.
///   9. Remove consumed context: session.contexts.remove(request_id).
///   10. Return Vec<ContentKey> — all extracted keys, all types.
///       Filtering to CONTENT-only happens at output time via format_keys().
pub fn parse_license_response(
    device: &WvdDevice,
    session: &mut Session,
    response_bytes: &[u8],
) -> Result<Vec<ContentKey>, CdmError>;

/// Format extracted keys as KID:KEY hex pairs.
///
/// Filters to CONTENT-type keys only (key_type == 2).
///
/// Output format per CONTENT key: "{kid_hex}:{key_hex}\n"
/// where kid_hex is 32 lowercase hex chars and key_hex is 32 lowercase hex chars.
///
/// Example: "000102030405060708090a0b0c0d0e0f:a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
pub fn format_keys(keys: &[ContentKey]) -> String;
```

---

## 8. End-to-End Data Flow

This section traces data through the entire system.

**Transport note:** The network layer (HTTP POST of challenge bytes to the license server,
receipt of response bytes) is intentionally out of scope. How challenge bytes reach the
server (raw POST body, base64 in JSON wrapper, etc.) varies by license server implementation.

```
┌─────────────────────────────────────────────────────────────────────┐
│ INPUT: DASH MPD URL + License Server URL + WVD file path           │
└──────────────────────────────┬──────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 1: pssh::extract_pssh_from_mpd()                           │
│   MPD XML → <cenc:pssh> element → base64 decode → PSSH box bytes │
│   → pssh::parse_pssh_box() → verify WIDEVINE_SYSTEM_ID           │
│   → extract init_data + key_ids                                  │
│   OUTPUT: PsshData { init_data, key_ids }                        │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 2: wvd::parse_wvd()                                        │
│   WVD bytes → verify magic "WVD" → verify version 2              │
│   → read private_key_der (offset 9, length from u16be at 7)     │
│   → read client_id_blob (offset 11+N, length from u16be at 9+N) │
│   OUTPUT: WvdDevice { device_type, security_level,               │
│           private_key_der, client_id_blob }                      │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 2b (optional): crypto::privacy::verify_service_certificate()│
│   Service certificate bytes (from license server or config)      │
│   → parse as SignedMessage or direct SignedDrmCertificate         │
│   → verify RSA-PSS-SHA1 signature against WIDEVINE_ROOT_CERT     │
│   → parse inner DrmCertificate                                   │
│   → store SignedDrmCertificate on session.service_certificate     │
│   If no service certificate is available, skip this phase.       │
│   Privacy mode will be inactive in Phase 3.                      │
│                                                                  │
│   NOTE: How the certificate bytes are obtained is out of scope   │
│   (typically fetched from the license server via a separate HTTP  │
│   request or bundled in application configuration).              │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 3: license::build_license_challenge()                      │
│   3a. generate_request_id(device_type, session.number)           │
│   3b. Build LicenseRequest struct:                               │
│       If session.service_certificate is Some (privacy mode):     │
│         crypto::privacy::encrypt_client_id(client_id_blob, cert) │
│         → EncryptedClientIdentification → field 8                │
│       Else: client_id_blob → field 1                             │
│       - init_data → ContentIdentification.WidevinePsshData       │
│         .pssh_data[0] (field 1 of inner message)                 │
│       - request_type = 1, protocol_version = 21                  │
│   3c. protobuf::serialize::serialize_license_request()           │
│       → license_request_bytes                                    │
│   3d. Build contexts:                                            │
│       enc_ctx  = b"ENCRYPTION\x00" || license_request_bytes      │
│                || [0x00,0x00,0x00,0x80]                          │
│       mac_ctx  = b"AUTHENTICATION\x00" || license_request_bytes  │
│                || [0x00,0x00,0x02,0x00]                          │
│       Store in session.contexts[request_id]                      │
│   3e. crypto::rsa::rsa_pss_sha1_sign(private_key, lr_bytes)     │
│       → signature_bytes                                          │
│   3f. Build + serialize SignedMessage(type=1, msg, signature)    │
│   OUTPUT: challenge_bytes (to POST to license server)            │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ NETWORK: HTTP POST challenge_bytes → license server              │
│ NETWORK: ← response_bytes                                        │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 4: license::parse_license_response() — key recovery        │
│   4a. Deserialize response as SignedMessage                      │
│   4b. Verify msg_type == 2 (LICENSE)                             │
│   4c. Deserialize .msg as License                                │
│   4d. Lookup (enc_context, mac_context) from                     │
│       session.contexts[License.id.request_id]                    │
│   4e. crypto::rsa::rsa_oaep_sha1_decrypt(private_key,            │
│                                           .session_key)          │
│       → session_key (16 bytes)                                   │
│   4f. crypto::aes::derive_keys(&enc_context, &mac_context,       │
│                                 &session_key)                    │
│       → DerivedKeys { enc_key, mac_key_server, mac_key_client }  │
│   4g. crypto::hmac::verify_license_signature(                    │
│           mac_key_server, oemcrypto_core_message, msg, signature) │
└──────────────────────────────┬───────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────┐
│ Phase 5: Key extraction (ALL types)                              │
│   For each KeyContainer in License.keys:                         │
│   5a. crypto::aes::aes_cbc_decrypt_key(enc_key, .iv, .key)      │
│       → padded plaintext bytes                                   │
│   5b. crypto::padding::pkcs7_unpad(padded, 16)                   │
│       → decrypted key bytes                                      │
│   5c. Convert .id → 16-byte KID (kid_to_uuid: see step 8c)      │
│   5d. Construct ContentKey { kid, key, key_type }                │
│                                                                  │
│   5e. session.contexts.remove(request_id)                        │
│                                                                  │
│   license::format_keys(keys)  — filters to CONTENT type only     │
│   OUTPUT: "kid_hex:key_hex\n" per CONTENT key                    │
└──────────────────────────────────────────────────────────────────┘
```

---

_End of technical specification._

_This document provides type-level architecture for a Rust reimplementation of the Widevine L3 license acquisition protocol. All type definitions, function signatures, cryptographic parameters, and protocol constants are implementation-ready. Implementation requires knowledge of Rust, cryptography, and binary protocols._
