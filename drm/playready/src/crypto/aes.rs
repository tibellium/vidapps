#![allow(dead_code)]

use aes::{
    Aes128,
    cipher::{BlockDecrypt, BlockEncrypt, KeyInit},
};
use cmac::{Cmac, Mac};

use crate::error::{CdmError, CdmResult};

/**
    Apply PKCS#7 padding to plaintext before AES-CBC encryption.

    Appends 1-16 bytes, each with the value of the padding length.
    If data is already block-aligned, a full 16-byte block is added.
*/
pub fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    let pad = 16 - (data.len() % 16);
    let mut out = Vec::with_capacity(data.len() + pad);
    out.extend_from_slice(data);
    out.resize(data.len() + pad, pad as u8);
    out
}

/**
    Remove PKCS#7 padding from decrypted AES-CBC plaintext.

    The last byte indicates the number of padding bytes (1-16).
    All padding bytes must have the same value.
*/
pub fn pkcs7_unpad(data: &[u8]) -> CdmResult<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(CdmError::Pkcs7PaddingInvalid);
    }

    let pad = data[data.len() - 1] as usize;
    if pad == 0 || pad > 16 || pad > data.len() {
        return Err(CdmError::Pkcs7PaddingInvalid);
    }

    for &byte in &data[data.len() - pad..] {
        if byte as usize != pad {
            return Err(CdmError::Pkcs7PaddingInvalid);
        }
    }

    Ok(data[..data.len() - pad].to_vec())
}

/**
    AES-128-CBC encrypt with PKCS#7 padding.

    Returns `iv || ciphertext` (IV prepended to output), matching the
    PlayReady spec for client data encryption in license challenges.
*/
pub fn aes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let padded = pkcs7_pad(plaintext);
    let cipher = Aes128::new(key.into());

    let mut output = Vec::with_capacity(16 + padded.len());
    output.extend_from_slice(iv);

    let mut prev = *iv;
    for chunk in padded.chunks_exact(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev[i];
        }
        let mut block_ga = aes::cipher::generic_array::GenericArray::from(block);
        cipher.encrypt_block(&mut block_ga);
        prev.copy_from_slice(&block_ga);
        output.extend_from_slice(&block_ga);
    }

    output
}

/**
    AES-128-CBC decrypt (no auto-unpad, caller must call pkcs7_unpad).
*/
pub fn aes_cbc_decrypt(key: &[u8; 16], iv: &[u8], ciphertext: &[u8]) -> CdmResult<Vec<u8>> {
    if iv.len() != 16 || ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
        return Err(CdmError::AesCbcInvalidInput(
            "IV must be 16 bytes and ciphertext must be non-empty and block-aligned".into(),
        ));
    }

    let cipher = Aes128::new(key.into());
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

/**
    AES-128-ECB encrypt a single 16-byte block.

    Used for scalable license key derivation chain where AES-ECB
    encrypt is applied in multiple passes.
*/
pub fn aes_ecb_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(key.into());
    let mut out = aes::cipher::generic_array::GenericArray::from(*block);
    cipher.encrypt_block(&mut out);
    out.into()
}

/**
    Compute AES-128-CMAC tag (RFC 4493).

    Returns 16-byte CMAC tag over the given data.
*/
pub fn aes_cmac(key: &[u8; 16], data: &[u8]) -> [u8; 16] {
    let mut mac = <Cmac<Aes128> as Mac>::new_from_slice(key)
        .expect("CMAC key length is always valid for AES-128");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/**
    Verify AES-128-CMAC integrity tag.

    Computes the CMAC of `data` with the given `key` and compares it
    to the `expected` tag. Returns `CdmError::CmacMismatch` on failure.
*/
pub fn aes_cmac_verify(key: &[u8; 16], data: &[u8], expected: &[u8]) -> CdmResult<()> {
    let computed = aes_cmac(key, data);
    if computed.as_slice() == expected {
        Ok(())
    } else {
        Err(CdmError::CmacMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkcs7_round_trip() {
        for len in 0..=48 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let padded = pkcs7_pad(&data);
            assert!(padded.len().is_multiple_of(16));
            assert!(padded.len() > data.len());
            let unpadded = pkcs7_unpad(&padded).unwrap();
            assert_eq!(unpadded, data);
        }
    }

    #[test]
    fn pkcs7_exact_block_adds_full_block() {
        let data = [0u8; 16];
        let padded = pkcs7_pad(&data);
        assert_eq!(padded.len(), 32);
        assert!(padded[16..].iter().all(|&b| b == 16));
    }

    #[test]
    fn pkcs7_unpad_rejects_empty() {
        assert!(pkcs7_unpad(&[]).is_err());
    }

    #[test]
    fn pkcs7_unpad_rejects_bad_padding() {
        let mut block = [0u8; 16];
        block[15] = 4;
        block[14] = 4;
        block[13] = 4;
        block[12] = 99; // should be 4
        assert!(pkcs7_unpad(&block).is_err());
    }

    #[test]
    fn cbc_encrypt_decrypt_round_trip() {
        let key = [0x42u8; 16];
        let iv = [0x13u8; 16];
        let plaintext = b"hello world data";

        let encrypted = aes_cbc_encrypt(&key, &iv, plaintext);
        // Output starts with IV
        assert_eq!(&encrypted[..16], &iv);

        // Decrypt the ciphertext (skip IV prefix)
        let decrypted = aes_cbc_decrypt(&key, &iv, &encrypted[16..]).unwrap();
        let unpadded = pkcs7_unpad(&decrypted).unwrap();
        assert_eq!(unpadded, plaintext);
    }

    #[test]
    fn cbc_multi_block_round_trip() {
        let key = [0xAA; 16];
        let iv = [0xBB; 16];
        let data = b"this is more than sixteen bytes of plaintext data!!";

        let encrypted = aes_cbc_encrypt(&key, &iv, data);
        let decrypted = aes_cbc_decrypt(&key, &iv, &encrypted[16..]).unwrap();
        let unpadded = pkcs7_unpad(&decrypted).unwrap();
        assert_eq!(unpadded, data);
    }

    #[test]
    fn cbc_decrypt_bad_inputs() {
        let key = [0u8; 16];
        assert!(aes_cbc_decrypt(&key, &[0u8; 15], &[0u8; 16]).is_err());
        assert!(aes_cbc_decrypt(&key, &[0u8; 16], &[]).is_err());
        assert!(aes_cbc_decrypt(&key, &[0u8; 16], &[0u8; 17]).is_err());
    }

    #[test]
    fn ecb_encrypt_block_deterministic() {
        let key = [0x01u8; 16];
        let block = [0x02u8; 16];
        let a = aes_ecb_encrypt_block(&key, &block);
        let b = aes_ecb_encrypt_block(&key, &block);
        assert_eq!(a, b);
        assert_ne!(a, block); // actually encrypted
    }

    #[test]
    fn cmac_deterministic() {
        let key = [0x01u8; 16];
        let data = b"test data for cmac";
        let a = aes_cmac(&key, data);
        let b = aes_cmac(&key, data);
        assert_eq!(a, b);
    }

    #[test]
    fn cmac_verify_accepts_correct() {
        let key = [0x01u8; 16];
        let data = b"test data";
        let tag = aes_cmac(&key, data);
        aes_cmac_verify(&key, data, &tag).unwrap();
    }

    #[test]
    fn cmac_verify_rejects_wrong() {
        let key = [0x01u8; 16];
        let data = b"test data";
        let wrong_tag = [0u8; 16];
        assert!(aes_cmac_verify(&key, data, &wrong_tag).is_err());
    }
}
