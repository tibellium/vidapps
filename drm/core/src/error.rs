use thiserror::Error;

use crate::types::SystemId;

/**
    Errors from PSSH box parsing.
*/
#[derive(Debug, Clone, Error)]
pub enum PsshError {
    #[error("invalid base64: {0}")]
    InvalidBase64(String),

    #[error("malformed PSSH box: {0}")]
    Malformed(String),

    #[error("PSSH system ID is {0}, expected {1}")]
    SystemIdMismatch(SystemId, SystemId),
}

/**
    Error returned by `FromStr` implementations on enum types.
*/
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {kind} '{value}'")]
pub struct ParseError {
    pub kind: &'static str,
    pub value: String,
}

/**
    Errors from [`ContentKey`](crate::ContentKey) construction.
*/
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContentKeyError {
    #[error("content key is empty")]
    EmptyKey,
    #[error("key ID must be 16 bytes, got {0}")]
    InvalidKidLength(usize),
    #[error("invalid hex in content key: {0}")]
    InvalidHex(String),
    #[error("expected format 'kid_hex:key_hex'")]
    InvalidFormat,
}
