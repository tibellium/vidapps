use thiserror::Error;

use drm_core::PsshError;
use drm_playready_format::FormatError;

/**
    Errors specific to the PlayReady CDM protocol exchange.
*/
#[derive(Debug, Clone, Error)]
pub enum CdmError {
    // ── PSSH (delegated to drm-core) ──────────────────────────────────
    #[error(transparent)]
    PsshCore(#[from] PsshError),

    // ── Format errors (delegated to drm-playready-format) ────────────
    #[error("format error: {0}")]
    Format(String),

    // ── Base64 ─────────────────────────────────────────────────────────
    #[error("invalid base64: {0}")]
    InvalidBase64(String),

    // ── PRD file parsing ──────────────────────────────────────────────
    #[error("invalid PRD magic bytes")]
    PrdBadMagic,
    #[error("PRD file is truncated")]
    PrdTruncated,
    #[error("unsupported PRD version {0}")]
    PrdUnsupportedVersion(u8),

    // ── ECC ────────────────────────────────────────────────────────────
    #[error("ECC key parse failed: {0}")]
    EccKeyParse(String),
    #[error("ECC operation failed: {0}")]
    EccOperation(String),

    // ── AES / CMAC ──────────────────────────────────────────────────────
    #[error("invalid AES-CBC input: {0}")]
    AesCbcInvalidInput(String),
    #[error("invalid PKCS#7 padding")]
    Pkcs7PaddingInvalid,
    #[error("AES-CMAC signature mismatch")]
    CmacMismatch,

    // ── ECDSA ──────────────────────────────────────────────────────────
    #[error("ECDSA signature verification failed")]
    EcdsaSignatureMismatch,
    #[error("ECDSA signing failed: {0}")]
    EcdsaSigningFailed(String),

    // ── ElGamal ────────────────────────────────────────────────────────
    #[error("ElGamal decryption failed: {0}")]
    ElGamalDecryptFailed(String),

    // ── Certificates ──────────────────────────────────────────────────
    #[error("certificate chain verification failed: {0}")]
    CertificateChainInvalid(String),

    // ── XML / SOAP ──────────────────────────────────────────────────────
    #[error("invalid XML: {0}")]
    InvalidXml(String),
    #[error("SOAP fault: {0}")]
    SoapFault(String),

    // ── License exchange ──────────────────────────────────────────────
    #[error("no content keys in license response")]
    NoContentKeys,
    #[error("device key mismatch: license encrypted for different device")]
    DeviceKeyMismatch,
    #[error("unsupported cipher type: {0}")]
    UnsupportedCipherType(String),
    #[error("license integrity check failed")]
    IntegrityCheckFailed,
    #[error("license response signature invalid: {0}")]
    LicenseSignatureInvalid(String),
}

impl From<FormatError> for CdmError {
    fn from(e: FormatError) -> Self {
        Self::Format(e.to_string())
    }
}

/**
    Type alias for results that may return a [`CdmError`].
*/
pub type CdmResult<T> = std::result::Result<T, CdmError>;
