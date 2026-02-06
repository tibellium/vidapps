use rsa::{
    RsaPrivateKey, RsaPublicKey, oaep,
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey},
    pss,
    traits::{Decryptor, RandomizedEncryptor},
};
use sha1::Sha1;
use signature::RandomizedSigner;

use crate::error::CdmError;

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
/// IMPORTANT -- hash ownership:
///   The reference Python implementation passes a pre-computed hash *object* to
///   PyCryptodome's pss.sign(), which uses the digest value directly -- it does NOT
///   hash again internally.
///   In Rust, rsa::pss::SigningKey::try_sign_with_rng() accepts raw message bytes
///   and hashes internally. Pass the raw license_request_bytes directly. Do NOT
///   pre-hash and then pass to this API -- that would produce a double-hash and
///   an invalid signature.
pub fn rsa_pss_sha1_sign(private_key_der: &[u8], message: &[u8]) -> Result<Vec<u8>, CdmError> {
    let private_key = RsaPrivateKey::from_pkcs1_der(private_key_der)
        .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;

    let signing_key = pss::SigningKey::<Sha1>::new_with_salt_len(private_key, 20);
    let mut rng = rand::thread_rng();
    let signature = signing_key
        .try_sign_with_rng(&mut rng, message)
        .map_err(|e| CdmError::RsaOperation(e.to_string()))?;

    let bytes: Box<[u8]> = signature.into();
    Ok(bytes.into_vec())
}

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
///   The caller must convert the Vec<u8> to [u8; 16] before passing to
///   derive_keys(). This conversion IS the length validation -- if OAEP
///   decryption yields non-16-byte output, the try_into() fails and should
///   produce CdmError::RsaOperation.
pub fn rsa_oaep_sha1_decrypt(
    private_key_der: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CdmError> {
    let private_key = RsaPrivateKey::from_pkcs1_der(private_key_der)
        .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;

    let decrypting_key = oaep::DecryptingKey::<Sha1>::new(private_key);
    decrypting_key
        .decrypt(ciphertext)
        .map_err(|e| CdmError::RsaOperation(e.to_string()))
}

/// RSA-OAEP-SHA1 encryption for privacy mode (wrapping the AES privacy key).
///
/// Parameters (same as decrypt):
///   Hash: SHA-1
///   MGF: MGF1-SHA-1
///   Label: empty (b"")
///
/// Input: 16-byte privacy_key (random AES key generated for this request).
/// Key: DrmCertificate.public_key from the verified service certificate
///      (DER-encoded RSA public key).
/// Output: RSA-OAEP ciphertext (size = RSA modulus size, typically 256 bytes
///         for 2048-bit keys).
///
/// Used only by crypto::privacy::encrypt_client_id().
pub fn rsa_oaep_sha1_encrypt(public_key_der: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CdmError> {
    let public_key = RsaPublicKey::from_pkcs1_der(public_key_der)
        .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;

    let encrypting_key = oaep::EncryptingKey::<Sha1>::new(public_key);
    let mut rng = rand::thread_rng();
    encrypting_key
        .encrypt_with_rng(&mut rng, plaintext)
        .map_err(|e| CdmError::RsaOperation(e.to_string()))
}
