use prost::Message;
use rsa::{RsaPublicKey, pkcs1::DecodeRsaPublicKey, pss};
use sha1::Sha1;
use signature::Verifier;

use wdv3_proto::{
    DrmCertificate, EncryptedClientIdentification, SignedDrmCertificate, SignedMessage,
    signed_message::MessageType,
};

use crate::error::CdmError;

use super::aes::aes_cbc_encrypt;
use super::padding::pkcs7_pad;
use super::rsa::rsa_oaep_sha1_encrypt;

/**
    Encrypt the device's ClientIdentification for privacy mode.

    Privacy mode prevents the plaintext device identity from being visible to
    network observers or intermediary proxy servers.

    Cryptographic operations (two-step hybrid encryption):

      Step 1 — AES-128-CBC encryption of the ClientIdentification:
        privacy_key = random 16 bytes
        privacy_iv  = random 16 bytes
        plaintext   = pkcs7_pad(client_id_blob, block_size=16)
        ciphertext  = AES-128-CBC-encrypt(privacy_key, privacy_iv, plaintext)

      Step 2 — RSA-OAEP-SHA1 wrapping of the AES key:
        encrypted_privacy_key = RSA-OAEP-SHA1-encrypt(
            public_key = DrmCertificate.public_key,
            plaintext  = privacy_key
        )
*/
pub fn encrypt_client_id(
    client_id_blob: &[u8],
    service_certificate: &DrmCertificate,
) -> Result<EncryptedClientIdentification, CdmError> {
    let pub_key = service_certificate.public_key.as_deref().ok_or_else(|| {
        CdmError::CertificateDecode("missing public_key in DrmCertificate".into())
    })?;

    // Generate random AES key and IV
    let mut privacy_key = [0u8; 16];
    let mut privacy_iv = [0u8; 16];
    let mut rng = rand::rng();
    rand::RngCore::fill_bytes(&mut rng, &mut privacy_key);
    rand::RngCore::fill_bytes(&mut rng, &mut privacy_iv);

    // Step 1: pad and encrypt the client ID
    let padded = pkcs7_pad(client_id_blob, 16);
    let ciphertext = aes_cbc_encrypt(&privacy_key, &privacy_iv, &padded);

    // Step 2: RSA-OAEP wrap the AES key
    let encrypted_key = rsa_oaep_sha1_encrypt(pub_key, &privacy_key)?;

    Ok(EncryptedClientIdentification {
        provider_id: service_certificate.provider_id.clone(),
        service_certificate_serial_number: service_certificate.serial_number.clone(),
        encrypted_client_id: Some(ciphertext),
        encrypted_client_id_iv: Some(privacy_iv.to_vec()),
        encrypted_privacy_key: Some(encrypted_key),
    })
}

/**
    Verify and parse a service privacy certificate.

    The service certificate arrives as either:
      (a) A SignedMessage with type=SERVICE_CERTIFICATE, whose msg field contains
          a serialized SignedDrmCertificate, or
      (b) A direct serialized SignedDrmCertificate.
    Attempt (a) first, then fall back to (b).

    Verification:
      1. Parse the outer container to extract a SignedDrmCertificate.
      2. Verify the signature: RSA-PSS-SHA1-verify(
             public_key = root_public_key,
             message    = signed_drm_certificate.drm_certificate,
             signature  = signed_drm_certificate.signature
         )
         Parameters: Hash=SHA-1, MGF=MGF1-SHA-1, Salt=20 bytes.
      3. Parse signed_drm_certificate.drm_certificate as a DrmCertificate.
      4. Return the verified SignedDrmCertificate for storage on the Session.
*/
pub fn verify_service_certificate(
    certificate_bytes: &[u8],
    root_public_key: &[u8],
) -> Result<SignedDrmCertificate, CdmError> {
    // Try parsing as SignedMessage first, then fall back to direct SignedDrmCertificate
    let signed_cert = try_extract_signed_certificate(certificate_bytes)?;

    let cert_bytes = signed_cert
        .drm_certificate
        .as_deref()
        .ok_or_else(|| CdmError::CertificateDecode("missing drm_certificate field".into()))?;
    let sig_bytes = signed_cert
        .signature
        .as_deref()
        .ok_or_else(|| CdmError::CertificateDecode("missing signature field".into()))?;

    // Verify RSA-PSS-SHA1 signature against the root public key
    let public_key = RsaPublicKey::from_pkcs1_der(root_public_key)
        .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;
    let verifying_key = pss::VerifyingKey::<Sha1>::new_with_salt_len(public_key, 20);

    let signature = rsa::pss::Signature::try_from(sig_bytes)
        .map_err(|e| CdmError::RsaOperation(e.to_string()))?;

    verifying_key
        .verify(cert_bytes, &signature)
        .map_err(|_| CdmError::CertificateSignatureMismatch)?;

    Ok(signed_cert)
}

/**
    Try to extract a SignedDrmCertificate from raw bytes.

    Attempts to parse as a SignedMessage (type=SERVICE_CERTIFICATE) first,
    falling back to a direct SignedDrmCertificate.
*/
fn try_extract_signed_certificate(data: &[u8]) -> Result<SignedDrmCertificate, CdmError> {
    // Attempt (a): parse as SignedMessage whose msg contains a SignedDrmCertificate
    if let Ok(signed_msg) = SignedMessage::decode(data)
        && signed_msg.r#type == Some(MessageType::ServiceCertificate as i32)
        && let Some(msg) = &signed_msg.msg
    {
        return SignedDrmCertificate::decode(msg.as_slice())
            .map_err(|e| CdmError::CertificateDecode(e.to_string()));
    }

    // Attempt (b): parse directly as SignedDrmCertificate
    SignedDrmCertificate::decode(data).map_err(|e| CdmError::CertificateDecode(e.to_string()))
}
