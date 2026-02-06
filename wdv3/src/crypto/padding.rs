use crate::error::CdmError;

/// Remove PKCS#7 padding from a decrypted AES-CBC plaintext.
///
/// The last byte indicates the number of padding bytes (1-16).
/// All padding bytes must have the same value as the last byte.
/// Returns the unpadded data, or CdmError::Pkcs7PaddingInvalid if the padding is malformed.
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

/// Apply PKCS#7 padding to plaintext before AES-CBC encryption.
/// Used by encrypt_client_id() to pad the serialized ClientIdentification
/// before AES-128-CBC encryption.
///
/// Appends 1-16 bytes, each with the value of the padding length.
pub fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let pad = block_size - (data.len() % block_size);
    let mut out = Vec::with_capacity(data.len() + pad);
    out.extend_from_slice(data);
    out.resize(data.len() + pad, pad as u8);
    out
}
