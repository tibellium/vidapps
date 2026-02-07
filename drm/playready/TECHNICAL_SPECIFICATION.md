# TECHNICAL SPECIFICATION

## PlayReady SL2000 CDM — Protocol Architecture and Reimplementation Blueprint

### Type Definitions, Binary Format Specifications, and Cryptographic Data Flow

---

**Document Classification: CONFIDENTIAL**

Purpose: Provide a complete structural blueprint for a Rust reimplementation of the Microsoft PlayReady SL2000 license acquisition protocol. Every type, function signature, binary format, and cryptographic operation is documented with its corresponding protocol phase. This document does not contain runnable code.

Derived from the mspr_toolkit by Security Explorations (Adam Gowdiak).

---

## 1. Module Architecture

The reimplementation is organized into modules that mirror the protocol phases. PlayReady differs architecturally from Widevine in three fundamental ways: (1) ECC P-256 key pairs replace RSA-2048 keys, (2) XML/SOAP wire format replaces protobuf, and (3) the XMR binary license container replaces protobuf-encoded License messages.

```
playready_sl2000/
├── main.rs                  // Entry point: orchestrates full license acquisition
├── prd.rs                   // Phase 2: PRD file parsing and credential loading
├── bcert/
│   ├── mod.rs               // BCert and BCertChain binary parsing
│   ├── structs.rs           // Binary struct definitions for certificate attributes
│   └── verify.rs            // Certificate chain verification against Microsoft root
├── pssh.rs                  // Phase 1: PlayReady PSSH box and WRMHEADER parsing
├── crypto/
│   ├── mod.rs               // Crypto facade: ecc256_encrypt/decrypt/sign/verify
│   ├── ecc_key.rs           // ECCKey: P-256 key pair management, serialization
│   ├── elgamal.rs           // ECC ElGamal point encryption/decryption
│   └── key_wrap.rs          // AES Key Wrap (RFC 3394) for device provisioning
├── challenge/
│   ├── mod.rs               // Phase 3: License challenge XML construction
│   ├── xml_key.rs           // Session key: random ECC point → AES key/IV derivation
│   ├── builder.rs           // XML element construction for AcquireLicense message
│   └── soap.rs              // SOAP envelope creation and serialization
├── license/
│   ├── mod.rs               // Phase 4: License response parsing and verification
│   ├── xmr.rs               // XMR binary license format parsing
│   ├── xmr_types.rs         // XMR object type enumeration (96 types)
│   └── key_extract.rs       // Phase 5: ElGamal content key decryption
├── wrmheader.rs             // WRMHEADER XML parsing (v4.0.0.0 through v4.3.0.0)
└── types.rs                 // Shared types: ContentKey, SecurityLevel, errors
```

---

## 2. Shared Types (`types.rs`)

These types are used across module boundaries. Each encodes a PlayReady-specific protocol concept.

```rust
/// PlayReady security level, extracted from the leaf BCert's BasicInfo.
/// Values are defined by Microsoft's PlayReady compliance and robustness rules.
/// SL150 and SL2000 are software-based; SL3000 requires hardware TEE.
#[repr(u32)]
pub enum SecurityLevel {
    SL150 = 150,
    SL2000 = 2000,
    SL3000 = 3000,
}

/// Content encryption algorithm, from WRMHEADER KID AlgId attribute.
pub enum ContentAlgorithm {
    /// AES-128-CTR (CENC mode). Most common for DASH/HLS content.
    AesCtr,
    /// AES-128-CBC (CBCS mode). Used for HLS with pattern encryption.
    AesCbc,
    /// Legacy PlayReady cocktail cipher. Obsolete but still defined.
    Cocktail,
}

/// Key type from XMR ContentKeyObject.key_type field.
/// Ref: XMR specification, ContentKeyObject structure.
#[repr(u16)]
pub enum KeyType {
    Invalid = 0x0000,
    Aes128Ctr = 0x0001,
    Rc4Cipher = 0x0002,
    Aes128Ecb = 0x0003,
    Cocktail = 0x0004,
    Aes128Cbc = 0x0005,
    KeyExchange = 0x0006,
    Unknown = 0xFFFF,
}

/// Cipher type from XMR ContentKeyObject.cipher_type field.
/// Determines how the encrypted_key blob is decrypted.
#[repr(u16)]
pub enum CipherType {
    Invalid = 0x0000,
    /// RSA-1024 encryption. Legacy, not used in SL2000.
    Rsa1024 = 0x0001,
    /// Chained license. Content key derived from parent license.
    ChainedLicense = 0x0002,
    /// ECC-256 ElGamal encryption. Standard path for SL2000.
    Ecc256 = 0x0003,
    /// ECC-256 ElGamal with KZ (integrity key separation). Variant of standard path.
    Ecc256WithKz = 0x0004,
    /// TEE transient key. Hardware-only, not applicable to SL2000 software extraction.
    TeeTransient = 0x0005,
    /// ECC-256 via symmetric key derivation. Used for scalable licenses.
    Ecc256ViaSymmetric = 0x0006,
    Unknown = 0xFFFF,
}

/// A content decryption key extracted from an XMR license.
pub struct ContentKey {
    /// Key ID: UUID (16 bytes, little-endian byte order as stored in XMR).
    /// From ContentKeyObject.key_id. Displayed as standard UUID string
    /// after bytes_le → UUID conversion.
    pub key_id: [u8; 16],
    /// Decrypted content key: 16 bytes for AES-128.
    /// Result of ElGamal decryption of ContentKeyObject.encrypted_key,
    /// followed by byte splitting (bytes 16..32 of decrypted point x-coordinate).
    pub key: [u8; 16],
    /// Key type from ContentKeyObject.key_type.
    pub key_type: KeyType,
    /// Cipher type from ContentKeyObject.cipher_type.
    pub cipher_type: CipherType,
}

/// ECC P-256 key pair. PlayReady devices carry two of these:
/// one for signing (ECDSA), one for encryption (ElGamal).
pub struct EccKeyPair {
    /// Private scalar d: 32 bytes, big-endian unsigned integer.
    pub private_key: [u8; 32],
    /// Public point Q = d*G: 64 bytes (32 bytes x-coordinate || 32 bytes y-coordinate),
    /// both big-endian unsigned integers. Uncompressed form without 0x04 prefix.
    pub public_key: [u8; 64],
}

/// PlayReady device credentials loaded from a .prd file.
pub struct DeviceCredentials {
    pub security_level: SecurityLevel,
    /// Group key: signs new leaf certificates. Only present in PRD v3.
    pub group_key: Option<EccKeyPair>,
    /// Encryption key: ElGamal decryption of content keys in license responses.
    pub encryption_key: EccKeyPair,
    /// Signing key: ECDSA-SHA256 signing of license challenges.
    pub signing_key: EccKeyPair,
    /// Group certificate chain: BCertChain binary blob proving device identity.
    pub group_certificate: Vec<u8>,
}

/// Session state for one license acquisition exchange.
pub struct Session {
    /// Random 16-byte session identifier.
    pub id: [u8; 16],
    /// Session key: random ECC P-256 point used to derive AES key/IV
    /// for encrypting client data in the license challenge.
    pub xml_key: XmlKey,
    /// References to device signing and encryption keys, set when
    /// get_license_challenge() is called.
    pub signing_key: Option<EccKeyPair>,
    pub encryption_key: Option<EccKeyPair>,
    /// Extracted content keys, populated after parse_license() completes.
    pub keys: Vec<ContentKey>,
}

/// Session key material derived from a random ECC point.
/// The x-coordinate of a random P-256 point is split to produce
/// both AES-128-CBC key and IV for client data encryption.
pub struct XmlKey {
    /// Random P-256 key pair (ephemeral, generated per session).
    pub point: EccKeyPair,
    /// AES-128-CBC key: bytes [16..32] of the public point x-coordinate.
    pub aes_key: [u8; 16],
    /// AES-128-CBC IV: bytes [0..16] of the public point x-coordinate.
    pub aes_iv: [u8; 16],
}
```

