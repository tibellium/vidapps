use crate::error::CdmError;

/**
    Remove PKCS#7 padding from a decrypted AES-CBC plaintext.

    The last byte indicates the number of padding bytes (1-16).
    All padding bytes must have the same value as the last byte.
    Returns the unpadded data, or CdmError::Pkcs7PaddingInvalid if the padding is malformed.
*/
pub fn pkcs7_unpad(data: &[u8], block_size: usize) -> Result<Vec<u8>, CdmError> {
    if data.is_empty() || !data.len().is_multiple_of(block_size) {
        return Err(CdmError::Pkcs7PaddingInvalid);
    }

    let pad = data[data.len() - 1] as usize;
    if pad == 0 || pad > block_size || pad > data.len() {
        return Err(CdmError::Pkcs7PaddingInvalid);
    }

    // Verify all padding bytes have the correct value
    for &byte in &data[data.len() - pad..] {
        if byte as usize != pad {
            return Err(CdmError::Pkcs7PaddingInvalid);
        }
    }

    Ok(data[..data.len() - pad].to_vec())
}

/**
    Apply PKCS#7 padding to plaintext before AES-CBC encryption.
    Used by encrypt_client_id() to pad the serialized ClientIdentification
    before AES-128-CBC encryption.

    Appends 1-16 bytes, each with the value of the padding length.
*/
pub fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let pad = block_size - (data.len() % block_size);
    let mut out = Vec::with_capacity(data.len() + pad);
    out.extend_from_slice(data);
    out.resize(data.len() + pad, pad as u8);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_unpad_round_trip() {
        for len in 0..=48 {
            let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let padded = pkcs7_pad(&data, 16);
            assert!(padded.len().is_multiple_of(16));
            assert!(padded.len() > data.len());
            let unpadded = pkcs7_unpad(&padded, 16).unwrap();
            assert_eq!(unpadded, data);
        }
    }

    #[test]
    fn pad_exact_block_adds_full_block() {
        // When data is exactly block-aligned, a full block of padding is added
        let data = [0u8; 16];
        let padded = pkcs7_pad(&data, 16);
        assert_eq!(padded.len(), 32);
        assert!(padded[16..].iter().all(|&b| b == 16));
    }

    #[test]
    fn unpad_empty_fails() {
        let err = pkcs7_unpad(&[], 16).unwrap_err();
        assert!(matches!(err, CdmError::Pkcs7PaddingInvalid));
    }

    #[test]
    fn unpad_bad_pad_value_zero() {
        let mut block = [0u8; 16];
        block[15] = 0; // pad value 0 is invalid
        let err = pkcs7_unpad(&block, 16).unwrap_err();
        assert!(matches!(err, CdmError::Pkcs7PaddingInvalid));
    }

    #[test]
    fn unpad_bad_pad_value_too_large() {
        let mut block = [0u8; 16];
        block[15] = 17; // larger than block size
        let err = pkcs7_unpad(&block, 16).unwrap_err();
        assert!(matches!(err, CdmError::Pkcs7PaddingInvalid));
    }

    #[test]
    fn unpad_inconsistent_padding() {
        // Last byte says 4, but not all 4 trailing bytes match
        let mut block = [0u8; 16];
        block[15] = 4;
        block[14] = 4;
        block[13] = 4;
        block[12] = 99; // should be 4
        let err = pkcs7_unpad(&block, 16).unwrap_err();
        assert!(matches!(err, CdmError::Pkcs7PaddingInvalid));
    }

    #[test]
    fn unpad_not_block_aligned() {
        let err = pkcs7_unpad(&[0u8; 15], 16).unwrap_err();
        assert!(matches!(err, CdmError::Pkcs7PaddingInvalid));
    }
}
