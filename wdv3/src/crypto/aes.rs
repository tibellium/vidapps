use aes::{
    Aes128,
    cipher::{BlockDecrypt, BlockEncrypt, KeyInit},
};
use cmac::{Cmac, Mac};

use crate::error::CdmError;
use crate::types::DerivedKeys;

/// AES-CMAC key derivation (RFC 4493).
///
/// Produces three derived keys from the decrypted session key and pre-built
/// derivation contexts. The contexts are built at license *request* time
/// (via build_enc_context / build_mac_context) and stored in the Session.
/// They are then passed into this function at license *response* time,
/// after the session key has been recovered via RSA-OAEP.
///
/// Derive keys using AES-128-CMAC with the session key:
///   enc_key         = CMAC(session_key, [0x01] || enc_context)                   -> 16 bytes
///   mac_key_server  = CMAC(session_key, [0x01] || mac_context)
///                   || CMAC(session_key, [0x02] || mac_context)                   -> 32 bytes
///   mac_key_client  = CMAC(session_key, [0x03] || mac_context)
///                   || CMAC(session_key, [0x04] || mac_context)                   -> 32 bytes
pub fn derive_keys(
    enc_context: &[u8],
    mac_context: &[u8],
    session_key: &[u8; 16],
) -> Result<DerivedKeys, CdmError> {
    let enc_key = aes_cmac(session_key, &prefixed(0x01, enc_context));

    let mut mac_key_server = [0u8; 32];
    mac_key_server[..16].copy_from_slice(&aes_cmac(session_key, &prefixed(0x01, mac_context)));
    mac_key_server[16..].copy_from_slice(&aes_cmac(session_key, &prefixed(0x02, mac_context)));

    let mut mac_key_client = [0u8; 32];
    mac_key_client[..16].copy_from_slice(&aes_cmac(session_key, &prefixed(0x03, mac_context)));
    mac_key_client[16..].copy_from_slice(&aes_cmac(session_key, &prefixed(0x04, mac_context)));

    Ok(DerivedKeys {
        enc_key,
        mac_key_server,
        mac_key_client,
    })
}

/// Build the encryption derivation context from serialized LicenseRequest bytes.
/// Called at request time, output stored in Session.contexts.
///
/// Returns: b"ENCRYPTION" || 0x00 || license_request_bytes || [0x00, 0x00, 0x00, 0x80]
pub fn build_enc_context(license_request_bytes: &[u8]) -> Vec<u8> {
    let label = b"ENCRYPTION\x00";
    let trailer = [0x00, 0x00, 0x00, 0x80];
    let mut out = Vec::with_capacity(label.len() + license_request_bytes.len() + trailer.len());
    out.extend_from_slice(label);
    out.extend_from_slice(license_request_bytes);
    out.extend_from_slice(&trailer);
    out
}

/// Build the authentication derivation context from serialized LicenseRequest bytes.
/// Called at request time, output stored in Session.contexts.
///
/// Returns: b"AUTHENTICATION" || 0x00 || license_request_bytes || [0x00, 0x00, 0x02, 0x00]
pub fn build_mac_context(license_request_bytes: &[u8]) -> Vec<u8> {
    let label = b"AUTHENTICATION\x00";
    let trailer = [0x00, 0x00, 0x02, 0x00];
    let mut out = Vec::with_capacity(label.len() + license_request_bytes.len() + trailer.len());
    out.extend_from_slice(label);
    out.extend_from_slice(license_request_bytes);
    out.extend_from_slice(&trailer);
    out
}

/// Single AES-128-CMAC computation (RFC 4493).
/// Key: 16-byte AES key (the session key).
/// Message: arbitrary bytes (counter_byte || context_bytes assembled by caller).
/// Output: 16 bytes (one AES block).
fn aes_cmac(key: &[u8; 16], message: &[u8]) -> [u8; 16] {
    let mut mac = <Cmac<Aes128> as Mac>::new_from_slice(key)
        .expect("CMAC key length is always valid for AES-128");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

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
) -> Result<Vec<u8>, CdmError> {
    if iv.len() != 16 || ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
        return Err(CdmError::AesCbcInvalidInput(
            "IV must be 16 bytes and ciphertext must be non-empty and block-aligned".into(),
        ));
    }

    let cipher = Aes128::new(enc_key.into());
    let mut plaintext = Vec::with_capacity(ciphertext.len());
    let mut prev: [u8; 16] = iv.try_into().unwrap();

    for chunk in ciphertext.chunks_exact(16) {
        let saved: [u8; 16] = chunk.try_into().unwrap();
        let mut block = *aes::cipher::generic_array::GenericArray::from_slice(chunk);
        cipher.decrypt_block(&mut block);
        let decrypted: [u8; 16] = block.into();
        for i in 0..16 {
            plaintext.push(decrypted[i] ^ prev[i]);
        }
        prev = saved;
    }

    Ok(plaintext)
}

/// AES-128-CBC encryption for privacy mode (ClientIdentification encryption).
///
/// Key: random 16-byte privacy_key (generated per-request).
/// IV: random 16-byte privacy_iv (generated per-request).
/// Plaintext: PKCS#7-padded serialized ClientIdentification bytes.
/// Output: Ciphertext bytes.
///
/// Used only by crypto::privacy::encrypt_client_id().
pub fn aes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    debug_assert!(
        !plaintext.is_empty() && plaintext.len().is_multiple_of(16),
        "plaintext must be pre-padded to AES block size"
    );

    let cipher = Aes128::new(key.into());
    let mut ciphertext = Vec::with_capacity(plaintext.len());
    let mut prev = *iv;

    for chunk in plaintext.chunks_exact(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        let mut block_ga = aes::cipher::generic_array::GenericArray::from(block);
        cipher.encrypt_block(&mut block_ga);
        prev.copy_from_slice(&block_ga);
        ciphertext.extend_from_slice(&block_ga);
    }

    ciphertext
}

/// Prepend a single counter byte to context bytes.
fn prefixed(counter: u8, context: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(1 + context.len());
    msg.push(counter);
    msg.extend_from_slice(context);
    msg
}