### Error types

```rust
pub enum PlayReadyError {
    InvalidPrd(String),
    InvalidPssh(String),
    InvalidWrmHeader(String),
    InvalidBCert(String),
    InvalidBCertChain(String),
    InvalidSession(String),
    TooManySessions,
    InvalidXmrLicense(String),
    InvalidLicenseResponse(String),
    InvalidSoapMessage(String),
    ServerFault(String),
    CryptoError(String),
    KeyMismatch,
    IntegrityCheckFailed,
}
```

---

## 3. Phase 1 — PSSH Parsing (`pssh.rs`, `wrmheader.rs`)

### 3.1 PlayReady PSSH Box

PlayReady content is identified by System ID `9a04f079-9840-4286-ab92-e65be0885f95`. The PSSH box data field contains a PlayReady Header (PRH), which is a binary container holding one or more PlayReady Objects. Type 1 objects contain WRMHEADER XML encoded as UTF-16-LE.

```rust
/// PlayReady System ID: 9a04f07998404286ab92e65be0885f95
const PLAYREADY_SYSTEM_ID: [u8; 16] = [
    0x9a, 0x04, 0xf0, 0x79, 0x98, 0x40, 0x42, 0x86,
    0xab, 0x92, 0xe6, 0x5b, 0xe0, 0x88, 0x5f, 0x95,
];

/// PSSH box structure (ISO BMFF).
/// Identical outer format to Widevine PSSH; only system_id and payload differ.
struct PsshBox {
    /// Box length including header: u32 big-endian.
    length: u32,
    /// Box type: ASCII "pssh" (0x70737368).
    box_type: [u8; 4],
    /// Version: 0 or 1. Version 1 includes key_ids array.
    version: u8,
    /// Flags: 24 bits, always 0.
    flags: [u8; 3],
    /// System ID: 16 bytes. Must match PLAYREADY_SYSTEM_ID.
    system_id: [u8; 16],
    /// (Version 1 only) Key ID count and key IDs.
    key_ids: Option<Vec<[u8; 16]>>,
    /// Data length: u32 big-endian, followed by PlayReady Header bytes.
    data: Vec<u8>,
}
```

### 3.2 PlayReady Header (PRH)

The PSSH data field contains a PlayReady Header. All integers are **little-endian** (unlike the PSSH box itself which is big-endian).

```rust
/// PlayReady Header binary format.
/// All fields are little-endian (contrast with PSSH box which is big-endian).
struct PlayReadyHeader {
    /// Total length in bytes: u32 little-endian.
    length: u32,
    /// Number of PlayReady Object records: u16 little-endian.
    record_count: u16,
    /// PlayReady Object records.
    records: Vec<PlayReadyObject>,
}

/// A single PlayReady Object within the PRH.
struct PlayReadyObject {
    /// Record type: u16 little-endian.
    ///   1 = Rights Management Header (WRMHEADER XML, UTF-16-LE)
    ///   2 = Reserved
    ///   3 = Embedded License Store
    record_type: u16,
    /// Data length in bytes: u16 little-endian.
    data_length: u16,
    /// Record data. For type 1: UTF-16-LE encoded WRMHEADER XML string.
    data: Vec<u8>,
}
```

**Parsing logic:** Filter records for `record_type == 1`. Decode each as UTF-16-LE to obtain WRMHEADER XML string. The PSSH module also handles direct WRMHEADER input (when no PSSH box wrapper is present) by attempting UTF-16-LE decode detection.

### 3.3 WRMHEADER XML

```rust
/// Parsed WRMHEADER. Four versions exist; each structures KID elements differently.
struct WrmHeader {
    /// Header version: determines KID location within XML tree.
    version: WrmHeaderVersion,
    /// Key IDs with algorithm and optional checksum.
    key_ids: Vec<SignedKeyId>,
    /// License acquisition URL. Optional; may be overridden by application.
    la_url: Option<String>,
    /// License UI URL. Optional.
    lui_url: Option<String>,
    /// Domain service ID. Optional.
    ds_id: Option<String>,
    /// Custom attributes XML subtree. Optional; opaque to CDM.
    custom_attributes: Option<String>,
    /// Decryptor setup hint. Optional.
    decryptor_setup: Option<String>,
    /// Raw UTF-16-LE bytes of the original WRMHEADER. Preserved exactly
    /// for inclusion in the license challenge (re-serialization could alter it).
    raw_data: Vec<u8>,
}

pub enum WrmHeaderVersion {
    /// v4.0.0.0: Single KID at DATA/KID, AlgId at DATA/PROTECTINFO/ALGID
    V4_0_0_0,
    /// v4.1.0.0: Single KID at DATA/PROTECTINFO/KID with VALUE/ALGID/CHECKSUM attributes
    V4_1_0_0,
    /// v4.2.0.0: Multiple KIDs at DATA/PROTECTINFO/KIDS/KID
    V4_2_0_0,
    /// v4.3.0.0: Same structure as v4.2, used with protocol version 5
    V4_3_0_0,
}

struct SignedKeyId {
    /// Key ID as UUID. Stored in XML as base64-encoded little-endian bytes.
    value: [u8; 16],
    /// Content encryption algorithm.
    alg_id: ContentAlgorithm,
    /// Optional checksum for key ID verification.
    /// For AESCTR: first 8 bytes of AES-ECB-encrypt(content_key, kid_bytes_le).
    /// For COCKTAIL: first 7 bytes of 5x iterated SHA-1 of content_key padded to 21 bytes.
    checksum: Option<Vec<u8>>,
}
```

**Version-to-protocol mapping** (used in challenge construction):

- v4.3.0.0 → protocol_version = 5
- v4.2.0.0 → protocol_version = 4
- All others → protocol_version = 1

---

## 4. Phase 2 — Device Credential Loading (`prd.rs`, `bcert/`)

### 4.1 PRD File Format

The `.prd` (PlayReady Device) file is a custom binary format invented by the Python project, analogous to Widevine's `.wvd` file. It packages the three ECC key pairs and the BCertChain needed to impersonate a PlayReady device.

```rust
/// PRD file binary layout.
///
/// The format has evolved through 3 versions. Version 1 was never in production.
/// Version 2 omits the group_key. Version 3 is current and includes all three keys.
struct PrdFile {
    /// Magic bytes: ASCII "PRD" (0x50, 0x52, 0x44).
    signature: [u8; 3],
    /// Format version: u8. Valid values: 1, 2, 3.
    version: u8,
    /// Version-specific payload (see below).
    payload: PrdPayload,
}

/// PRD v2 layout (no group_key):
///   [0..4]     group_certificate_length: u32 big-endian
///   [4..N]     group_certificate: BCertChain binary (N = group_certificate_length)
///   [N..N+96]  encryption_key: 96 bytes (32 private || 64 public)
///   [N+96..N+192] signing_key: 96 bytes (32 private || 64 public)
///
/// PRD v3 layout (current):
///   [0..96]    group_key: 96 bytes (32 private || 64 public)
///   [96..192]  encryption_key: 96 bytes (32 private || 64 public)
///   [192..288] signing_key: 96 bytes (32 private || 64 public)
///   [288..292] group_certificate_length: u32 big-endian
///   [292..]    group_certificate: BCertChain binary

/// Each ECC key is stored as 96 bytes:
///   [0..32]   private key d: 32 bytes, big-endian unsigned integer
///   [32..96]  public key Q: 64 bytes (32 bytes x || 32 bytes y), big-endian
///
/// The public key is always derivable from the private key (Q = d*G on secp256r1),
/// but is stored redundantly. On load, the implementation constructs the key pair
/// from the private key alone, ignoring the stored public key bytes.
/// Source: ecc_key.py line 35, "The public key is always derived from the private key"
```

