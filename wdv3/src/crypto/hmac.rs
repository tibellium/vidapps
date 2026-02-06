use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::CdmError;

/**
    HMAC-SHA256 verification of license response signature.

    Key: mac_key_server (32 bytes, from derive_keys).
    Message: oemcrypto_core_message (if present) || msg
      — where msg is SignedMessage.msg (the serialized License protobuf)
      — and oemcrypto_core_message is SignedMessage.oemcrypto_core_message (field 9)

    Returns Ok(()) if computed HMAC matches SignedMessage.signature (field 3),
    or Err(CdmError::HmacMismatch) on mismatch.
*/
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

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::Mac;

    /// Compute HMAC-SHA256 for test reference values.
    fn compute_hmac(key: &[u8; 32], parts: &[&[u8]]) -> Vec<u8> {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
        for part in parts {
            mac.update(part);
        }
        mac.finalize().into_bytes().to_vec()
    }

    #[test]
    fn valid_signature_without_oemcrypto() {
        let key = [0x42u8; 32];
        let msg = b"license-response-bytes";
        let sig = compute_hmac(&key, &[msg]);
        verify_license_signature(&key, None, msg, &sig).unwrap();
    }

    #[test]
    fn valid_signature_with_oemcrypto() {
        let key = [0xAA; 32];
        let ocm = b"oemcrypto-core-message";
        let msg = b"license-msg";
        // HMAC is computed over ocm || msg
        let sig = compute_hmac(&key, &[ocm.as_slice(), msg.as_slice()]);
        verify_license_signature(&key, Some(ocm), msg, &sig).unwrap();
    }

    #[test]
    fn wrong_signature_fails() {
        let key = [0x01; 32];
        let msg = b"data";
        let bad_sig = vec![0u8; 32];
        let err = verify_license_signature(&key, None, msg, &bad_sig).unwrap_err();
        assert!(matches!(err, CdmError::HmacMismatch));
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = [0x01; 32];
        let key2 = [0x02; 32];
        let msg = b"data";
        let sig = compute_hmac(&key1, &[msg]);
        let err = verify_license_signature(&key2, None, msg, &sig).unwrap_err();
        assert!(matches!(err, CdmError::HmacMismatch));
    }

    #[test]
    fn oemcrypto_presence_changes_signature() {
        let key = [0xBB; 32];
        let msg = b"msg";
        let ocm = b"ocm";
        let sig_without = compute_hmac(&key, &[msg]);
        let sig_with = compute_hmac(&key, &[ocm.as_slice(), msg.as_slice()]);
        // Signatures must differ
        assert_ne!(sig_without, sig_with);
        // Cross-verifying must fail
        assert!(verify_license_signature(&key, Some(ocm), msg, &sig_without).is_err());
        assert!(verify_license_signature(&key, None, msg, &sig_with).is_err());
    }
}
