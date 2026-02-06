use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::CdmError;

/// HMAC-SHA256 verification of license response signature.
///
/// Key: mac_key_server (32 bytes, from derive_keys).
/// Message: oemcrypto_core_message (if present) || msg
///   — where msg is SignedMessage.msg (the serialized License protobuf)
///   — and oemcrypto_core_message is SignedMessage.oemcrypto_core_message (field 9)
///
/// Returns Ok(()) if computed HMAC matches SignedMessage.signature (field 3),
/// or Err(CdmError::HmacMismatch) on mismatch.
pub fn verify_license_signature(
    mac_key_server: &[u8; 32],
    oemcrypto_core_message: Option<&[u8]>,
    msg: &[u8],
    expected_signature: &[u8],
) -> Result<(), CdmError> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(mac_key_server)
        .expect("HMAC key length is always valid for 32-byte key");
    if let Some(ocm) = oemcrypto_core_message {
        mac.update(ocm);
    }
    mac.update(msg);
    mac.verify_slice(expected_signature)
        .map_err(|_| CdmError::HmacMismatch)
}