**Key role separation** — critical difference from Widevine:

| Key              | Role                             | Algorithm                 | When Used                     |
| ---------------- | -------------------------------- | ------------------------- | ----------------------------- |
| `signing_key`    | Signs license challenge XML      | ECDSA-SHA256 (FIPS 186-3) | Phase 3: Challenge generation |
| `encryption_key` | Decrypts content keys in license | ElGamal (ECC P-256)       | Phase 5: Key extraction       |
| `group_key`      | Signs new leaf BCerts            | ECDSA-SHA256 (FIPS 186-3) | Device provisioning only      |

Widevine uses a single RSA-2048 key for both signing (RSA-PSS) and key transport (RSA-OAEP). PlayReady's separation into distinct signing and encryption keys means the encryption private key is the sole critical secret for content key recovery; the signing key is used only for challenge authentication.

### 4.2 BCert and BCertChain Format

PlayReady certificates use a custom binary format (not X.509). The chain is verified against a hardcoded Microsoft root issuer public key.

```rust
/// BCertChain binary format.
/// Source: bcert.py, _BCertStructs.BCertChain
struct BCertChain {
    /// Magic: ASCII "CHAI" (0x43, 0x48, 0x41, 0x49).
    signature: [u8; 4],
    /// Format version: u32 big-endian.
    version: u32,
    /// Total length of the chain in bytes: u32 big-endian.
    total_length: u32,
    /// Flags: u32 big-endian.
    flags: u32,
    /// Number of certificates: u32 big-endian. Must be 1..=6.
    certificate_count: u32,
    /// Certificate entries, ordered leaf-first (index 0 = device cert).
    certificates: Vec<BCert>,
}

/// Single BCert binary format.
/// Source: bcert.py, _BCertStructs.BCert
struct BCert {
    /// Magic: ASCII "CERT" (0x43, 0x45, 0x52, 0x54).
    signature: [u8; 4],
    /// Format version: u32 big-endian.
    version: u32,
    /// Total length including signature attribute: u32 big-endian.
    total_length: u32,
    /// Length of certificate body (excluding signature): u32 big-endian.
    /// Used as the signing payload boundary.
    certificate_length: u32,
    /// Typed attributes, parsed sequentially until data exhausted.
    attributes: Vec<BCertAttribute>,
}

/// BCert attribute header (common to all attribute types).
struct BCertAttributeHeader {
    /// Flags: u16 big-endian.
    ///   0x0001 = MUST_UNDERSTAND
    ///   0x0002 = CONTAINER_OBJ
    flags: u16,
    /// Attribute type tag: u16 big-endian. See BCertObjType enum.
    tag: u16,
    /// Total attribute length including this 8-byte header: u32 big-endian.
    length: u32,
}
```

#### BCert Attribute Types

```rust
/// BCert attribute type tags.
/// Source: bcert.py, BCertObjType enum.
#[repr(u16)]
pub enum BCertObjType {
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

/// BCert certificate types.
/// Source: bcert.py, BCertCertType enum.
#[repr(u32)]
pub enum BCertCertType {
    Unknown = 0x00000000,
    Pc = 0x00000001,
    Device = 0x00000002,
    Domain = 0x00000003,
    Issuer = 0x00000004,
    CrlSigner = 0x00000005,
    Service = 0x00000006,
    Silverlight = 0x00000007,
    Application = 0x00000008,
    Metering = 0x00000009,
    KeyFileSigner = 0x0000000A,
    Server = 0x0000000B,
    LicenseSigner = 0x0000000C,
    SecureTimeServer = 0x0000000D,
    RProvModelAuth = 0x0000000E,
}

/// BCert key usage values. Determines what operations a key is authorized for.
/// Source: bcert.py, BCertKeyUsage enum.
#[repr(u32)]
pub enum BCertKeyUsage {
    Unknown = 0x00000000,
    Sign = 0x00000001,
    EncryptKey = 0x00000002,
    SignCrl = 0x00000003,
    IssuerAll = 0x00000004,
    // ... (22 total values through 0x00000018)
    SignResponse = 0x00000014,
    EncryptKeySampleProtectionAes128Ctr = 0x00000016,
}
```

#### Key Attribute Structures

```rust
/// BasicInfo attribute (tag 0x0001). Present in every BCert.
/// Source: bcert.py, _BCertStructs.BasicInfo
struct BasicInfo {
    /// Certificate ID: 16 bytes.
    cert_id: [u8; 16],
    /// Security level: u32 big-endian. Values: 150, 2000, 3000.
    security_level: u32,
    /// Flags: u32 big-endian. Bit 0 = EXTDATA_PRESENT.
    flags: u32,
    /// Certificate type: u32 big-endian. See BCertCertType.
    cert_type: u32,
    /// SHA-256 digest of the signing public key: 32 bytes.
    public_key_digest: [u8; 32],
    /// Expiration date: u32 big-endian (POSIX timestamp).
    expiration_date: u32,
    /// Client ID: 16 bytes.
    client_id: [u8; 16],
}

/// KeyInfo attribute (tag 0x0006). Contains the certificate's public keys.
/// Source: bcert.py, _BCertStructs.KeyInfo
struct KeyInfo {
    /// Number of keys: u32 big-endian.
    key_count: u32,
    /// Key entries.
    keys: Vec<CertKey>,
}

/// A single key within a KeyInfo attribute.
struct CertKey {
    /// Key type: u16 big-endian. 0x0001 = ECC256.
    key_type: u16,
    /// Key length in bits: u16 big-endian. 512 for 64-byte P-256 public key.
    key_length_bits: u16,
    /// Flags: u32 big-endian.
    flags: u32,
    /// Public key bytes: (key_length_bits / 8) bytes.
    /// For ECC256: 64 bytes (32 x || 32 y), big-endian, uncompressed.
    key: Vec<u8>,
    /// Number of usage values: u32 big-endian.
    usages_count: u32,
    /// Usage values: u32 big-endian each. See BCertKeyUsage.
    usages: Vec<u32>,
}

/// SignatureInfo attribute (tag 0x0008). Contains the certificate's signature
/// and the issuer's public key that created it.
/// Source: bcert.py, _BCertStructs.SignatureInfo
struct SignatureInfo {
    /// Signature type: u16 big-endian. 0x0001 = P256 (ECDSA-SHA256).
    signature_type: u16,
    /// Signature length: u16 big-endian.
    signature_size: u16,
    /// ECDSA signature bytes (DER-encoded r,s pair).
    signature: Vec<u8>,
    /// Issuer public key length in bits: u32 big-endian. 512 for P-256.
    signature_key_size_bits: u32,
    /// Issuer public key: 64 bytes (32 x || 32 y).
    signature_key: Vec<u8>,
}

/// ManufacturerInfo attribute (tag 0x0007).
/// Source: bcert.py, _BCertStructs.ManufacturerInfo
struct ManufacturerInfo {
    /// Flags: u32 big-endian.
    flags: u32,
    /// Manufacturer name: length-prefixed, padded to 4-byte alignment.
    manufacturer_name_length: u32,
    manufacturer_name: Vec<u8>,
    /// Model name: length-prefixed, padded to 4-byte alignment.
    model_name_length: u32,
    model_name: Vec<u8>,
    /// Model number: length-prefixed, padded to 4-byte alignment.
    model_number_length: u32,
    model_number: Vec<u8>,
}
```

### 4.3 Certificate Chain Verification

