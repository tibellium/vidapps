/*!
    Certificate chain verification.

    Verifies a PlayReady BCert chain by walking the certificates from leaf
    to root, checking ECDSA-SHA256 signatures, adjacent-cert key linkage,
    and root issuer key against the Microsoft PlayReady root public key.
*/

use drm_playready_format::bcert::{BCertChain, CertType};

use crate::constants::MS_PLAYREADY_ROOT_ISSUER_KEY;
use crate::crypto::signing;
use crate::error::{CdmError, CdmResult};

/**
    Verify a PlayReady BCert certificate chain.

    Checks:
    1. Chain length is 1..=6 certificates.
    2. Each certificate's ECDSA-SHA256 signature is valid against its
       embedded issuer public key.
    3. Adjacent certificates are linked: the child's issuer key (from
       `SignatureInfo.signing_key`) matches a key in the parent's `KeyInfo`.
    4. The root certificate's issuer key matches the Microsoft PlayReady
       root issuer public key.
*/
pub(crate) fn verify_chain(chain: &BCertChain) -> CdmResult<()> {
    let count = chain.certificates.len();
    if count == 0 || count > 6 {
        return Err(CdmError::CertificateChainInvalid(format!(
            "chain must have 1-6 certificates, got {count}"
        )));
    }

    for i in 0..count {
        let cert = &chain.certificates[i];

        // Get signature info
        let sig_info = cert.signature_info().ok_or_else(|| {
            CdmError::CertificateChainInvalid(format!(
                "certificate {i} has no SignatureInfo attribute"
            ))
        })?;

        // Verify the certificate's own signature
        let signing_key: [u8; 64] = sig_info.signing_key.as_slice().try_into().map_err(|_| {
            CdmError::CertificateChainInvalid(format!(
                "certificate {i} signing key is not 64 bytes (got {})",
                sig_info.signing_key.len()
            ))
        })?;

        signing::ecdsa_sha256_verify(&signing_key, cert.signed_bytes(), &sig_info.signature)
            .map_err(|_| {
                CdmError::CertificateChainInvalid(format!(
                    "certificate {i} signature verification failed"
                ))
            })?;

        // Verify adjacent cert linkage: child's issuer key must be in parent's KeyInfo
        if i > 0 {
            let parent = &chain.certificates[i];
            let child = &chain.certificates[i - 1];

            // Parent must be an issuer cert
            if let Some(info) = parent.basic_info()
                && let Some(ct) = CertType::from_u32(info.cert_type)
                && ct != CertType::Issuer
                && i < count - 1
            {
                // Non-root intermediate must be Issuer type
                return Err(CdmError::CertificateChainInvalid(format!(
                    "certificate {i} is not an Issuer type"
                )));
            }

            // Child's issuer key must match one of parent's keys
            let child_sig = child.signature_info().ok_or_else(|| {
                CdmError::CertificateChainInvalid(format!(
                    "certificate {} has no SignatureInfo attribute",
                    i - 1
                ))
            })?;

            let parent_ki = parent.key_info().ok_or_else(|| {
                CdmError::CertificateChainInvalid(format!(
                    "certificate {i} has no KeyInfo attribute"
                ))
            })?;
            let issuer_key_matches = parent_ki
                .keys
                .iter()
                .any(|k| k.key == child_sig.signing_key);
            if !issuer_key_matches {
                return Err(CdmError::CertificateChainInvalid(format!(
                    "certificate {} issuer key not found in certificate {i} KeyInfo",
                    i - 1
                )));
            }
        }

        // Root certificate: issuer key must match Microsoft root key
        if i == count - 1
            && sig_info.signing_key.as_slice() != MS_PLAYREADY_ROOT_ISSUER_KEY.as_slice()
        {
            return Err(CdmError::CertificateChainInvalid(
                "root certificate issuer key does not match Microsoft PlayReady root key".into(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_test_device_chain() {
        let prd_data = include_bytes!("../testfiles/device.prd");
        let device = crate::Device::from_bytes(prd_data).unwrap();
        let chain = device.group_certificate_chain().unwrap();
        verify_chain(&chain).unwrap();
    }

    #[test]
    fn verify_v2_device_chain() {
        let prd_data = include_bytes!("../testfiles/device_v2.prd");
        let device = crate::Device::from_bytes(prd_data).unwrap();
        let chain = device.group_certificate_chain().unwrap();
        verify_chain(&chain).unwrap();
    }

    #[test]
    fn empty_chain_fails() {
        let chain = BCertChain {
            version: 1,
            flags: 0,
            certificates: vec![],
        };
        let err = verify_chain(&chain).unwrap_err();
        assert!(matches!(err, CdmError::CertificateChainInvalid(_)));
    }
}
