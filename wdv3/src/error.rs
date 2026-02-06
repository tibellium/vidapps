use thiserror::Error;

/// Errors specific to the CDM protocol exchange.
#[derive(Debug, Clone, Error)]
pub enum CdmError {
    // ── Encoding ───────────────────────────────────────────────────────
    #[error("invalid base64: {0}")]
    InvalidBase64(String),

    // ── WVD file parsing ──────────────────────────────────────────────
    #[error("invalid WVD magic bytes")]
    WvdBadMagic,
    #[error("WVD file is truncated")]
    WvdTruncated,
    #[error("unsupported WVD version {0}")]
    WvdUnsupportedVersion(u8),
    #[error("invalid WVD device type {0}")]
    WvdBadDeviceType(u8),
    #[error("invalid WVD security level {0}")]
    WvdBadSecurityLevel(u8),
    #[error("WVD field too large to serialize ({0} bytes, max 65535)")]
    WvdFieldTooLarge(usize),

    // ── PSSH box parsing ──────────────────────────────────────────────
    #[error("malformed PSSH box: {0}")]
    PsshMalformed(String),
    #[error("PSSH system ID does not match Widevine")]
    PsshSystemIdMismatch,

    // ── Protobuf ──────────────────────────────────────────────────────
    #[error("protobuf decode failed: {0}")]
    ProtobufDecode(String),

    // ── RSA ───────────────────────────────────────────────────────────
    #[error("RSA key parse failed: {0}")]
    RsaKeyParse(String),
    #[error("RSA operation failed: {0}")]
    RsaOperation(String),

    // ── AES / padding ─────────────────────────────────────────────────
    #[error("invalid AES-CBC input: {0}")]
    AesCbcInvalidInput(String),
    #[error("invalid PKCS#7 padding")]
    Pkcs7PaddingInvalid,

    // ── HMAC ──────────────────────────────────────────────────────────
    #[error("HMAC-SHA256 signature mismatch")]
    HmacMismatch,

    // ── Certificates ──────────────────────────────────────────────────
    #[error("certificate decode failed: {0}")]
    CertificateDecode(String),
    #[error("certificate signature verification failed")]
    CertificateSignatureMismatch,

    // ── License exchange ──────────────────────────────────────────────
    #[error("no content keys in license response")]
    NoContentKeys,
    #[error("no session context for request_id")]
    ContextNotFound,
}

impl From<prost::DecodeError> for CdmError {
    fn from(e: prost::DecodeError) -> Self {
        Self::ProtobufDecode(e.to_string())
    }
}

/// Type alias for results that may return a [`CdmError`].
pub type CdmResult<T> = std::result::Result<T, CdmError>;

/// Error returned by `FromStr` implementations on enum types.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {kind} '{value}'")]
pub struct ParseError {
    pub kind: &'static str,
    pub value: String,
}