```rust
/// Microsoft PlayReady root issuer public key (P-256, 64 bytes).
/// All legitimate PlayReady certificate chains must terminate at this root.
/// Source: bcert.py, CertificateChain.MSPlayReadyRootIssuerPubKey
const MS_PLAYREADY_ROOT_ISSUER_KEY: [u8; 64] = [
    0x86, 0x4D, 0x61, 0xCF, 0xF2, 0x25, 0x6E, 0x42,
    0x2C, 0x56, 0x8B, 0x3C, 0x28, 0x00, 0x1C, 0xFB,
    0x3E, 0x15, 0x27, 0x65, 0x85, 0x84, 0xBA, 0x05,
    0x21, 0xB7, 0x9B, 0x18, 0x28, 0xD9, 0x36, 0xDE,
    // x-coordinate above (32 bytes), y-coordinate below (32 bytes)
    0x1D, 0x82, 0x6A, 0x8F, 0xC3, 0xE6, 0xE7, 0xFA,
    0x7A, 0x90, 0xD5, 0xCA, 0x29, 0x46, 0xF1, 0xF6,
    0x4A, 0x2E, 0xFB, 0x9F, 0x5D, 0xCF, 0xFE, 0x7E,
    0x43, 0x4E, 0xB4, 0x42, 0x93, 0xFA, 0xC5, 0xAB,
];

/// fn verify_chain(chain: &BCertChain) -> Result<(), PlayReadyError>
///
/// Verification steps (source: bcert.py, CertificateChain.verify_chain):
///
/// 1. Certificate count must be 1..=6.
///
/// 2. For each certificate at index i:
///    a. Extract SignatureInfo attribute.
///    b. Construct signing payload: serialize BCert bytes up to certificate_length.
///    c. ECDSA-SHA256 verify: signature over payload using signature_key from SignatureInfo.
///    d. If BasicInfo.flags has EXTDATA_PRESENT bit set:
///       - Extract ExtDataSignKey attribute → get public key.
///       - Extract ExtDataContainer attribute → serialize record portion.
///       - ECDSA-SHA256 verify: ExtDataContainer.signature over serialized record.
///
/// 3. For adjacent certificates (i > 0):
///    a. Parent (index i) must have cert_type == ISSUER (0x00000004).
///    b. Child's (index i-1) security_level must not exceed parent's expiration_date.
///       (Note: this comparison appears to be a bug in the Python implementation, comparing
///        security_level against expiration_date rather than against security_level.)
///    c. Child's issuer_key (from SignatureInfo.signature_key) must match one of
///       the keys in parent's KeyInfo attribute.
///
/// 4. Root certificate (last in chain): its SignatureInfo.signature_key must equal
///    MS_PLAYREADY_ROOT_ISSUER_KEY.
```

---

## 5. Phase 3 — License Challenge Construction (`challenge/`)

### 5.1 Session Initialization

```rust
/// fn open_session() -> Result<Session, PlayReadyError>
///
/// Source: cdm.py, Cdm.open()
///
/// Steps:
/// 1. Check session count < MAX_NUM_OF_SESSIONS (16).
/// 2. Generate random 16-byte session ID.
/// 3. Create XmlKey (session key material):
///    a. Generate random P-256 key pair (ephemeral, per-session).
///    b. Compute x-coordinate of public point as 32 big-endian bytes.
///    c. Split: aes_iv = x_bytes[0..16], aes_key = x_bytes[16..32].
///       Source: xml_key.py, XmlKey.__init__()
/// 4. Store session in session map, return session ID.
///
/// CRITICAL DESIGN NOTE: The AES key material is derived by splitting the
/// x-coordinate of a random ECC point. This means both the AES key and IV
/// are deterministically linked to the same ECC point. There is no KDF,
/// HKDF, or other key derivation step — the x-coordinate bytes ARE the key
/// material. Security rests entirely on the entropy of the ECC key generation.
```

### 5.2 Client Data Construction

```rust
/// fn build_client_data(
///     certificate_chain: &BCertChain,
///     session: &Session,
/// ) -> Vec<u8>
///
/// Source: cdm.py, Cdm._get_cipher_data() + builder.py, XmlBuilder.ClientData()
///
/// Steps:
/// 1. Build client data XML:
///    <Data>
///      <CertificateChains>
///        <CertificateChain> {base64(bcertchain_bytes)} </CertificateChain>
///      </CertificateChains>
///      <Features>
///        <Feature Name="AESCBC"/>
///        <REE>
///          <AESCBCS/>
///        </REE>
///      </Features>
///    </Data>
///
///    Serialized to UTF-8 bytes with xml_declaration and short_empty_elements=False.
///    NOTE: The CertificateChain base64 value has leading and trailing spaces
///    (source: builder.py line 231, f" {base64...} ").
///
/// 2. AES-128-CBC encrypt:
///    - Key: session.xml_key.aes_key (16 bytes)
///    - IV: session.xml_key.aes_iv (16 bytes)
///    - Plaintext: PKCS#7 padded client data XML bytes
///    - Output: iv || ciphertext (IV prepended to ciphertext)
///    Source: cdm.py lines 80-91
```

### 5.3 ElGamal Session Key Encryption

```rust
/// WMRMServer public key: hardcoded P-256 point used to encrypt the session
/// key so that only the PlayReady license server can decrypt it.
/// Source: cdm.py, Cdm.__init__(), self._wmrm_key
const WMRM_SERVER_KEY: EccPoint = EccPoint {
    x: [
        0xc8, 0xb6, 0xaf, 0x16, 0xee, 0x94, 0x1a, 0xad,
        0xaa, 0x53, 0x89, 0xb4, 0xaf, 0x2c, 0x10, 0xe3,
        0x56, 0xbe, 0x42, 0xaf, 0x17, 0x5e, 0xf3, 0xfa,
        0xce, 0x93, 0x25, 0x4e, 0x7b, 0x0b, 0x3d, 0x9b,
    ],
    y: [
        0x98, 0x2b, 0x27, 0xb5, 0xcb, 0x23, 0x41, 0x32,
        0x6e, 0x56, 0xaa, 0x85, 0x7d, 0xbf, 0xd5, 0xc6,
        0x34, 0xce, 0x2c, 0xf9, 0xea, 0x74, 0xfc, 0xa8,
        0xf2, 0xaf, 0x59, 0x57, 0xef, 0xee, 0xa5, 0x62,
    ],
};

/// fn elgamal_encrypt(public_key: &EccPoint, plaintext_point: &EccPoint) -> [u8; 128]
///
/// Source: elgamal.py, ElGamal.encrypt() + crypto/__init__.py, Crypto.ecc256_encrypt()
///
/// ECC ElGamal encryption over secp256r1:
///
/// 1. Generate random ephemeral scalar k: 0 < k < curve_order.
///    Source: elgamal.py line 19, secrets.randbelow(curve.order)
///
/// 2. Compute point1 = k * G (ephemeral public key).
///    G is the secp256r1 generator point.
///
/// 3. Compute point2 = plaintext_point + (k * public_key).
///    This is standard ElGamal: message point added to shared secret point.
///
/// 4. Serialize as 128 bytes:
///    point1.x (32 bytes) || point1.y (32 bytes) ||
///    point2.x (32 bytes) || point2.y (32 bytes)
///    All big-endian.
///
/// The session key's public point is encrypted to WMRM_SERVER_KEY so that
/// only the license server (holding the corresponding private key) can
/// recover the session point and thus derive the AES key/IV used to encrypt
/// the client data.
///
/// Source: cdm.py line 126:
///   wrmserver_data = Crypto.ecc256_encrypt(self._wmrm_key, session.xml_key.get_point())
```

### 5.4 XML Challenge Assembly

```rust
/// fn get_license_challenge(
///     session: &mut Session,
///     wrm_header: &WrmHeader,
///     device: &DeviceCredentials,
/// ) -> Result<String, PlayReadyError>
///
/// Source: cdm.py, Cdm.get_license_challenge() + builder.py, XmlBuilder.AcquireLicenseMessage()
///
/// Produces a complete SOAP-wrapped license challenge XML string.
///
/// Steps:
///
/// 1. Determine protocol_version from WRMHEADER version:
///    - v4.3.0.0 → 5
///    - v4.2.0.0 → 4
///    - all others → 1
///    Source: cdm.py lines 112-118
///
/// 2. Store signing_key and encryption_key references in session.
///    Source: cdm.py lines 120-121
///
/// 3. Build wrmserver_data: ElGamal encrypt session point to WMRM_SERVER_KEY.
///    Result: 128 bytes. Source: cdm.py line 126
///
/// 4. Build client_data: AES-CBC encrypted client XML. Source: cdm.py line 127
///
/// 5. Construct AcquireLicense XML tree:
///    Source: builder.py, XmlBuilder.AcquireLicenseMessage() and _LicenseAcquisition()
///
///    Full structure (see Section 5.5 for complete XML schema).
///
/// 6. Compute LA element digest:
///    a. Serialize <LA> element to UTF-8 with short_empty_elements=False.
///    b. HTML-unescape the serialized bytes.
///       Source: builder.py line 192, html.unescape(la_xml.decode())
///       NOTE: This unescape step exists because Python's ET serializer
///       may escape characters in the WRMHEADER content that were not
///       originally escaped. Re-serialization must not alter the WRMHEADER.
///    c. SHA-256 hash the unescaped UTF-8 bytes → la_digest (32 bytes).
///       Source: builder.py line 193
///
/// 7. Build <SignedInfo> element containing la_digest as <DigestValue>.
///
/// 8. Compute signature:
///    a. Serialize <SignedInfo> to UTF-8 with short_empty_elements=False.
///    b. SHA-256 hash the SignedInfo bytes.
///    c. ECDSA sign (FIPS 186-3 deterministic) with device.signing_key.
///       Source: crypto/__init__.py line 74, DSS.new(private_key, 'fips-186-3')
///    d. Base64-encode signature → <SignatureValue>.
///
/// 9. Append <KeyInfo> with signing public key to <Signature>.
///
/// 10. Wrap in SOAP envelope:
///     a. Create <soap:Envelope> with namespace declarations.
///     b. Insert AcquireLicense as child of <soap:Body>.
///     c. Serialize with XML declaration prepended.
///        Source: soap_message.py line 92:
///        XML_DECLARATION + html.unescape(xml_data.decode())
///
/// Return: Complete SOAP XML string ready for HTTP POST to license server.
```

### 5.5 Complete Challenge XML Schema

```xml
<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:xsd="http://www.w3.org/2001/XMLSchema"
    xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <AcquireLicense xmlns="http://schemas.microsoft.com/DRM/2007/03/protocols">
      <challenge>
        <Challenge xmlns="http://schemas.microsoft.com/DRM/2007/03/protocols/messages">

          <!-- LA element: signed data block -->
          <LA xmlns="http://schemas.microsoft.com/DRM/2007/03/protocols"
              Id="SignedData" xml:space="preserve">

            <Version>{protocol_version: 1|4|5}</Version>

            <!-- WRMHEADER from PSSH, serialized as UTF-16-LE then presented as string -->
            <ContentHeader>{wrmheader_string}</ContentHeader>

            <CLIENTINFO>
              <CLIENTVERSION>{client_version: "10.0.16384.10011"}</CLIENTVERSION>
            </CLIENTINFO>

            <!-- Optional: revocation list versions -->
            <RevocationLists>
              <RevListInfo>
                <ListID>{base64(list_id_bytes_le)}</ListID>
                <Version>{version_number}</Version>
              </RevListInfo>
            </RevocationLists>

            <!-- Optional: application-supplied custom data -->
            <CustomData>{html_escaped_custom_data}</CustomData>

            <!-- Random nonce: 16 bytes, base64 encoded -->
            <LicenseNonce>{base64(random_16_bytes)}</LicenseNonce>

            <!-- Current POSIX timestamp as integer string -->
            <ClientTime>{posix_timestamp}</ClientTime>

            <!-- Encrypted client data (certificate chain + features) -->
            <EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#"
                           Type="http://www.w3.org/2001/04/xmlenc#Element">
              <EncryptionMethod
                  Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/>
              <KeyInfo xmlns="http://www.w3.org/2000/09/xmldsig#">
                <EncryptedKey xmlns="http://www.w3.org/2001/04/xmlenc#">
                  <EncryptionMethod
                      Algorithm="http://schemas.microsoft.com/DRM/2007/03/protocols#ecc256"/>
                  <KeyInfo xmlns="http://www.w3.org/2000/09/xmldsig#">
                    <KeyName>WMRMServer</KeyName>
                  </KeyInfo>
                  <CipherData>
                    <!-- ElGamal ciphertext: 128 bytes, base64 -->
                    <CipherValue>{base64(elgamal_encrypted_session_point)}</CipherValue>
                  </CipherData>
                </EncryptedKey>
              </KeyInfo>
              <CipherData>
                <!-- AES-128-CBC ciphertext: IV || encrypted client XML, base64 -->
                <CipherValue>{base64(iv_plus_aes_cbc_ciphertext)}</CipherValue>
              </CipherData>
            </EncryptedData>

          </LA>

          <!-- XML Digital Signature over LA element -->
          <Signature xmlns="http://www.w3.org/2000/09/xmldsig#">
            <SignedInfo xmlns="http://www.w3.org/2000/09/xmldsig#">
              <CanonicalizationMethod
                  Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315"/>
              <SignatureMethod
                  Algorithm="http://schemas.microsoft.com/DRM/2007/03/protocols#ecdsa-sha256"/>
              <Reference URI="#SignedData">
                <DigestMethod
                    Algorithm="http://schemas.microsoft.com/DRM/2007/03/protocols#sha256"/>
                <!-- SHA-256 of serialized LA element (after html.unescape) -->
                <DigestValue>{base64(sha256_of_la_xml)}</DigestValue>
              </Reference>
            </SignedInfo>
            <!-- ECDSA-SHA256 signature of serialized SignedInfo element -->
            <SignatureValue>{base64(ecdsa_signature)}</SignatureValue>
            <KeyInfo xmlns="http://www.w3.org/2000/09/xmldsig#">
              <KeyValue>
                <ECCKeyValue>
                  <!-- Device signing public key: 64 bytes, base64 -->
                  <PublicKey>{base64(signing_public_key)}</PublicKey>
                </ECCKeyValue>
              </KeyValue>
            </KeyInfo>
          </Signature>

        </Challenge>
      </challenge>
    </AcquireLicense>
  </soap:Body>
</soap:Envelope>
```

---

## 6. Phase 4 — License Response Parsing (`license/`)

### 6.1 SOAP Response Unwrapping

```rust
/// fn parse_license(
///     session: &mut Session,
///     soap_response: &str,
/// ) -> Result<(), PlayReadyError>
///
/// Source: cdm.py, Cdm.parse_license()
///
/// Steps:
///
/// 1. Parse SOAP XML: extract <soap:Body> content.
///    Handle both "soap:" and "envelope:" namespace prefixes.
///    Source: soap_message.py, SoapMessage.loads()
///
/// 2. Check for SOAP faults:
///    If body contains a <Fault> element, extract:
///    - faultstring (SOAP 1.1) or Reason/Text (SOAP 1.2)
///    - detail/Exception/StatusCode → map to DrmResult error code
///    Raise ServerFault with descriptive message.
///    Source: soap_message.py, SoapMessage.raise_faults()
///
/// 3. Extract <AcquireLicenseResponse> from body.
///    Strip all XML namespaces for simplified traversal.
///    Source: license.py, License.__init__()
///
/// 4. Navigate to <AcquireLicenseResult><Response><LicenseResponse>.
///
/// 5. Extract metadata:
///    - rmsdkVersion: attribute on <Response>
///    - Version: text of <LicenseResponse><Version>
///    - LicenseNonce: text (for replay protection verification)
///    - ResponseID: text
///    - RevInfo: optional <RevInfo> element for revocation list updates
///
/// 6. Optional: Parse SigningCertificateChain if present.
///    Source: license.py line 61
///
/// 7. Verify response signature (if verifiable — Section 6.2).
///
/// 8. Extract XMR licenses:
///    For each <Licenses><License> element:
///    - Base64-decode text content → XMR binary blob.
///    - Parse XMR structure (Section 7).
///    - Extract content key using session.encryption_key (Section 8).
///    - Append to session.keys.
///    Source: cdm.py lines 172-173
///
/// 9. Process revocation list updates if RevInfo is present.
///    Source: cdm.py lines 156-170
```

### 6.2 Response Signature Verification

```rust
/// fn verify_license_response(license: &License) -> Result<(), PlayReadyError>
///
/// Source: license.py, License.verify()
///
/// Prerequisites: SigningCertificateChain, Signature/SignedInfo/DigestValue,
/// and Signature/SignatureValue must all be present.
///
/// Steps:
///
/// 1. Serialize <LicenseResponse> element to UTF-8 bytes.
///    CRITICAL: Use the original namespace-preserved XML (self._original_root),
///    NOT the namespace-stripped version. Register namespace:
///    xmlns="" → "http://schemas.microsoft.com/DRM/2007/03/protocols"
///    Source: license.py lines 98-100
///
/// 2. SHA-256 hash the serialized LicenseResponse → response_hash.
///
/// 3. Base64-decode <DigestValue> from <SignedInfo><Reference>.
///
/// 4. Compare: response_hash must equal decoded digest_value.
///    Source: license.py lines 106-107
///
/// 5. Extract signing public key from SigningCertificateChain:
///    a. Parse as BCertChain.
///    b. Get leaf certificate (index 0).
///    c. Find key with usage SIGN_RESPONSE (0x00000014) in KeyInfo attribute.
///    d. Construct P-256 public key from 64-byte key material.
///    Source: license.py lines 109-116
///
/// 6. Serialize <SignedInfo> element to UTF-8 bytes.
///    Register namespace: xmlns="" → "http://www.w3.org/2000/09/xmldsig#"
///    Source: license.py lines 118-119
///
/// 7. Base64-decode <SignatureValue>.
///
/// 8. ECDSA-SHA256 verify: signature over SignedInfo bytes using extracted public key.
///    Source: license.py line 123
```

---

## 7. XMR Binary License Format (`license/xmr.rs`, `license/xmr_types.rs`)

### 7.1 XMR Container Structure

XMR (eXtensible Media Rights) is a binary TLV (Type-Length-Value) format used to encode PlayReady license data. It is base64-encoded within the XML license response.

```rust
/// XMR license top-level structure.
/// Source: xmrlicense.py, _XMRLicenseStructs.XmrLicense
struct XmrLicense {
    /// Magic bytes: "XMR\x00" (0x584D5200).
    signature: [u8; 4],
    /// XMR format version: u32 big-endian.
    xmr_version: u32,
    /// Rights ID: 16 bytes (UUID).
    rights_id: [u8; 16],
    /// Sequence of XMR objects (containers and leaf objects).
    containers: Vec<XmrObject>,
}

/// XMR object header + payload.
/// Source: xmrlicense.py, _XMRLicenseStructs.XmrObject
struct XmrObject {
    /// Flags: u16 big-endian.
    ///   0 or 1 = leaf object (data is type-specific struct)
    ///   2 or 3 = container object (data is nested XmrObject)
    flags: u16,
    /// Object type: u16 big-endian. See XmrObjectType enum.
    object_type: u16,
    /// Total length including this 8-byte header: u32 big-endian.
    length: u32,
    /// Payload: either a type-specific struct (leaf) or nested XmrObject (container).
    data: XmrObjectData,
}
```

### 7.2 XMR Object Types

```rust
/// XMR object type enumeration.
/// Source: xmrlicense.py, XMRObjectTypes enum.
/// 96 types defined; only the forensically relevant ones are listed here.
#[repr(u16)]
pub enum XmrObjectType {
    Invalid = 0x0000,
    OuterContainer = 0x0001,
    GlobalPolicyContainer = 0x0002,
    MinimumEnvironmentObject = 0x0003,
    PlaybackPolicyContainer = 0x0004,
    OutputProtectionObject = 0x0005,
    UplinkKidObject = 0x0006,
    KeyMaterialContainer = 0x0009,
    /// Contains the encrypted content key. Critical for key extraction.
    ContentKeyObject = 0x000A,
    /// Contains the license integrity signature (AES-CMAC).
    SignatureObject = 0x000B,
    SettingsObject = 0x000D,
    ExpirationObject = 0x0012,
    IssueDateObject = 0x0013,
    RevocationInfoVersion2Object = 0x0032,
    SecurityLevel = 0x0034,
    /// Contains the device's encryption public key.
    /// Used to verify the license is addressed to this device.
    EccDeviceKeyObject = 0x002A,
    /// Auxiliary key object. Presence indicates a scalable license.
    AuxKeyObject = 0x0051,
    /// Uplink X key object. Used in scalable license derivation.
    UplinkXObject = 0x0052,
    OptimizedContentKey2 = 0x005D,
}
```

### 7.3 Key XMR Object Structures

```rust
/// ContentKeyObject (type 0x000A).
/// Contains the ElGamal-encrypted content key.
/// Source: xmrlicense.py, _XMRLicenseStructs.ContentKeyObject
struct ContentKeyObject {
    /// Key ID: 16 bytes (UUID, little-endian byte order).
    key_id: [u8; 16],
    /// Key type: u16 big-endian. See KeyType enum (Section 2).
    key_type: u16,
    /// Cipher type: u16 big-endian. See CipherType enum (Section 2).
    /// Determines decryption method:
    ///   0x0003 (ECC_256): Standard ElGamal, 128-byte encrypted_key.
    ///   0x0004 (ECC_256_WITH_KZ): ElGamal with integrity key separation.
    ///   0x0006 (ECC_256_VIA_SYMMETRIC): Scalable license with AES derivation chain.
    cipher_type: u16,
    /// Encrypted key length: u16 big-endian.
    key_length: u16,
    /// Encrypted key bytes. For ECC_256: 128 bytes (two P-256 points).
    /// For ECC_256_VIA_SYMMETRIC: >128 bytes (128 + embedded license data).
    encrypted_key: Vec<u8>,
}

/// EccDeviceKeyObject (type 0x002A).
/// Contains the public key the license was encrypted to.
/// Source: xmrlicense.py, _XMRLicenseStructs.ECCKeyObject
struct EccDeviceKeyObject {
    /// Curve type: u16 big-endian.
    curve_type: u16,
    /// Key length: u16 big-endian.
    key_length: u16,
    /// Public key bytes: {key_length} bytes.
    /// For P-256: 64 bytes (32 x || 32 y).
    key: Vec<u8>,
}

/// SignatureObject (type 0x000B).
/// Contains AES-CMAC integrity tag over the license binary.
/// Source: xmrlicense.py, _XMRLicenseStructs.SignatureObject
struct SignatureObject {
    /// Signature type: u16 big-endian.
    signature_type: u16,
    /// Signature data length: u16 big-endian.
    signature_data_length: u16,
    /// AES-CMAC tag: {signature_data_length} bytes.
    signature_data: Vec<u8>,
}

/// AuxiliaryKeysObject (type 0x0051).
/// Present in scalable licenses. Contains auxiliary keys for
/// multi-layer key derivation.
/// Source: xmrlicense.py, _XMRLicenseStructs.AuxiliaryKeysObject
struct AuxiliaryKeysObject {
    /// Number of auxiliary keys: u16 big-endian.
    count: u16,
    /// Auxiliary key entries.
    keys: Vec<AuxiliaryKey>,
}

struct AuxiliaryKey {
    /// Key location/index: u32 big-endian.
    location: u32,
    /// Key material: 16 bytes.
    key: [u8; 16],
}
```

### 7.4 XMR Container Navigation

```rust
/// fn locate_object(containers: &[XmrObject], target_type: XmrObjectType) -> Option<&XmrObjectData>
///
/// Source: xmrlicense.py, XMRLicense._locate() + get_object()
///
/// XMR objects are nested: container objects (flags 2 or 3) hold inner objects.
/// To find a specific object type, recursively traverse:
///
/// 1. For each object in containers:
///    a. If flags == 2 or 3: recurse into object.data (which is another XmrObject).
///    b. If flags == 0 or 1: check if object_type matches target_type.
///       If match, return &object.data.
///
/// Example traversal for a typical license:
///   OuterContainer (0x0001, flags=2)
///     └─ GlobalPolicyContainer (0x0002, flags=2)
///          ├─ SecurityLevel (0x0034, flags=0) ← leaf
///          └─ ...
///     └─ KeyMaterialContainer (0x0009, flags=2)
///          ├─ ContentKeyObject (0x000A, flags=0) ← target
///          ├─ EccDeviceKeyObject (0x002A, flags=0) ← target
///          └─ ...
///   SignatureObject (0x000B, flags=0) ← leaf at top level
```

---

## 8. Phase 5 — Content Key Extraction (`license/key_extract.rs`)

### 8.1 Standard Path (CipherType::Ecc256 and Ecc256WithKz)

```rust
/// fn extract_content_key(
///     xmr_license: &XmrLicense,
///     encryption_key: &EccKeyPair,
/// ) -> Result<ContentKey, PlayReadyError>
///
/// Source: xmrlicense.py, XMRLicense.get_content_key()
///
/// Steps:
///
/// 1. DEVICE KEY VERIFICATION
///    Locate EccDeviceKeyObject (type 0x002A) in the XMR license.
///    Verify that ecc_device_key.key == encryption_key.public_key (64 bytes).
///    If mismatch: the license was encrypted to a different device.
///    Source: xmrlicense.py lines 360-365
///
/// 2. CONTENT KEY OBJECT EXTRACTION
///    Locate ContentKeyObject (type 0x000A).
///    Extract cipher_type. Must be one of:
///      ECC_256 (0x0003), ECC_256_WITH_KZ (0x0004), ECC_256_VIA_SYMMETRIC (0x0006)
///    Source: xmrlicense.py lines 367-371
///
/// 3. ELGAMAL DECRYPTION
///    Decrypt content_key_obj.encrypted_key using encryption_key.private_key.
///
///    fn elgamal_decrypt(private_key: &[u8; 32], ciphertext: &[u8; 128]) -> [u8; 32]
///    Source: elgamal.py, ElGamal.decrypt() + crypto/__init__.py, Crypto.ecc256_decrypt()
///
///    a. Parse ciphertext as two P-256 points:
///       point1 = (ciphertext[0..32], ciphertext[32..64])   // ephemeral public key
///       point2 = (ciphertext[64..96], ciphertext[96..128]) // encrypted message
///
///    b. Compute shared secret:
///       shared_secret = private_key_scalar * point1
///       (Scalar multiplication of the device's private key with the ephemeral point.)
///
///    c. Recover plaintext point:
///       decrypted_point = point2 - shared_secret
///       (Elliptic curve point subtraction.)
///
///    d. Extract x-coordinate as 32 big-endian bytes.
///       Source: crypto/__init__.py line 60, Util.to_bytes(decrypted.x)
///       NOTE: Util.to_bytes() pads to even byte length. For P-256 x-coordinates
///       this is always 32 bytes, but the padding logic rounds up if bit_length
///       produces an odd byte count. Source: util.py lines 16-20.
///
/// 4. KEY SPLITTING (non-scalable)
///    For cipher_type ECC_256 or ECC_256_WITH_KZ:
///    decrypted = 32 bytes from step 3d.
///    integrity_key (CI) = decrypted[0..16]    // first 16 bytes
///    content_key  (CK) = decrypted[16..32]    // last 16 bytes
///    Source: xmrlicense.py line 376
///
/// 5. LICENSE INTEGRITY VERIFICATION
///    fn check_signature(xmr_license: &XmrLicense, integrity_key: &[u8; 16]) -> bool
///    Source: xmrlicense.py, XMRLicense.check_signature()
///
///    a. Locate SignatureObject (type 0x000B).
///    b. Compute AES-CMAC:
///       - Key: integrity_key (16 bytes)
///       - Data: XMR license bytes from start through the byte immediately
///         before the signature data within the SignatureObject.
///         Specifically: license_bytes[.. -(signature_data_length + 12)]
///         The 12 accounts for: flags(2) + type(2) + length(4) + sig_type(2) + sig_len(2).
///       Source: xmrlicense.py lines 410-413
///    c. Compare CMAC tag with signature_data. Must be equal.
///
/// 6. RETURN ContentKey
///    key_id: UUID from ContentKeyObject.key_id (bytes_le → UUID conversion).
///    key: content_key (CK), 16 bytes.
///    key_type: from ContentKeyObject.key_type.
///    cipher_type: from ContentKeyObject.cipher_type.
```

### 8.2 Scalable License Path (CipherType::Ecc256ViaSymmetric)

```rust
/// Scalable license key extraction.
/// Source: xmrlicense.py lines 378-396
///
/// This path is taken when:
///   - cipher_type == ECC_256_VIA_SYMMETRIC (0x0006)
///   - AuxKeyObject (type 0x0051) is present in the license
///
/// The encrypted_key field is larger than 128 bytes, structured as:
///   [0..128]  = embedded_root_license (standard ElGamal ciphertext)
///   [128..]   = embedded_leaf_license
///
/// Magic constant used in key derivation:
/// Source: xmrlicense.py, XMRLicense.MagicConstantZero
const MAGIC_CONSTANT_ZERO: [u8; 16] = [
    0x7e, 0xe9, 0xed, 0x4a, 0xf7, 0x73, 0x22, 0x4f,
    0x00, 0xb8, 0xea, 0x7e, 0xfb, 0x02, 0x7c, 0xbb,
];

/// Scalable license derivation steps:
///
/// 1. ElGamal decrypt first 128 bytes → 32 bytes decrypted point x-coordinate.
///    (Same as standard path step 3.)
///
/// 2. INTERLEAVED BYTE SPLIT (different from standard path):
///    CI = decrypted[0::2][:16]   // even-indexed bytes, first 16
///    CK = decrypted[1::2][:16]   // odd-indexed bytes, first 16
///    Source: xmrlicense.py line 379
///
///    Concretely, for 32 decrypted bytes [b0, b1, b2, b3, ...]:
///    CI = [b0, b2, b4, b6, b8, b10, b12, b14, b16, b18, b20, b22, b24, b26, b28, b30]
///    CK = [b1, b3, b5, b7, b9, b11, b13, b15, b17, b19, b21, b23, b25, b27, b29, b31]
///
/// 3. Split encrypted_key into root and leaf:
///    embedded_root_license = encrypted_key[0..144]    // 128 + 16 overlap
///    embedded_leaf_license = encrypted_key[144..]
///    Source: xmrlicense.py lines 382-383
///
/// 4. MULTI-LAYER AES-ECB KEY DERIVATION:
///
///    a. rgb_key = CK XOR MAGIC_CONSTANT_ZERO (16 bytes)
///       Source: xmrlicense.py line 385
///
///    b. content_key_prime = AES-ECB-encrypt(CK, rgb_key)
///       Source: xmrlicense.py line 386
///
///    c. aux_key = first auxiliary key from AuxKeyObject.keys[0].key (16 bytes)
///       Source: xmrlicense.py line 388
///
///    d. uplink_x_key = AES-ECB-encrypt(content_key_prime, aux_key)
///       Source: xmrlicense.py line 390
///
///    e. secondary_key = AES-ECB-encrypt(CK, embedded_root_license[128..144])
///       Source: xmrlicense.py line 391
///       NOTE: This uses bytes 128-144 of the root license blob (the 16 bytes
///       immediately following the 128-byte ElGamal ciphertext).
///
///    f. Decrypt embedded leaf license (two passes):
///       embedded_leaf = AES-ECB-encrypt(uplink_x_key, embedded_leaf_license)
///       embedded_leaf = AES-ECB-encrypt(secondary_key, embedded_leaf)
///       Source: xmrlicense.py lines 393-394
///       NOTE: These use AES-ECB encrypt (not decrypt). This is intentional;
///       the server encrypted using AES-ECB decrypt, so the client "decrypts"
///       by encrypting. This is symmetric (AES-ECB-encrypt == AES-ECB-decrypt
///       only for single-block operations, and these are single-block 16-byte ops).
///
///    g. Final key split:
///       CI = embedded_leaf[0..16]     // integrity key
///       CK = embedded_leaf[16..32]    // content key
///       Source: xmrlicense.py line 396
///
/// 5. Proceed to integrity verification (step 5 of standard path) using new CI.
/// 6. Return ContentKey with new CK.
```

---

## 9. Device Provisioning Key Wrap (`crypto/key_wrap.rs`)

This module handles unwrapping device private keys that are stored in wrapped form. It is used during device provisioning (creating .prd files from raw device data), not during license acquisition.

```rust
/// NIST SP 800-108 KDF followed by AES Key Unwrap (RFC 3394).
/// Source: key_wrap.py, derive_wrapping_key() + unwrap_wrapped_key()
///
/// Two hardcoded constants:
///
/// KeyDerivationCertificatePrivateKeysWrap (label):
const KD_CERT_PRIV_KEYS_WRAP: [u8; 16] = [
    0x9c, 0xe9, 0x34, 0x32, 0xc7, 0xd7, 0x40, 0x16,
    0xba, 0x68, 0x47, 0x63, 0xf8, 0x01, 0xe1, 0x36,
];

/// CTK_TEST (base key for CMAC-based KDF):
const CTK_TEST: [u8; 16] = [
    0x8B, 0x22, 0x2F, 0xFD, 0x1E, 0x76, 0x19, 0x56,
    0x59, 0xCF, 0x27, 0x03, 0x89, 0x8C, 0x42, 0x7F,
];

/// fn derive_wrapping_key() -> [u8; 16]
///
/// NIST SP 800-108 counter-mode KDF using AES-CMAC as PRF.
///
/// Input to CMAC:
///   0x01                          // iteration counter (1 byte)
///   || KD_CERT_PRIV_KEYS_WRAP     // label (16 bytes)
///   || 0x00                       // separator (1 byte)
///   || [0u8; 16]                  // context (16 zero bytes)
///   || 0x00, 0x80                 // output length in bits = 128 (2 bytes, big-endian)
///
/// wrapping_key = AES-CMAC(CTK_TEST, input)
///
/// Source: key_wrap.py lines 6-36

/// fn unwrap_key(wrapped_key: &[u8]) -> [u8; 32]
///
/// AES Key Unwrap per RFC 3394 using derived wrapping_key.
/// Returns first 32 bytes of unwrapped result (remaining 16 bytes are random padding).
///
/// Source: key_wrap.py lines 38-52
```

---

## 10. Cryptographic Primitive Summary

| Operation                  | Algorithm                      | Library (Python equivalents)  | Parameters                                   |
| -------------------------- | ------------------------------ | ----------------------------- | -------------------------------------------- |
| Challenge signing          | ECDSA-SHA256                   | pycryptodome DSS (FIPS 186-3) | Sign serialized SignedInfo XML bytes         |
| Session key exchange       | ECC ElGamal                    | ecpy + custom ElGamal class   | Encrypt session point to WMRM_SERVER_KEY     |
| Client data encryption     | AES-128-CBC                    | pycryptodome AES MODE_CBC     | Key/IV from XmlKey, PKCS#7 padding           |
| Content key decryption     | ECC ElGamal                    | ecpy + custom ElGamal class   | Decrypt using device encryption private key  |
| License integrity          | AES-CMAC                       | pycryptodome CMAC             | 128-bit integrity key from decrypted content |
| Certificate signing        | ECDSA-SHA256                   | pycryptodome DSS (FIPS 186-3) | Verify BCert chain against MS root key       |
| Response verification      | ECDSA-SHA256                   | pycryptodome DSS (FIPS 186-3) | Verify SignedInfo against server cert key    |
| Key wrapping               | AES Key Wrap (RFC 3394)        | cryptography library          | Unwrap device provisioning keys              |
| Key derivation (wrap)      | AES-CMAC KDF (NIST SP 800-108) | pycryptodome CMAC             | Derive wrapping key from CTK_TEST            |
| Scalable key derivation    | AES-128-ECB (multiple passes)  | pycryptodome AES MODE_ECB     | Multi-layer key unwrapping chain             |
| Key ID checksum (AESCTR)   | AES-128-ECB                    | pycryptodome AES MODE_ECB     | First 8 bytes of ECB(key, kid_le)            |
| Key ID checksum (COCKTAIL) | SHA-1 (5 iterations)           | hashlib                       | First 7 bytes of iterated hash               |

---

## Appendix A: Hardcoded Cryptographic Constants

### A.1 WMRM Server ElGamal Public Key

Used to encrypt the session key so only the license server can recover it.

```
x = 0xc8b6af16ee941aadaa5389b4af2c10e356be42af175ef3face93254e7b0b3d9b
y = 0x982b27b5cb2341326e56aa857dbfd5c634ce2cf9ea74fca8f2af5957efeea562
Curve: secp256r1 (NIST P-256)
Source: cdm.py lines 42-46
```

### A.2 Microsoft PlayReady Root Issuer Public Key

Used to verify the root of all BCert certificate chains.

```
x = 0x864d61cff2256e422c568b3c28001cfb3e15276585 84ba0521b79b1828d936de
y = 0x1d826a8fc3e6e7fa7a90d5ca2946f1f64a2efb9f5d cffe7e434eb44293fac5ab
Curve: secp256r1 (NIST P-256)
Source: bcert.py lines 607-612
```

### A.3 Scalable License Magic Constant

Used in the scalable license key derivation XOR step.

```
0x7ee9ed4af773224f00b8ea7efb027cbb
Source: xmrlicense.py lines 309-312
```

### A.4 Device Key Wrap Constants

Used for NIST SP 800-108 KDF during device provisioning.

```
KD_CERT_PRIV_KEYS_WRAP (label): 0x9ce93432c7d74016ba684763f801e136
CTK_TEST (base key):             0x8b222ffd1e76195659cf2703898c427f
Source: key_wrap.py lines 14-22
```
