use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE64;
use p256::{
    ProjectivePoint, Scalar,
    elliptic_curve::{Field, rand_core::OsRng, sec1::ToEncodedPoint},
};
use sha2::{Digest, Sha256};

use drm_core::{ContentKey, KeyType, PsshBox};
use drm_playready_format::{
    key::CipherType,
    soap,
    wrm_header::{WrmHeader, WrmHeaderVersion, kid_to_uuid},
    xmr::XmrLicense,
};

use crate::constants::{MAGIC_CONSTANT_ZERO, WMRM_SERVER_KEY};
use crate::crypto::{aes, elgamal, signing};
use crate::device::Device;
use crate::error::{CdmError, CdmResult};
use crate::pssh_ext::PlayReadyExt;

/**
    Global session counter for monotonically-increasing session numbers.
*/
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

/**
    Ephemeral ECC session key material.
*/
struct XmlKey {
    /// The ECC keypair — private scalar (kept for potential future use).
    #[allow(dead_code)]
    private_key: [u8; 32],
    public_key: [u8; 64],
    /// AES-128 key derived from the x-coordinate: x[16..32].
    aes_key: [u8; 16],
    /// AES-128 IV derived from the x-coordinate: x[0..16].
    aes_iv: [u8; 16],
}

impl XmlKey {
    /**
        Generate a new random session key.
    */
    fn generate() -> Self {
        let scalar = Scalar::random(&mut OsRng);
        let point = (ProjectivePoint::GENERATOR * scalar).to_affine();
        let encoded = point.to_encoded_point(false);

        let mut private_key = [0u8; 32];
        private_key.copy_from_slice(&scalar.to_bytes());

        let mut public_key = [0u8; 64];
        public_key.copy_from_slice(&encoded.as_bytes()[1..65]);

        let x = encoded.x().expect("non-identity point");

        let mut aes_iv = [0u8; 16];
        aes_iv.copy_from_slice(&x[..16]);

        let mut aes_key = [0u8; 16];
        aes_key.copy_from_slice(&x[16..32]);

        Self {
            private_key,
            public_key,
            aes_key,
            aes_iv,
        }
    }
}

/**
    A PlayReady CDM session that builds license challenges and parses license responses.
*/
pub struct Session {
    /// Monotonically-increasing session number (for display/logging).
    number: u64,
    /// Parsed PRD device credentials.
    device: Device,
    /// Ephemeral session key (generated during challenge building).
    xml_key: Option<XmlKey>,
    /// Extracted content keys after a successful parse_license_response().
    content_keys: Vec<ContentKey>,
}

impl Session {
    /**
        Create a new session for the given device.
    */
    pub fn new(device: Device) -> Self {
        Session {
            number: SESSION_COUNTER.fetch_add(1, Ordering::Relaxed),
            device,
            xml_key: None,
            content_keys: Vec::new(),
        }
    }

    /**
        Session number (monotonically increasing across all sessions in the process).
    */
    pub fn number(&self) -> u64 {
        self.number
    }

    /**
        Build a license challenge (SOAP XML) for the given PSSH box.

        Returns the complete SOAP envelope as UTF-8 bytes, ready for HTTP POST
        to a PlayReady license server.
    */
    pub fn build_license_challenge(&mut self, pssh: &PsshBox) -> CdmResult<Vec<u8>> {
        // 1. Extract WRM header XML from PSSH
        let wrm_header_xml = pssh.playready_wrm_header_xml()?;
        let wrm_header =
            WrmHeader::from_xml(&wrm_header_xml).map_err(|e| CdmError::Format(e.to_string()))?;

        // 2. Determine protocol version from WRM header version
        let protocol_version = match wrm_header.version {
            WrmHeaderVersion::V4_3_0_0 => 5,
            WrmHeaderVersion::V4_2_0_0 => 4,
            _ => 1,
        };

        // 3. Generate session key
        let xml_key = XmlKey::generate();

        // 4. ElGamal encrypt session public point to WMRM server key
        let wrmserver_data = elgamal::ecc256_encrypt(&WMRM_SERVER_KEY, &xml_key.public_key)?;

        // 5. Build encrypted client data
        let client_data_xml = build_client_data_xml(&self.device.group_certificate);
        let encrypted_client_data =
            aes::aes_cbc_encrypt(&xml_key.aes_key, &xml_key.aes_iv, &client_data_xml);

        // 6. Generate nonce and timestamp
        let mut nonce = [0u8; 16];
        {
            use p256::elliptic_curve::rand_core::RngCore;
            OsRng.fill_bytes(&mut nonce);
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 7. Build the <LA> element
        let la_xml = build_la_element(
            protocol_version,
            &wrm_header_xml,
            &nonce,
            timestamp,
            &wrmserver_data,
            &encrypted_client_data,
        );

        // 8. SHA-256 hash the LA element
        let la_digest = Sha256::digest(la_xml.as_bytes());

        // 9. Build <SignedInfo> and sign it
        let signed_info_xml = build_signed_info_element(&la_digest);
        let signature = signing::ecdsa_sha256_sign(
            &self.device.signing_key.private_key,
            signed_info_xml.as_bytes(),
        )?;

        // 10. Assemble full SOAP envelope
        let soap_envelope = build_soap_envelope(
            &la_xml,
            &signed_info_xml,
            &signature,
            self.device.signing_public_key(),
        );

        // Store session key
        self.xml_key = Some(xml_key);

        Ok(soap_envelope.into_bytes())
    }

    /**
        Parse a license response and extract content keys.

        Takes the raw SOAP XML bytes received from the license server.
        Returns the extracted content keys on success.
    */
    pub fn parse_license_response(&mut self, raw: &[u8]) -> CdmResult<&[ContentKey]> {
        let response_str =
            std::str::from_utf8(raw).map_err(|e| CdmError::InvalidXml(e.to_string()))?;

        // 1. Parse SOAP response and extract license blobs
        let license_blobs = extract_license_blobs(response_str)?;

        if license_blobs.is_empty() {
            return Err(CdmError::NoContentKeys);
        }

        // 2. Process each license blob
        let mut keys = Vec::new();
        for blob_b64 in &license_blobs {
            let blob = BASE64
                .decode(blob_b64.as_bytes())
                .map_err(|e| CdmError::InvalidBase64(e.to_string()))?;

            let xmr = XmrLicense::from_bytes(&blob).map_err(|e| CdmError::Format(e.to_string()))?;

            // 3. Verify device key matches
            if let Some(ecc_key) = xmr.find_ecc_key()
                && ecc_key.key.as_slice() != self.device.encryption_public_key().as_slice()
            {
                return Err(CdmError::DeviceKeyMismatch);
            }

            // 4. Extract content keys
            for ck_obj in xmr.find_content_keys() {
                keys.push(extract_content_key(ck_obj, &xmr, &self.device)?);
            }
        }

        if keys.is_empty() {
            return Err(CdmError::NoContentKeys);
        }

        self.content_keys = keys;
        Ok(&self.content_keys)
    }

    /**
        Returns all extracted keys (empty until `parse_license_response` succeeds).
    */
    pub fn keys(&self) -> &[ContentKey] {
        &self.content_keys
    }

    /**
        Returns only content keys (`KeyType::Content`).
    */
    pub fn content_keys(&self) -> Vec<&ContentKey> {
        self.keys_of_type(KeyType::Content)
    }

    /**
        Returns keys matching the given type.
    */
    pub fn keys_of_type(&self, key_type: KeyType) -> Vec<&ContentKey> {
        self.content_keys
            .iter()
            .filter(|k| k.key_type == key_type)
            .collect()
    }

    /**
        Look up a key by its 16-byte key ID.
    */
    pub fn key_by_kid(&self, kid: [u8; 16]) -> Option<&ContentKey> {
        self.content_keys.iter().find(|k| k.kid == kid)
    }
}

/**
    Build the client data XML containing the certificate chain and features.
*/
fn build_client_data_xml(group_certificate: &[u8]) -> Vec<u8> {
    let cert_b64 = BASE64.encode(group_certificate);
    let xml = format!(
        "<Data>\
<CertificateChains>\
<CertificateChain> {cert_b64} </CertificateChain>\
</CertificateChains>\
<Features>\
<Feature Name=\"AESCBC\"></Feature>\
<REE>\
<AESCBCS></AESCBCS>\
</REE>\
</Features>\
</Data>"
    );
    xml.into_bytes()
}

/**
    Build the `<LA>` element of the challenge.
*/
fn build_la_element(
    protocol_version: u32,
    wrm_header_xml: &str,
    nonce: &[u8; 16],
    timestamp: u64,
    wrmserver_data: &[u8; 128],
    encrypted_client_data: &[u8],
) -> String {
    let nonce_b64 = BASE64.encode(nonce);
    let wrmserver_b64 = BASE64.encode(wrmserver_data);
    let client_data_b64 = BASE64.encode(encrypted_client_data);

    format!(
        "<LA xmlns=\"{protocol_ns}\" Id=\"SignedData\" xml:space=\"preserve\">\
<Version>{protocol_version}</Version>\
<ContentHeader>{wrm_header_xml}</ContentHeader>\
<CLIENTINFO>\
<CLIENTVERSION>{client_version}</CLIENTVERSION>\
</CLIENTINFO>\
<LicenseNonce>{nonce_b64}</LicenseNonce>\
<ClientTime>{timestamp}</ClientTime>\
<EncryptedData xmlns=\"{xmlenc_ns}\" Type=\"{xmlenc_ns}Element\">\
<EncryptionMethod Algorithm=\"{aes_algorithm}\"></EncryptionMethod>\
<KeyInfo xmlns=\"{xmldsig_ns}\">\
<EncryptedKey xmlns=\"{xmlenc_ns}\">\
<EncryptionMethod Algorithm=\"{ecc_algorithm}\"></EncryptionMethod>\
<KeyInfo xmlns=\"{xmldsig_ns}\">\
<KeyName>WMRMServer</KeyName>\
</KeyInfo>\
<CipherData>\
<CipherValue>{wrmserver_b64}</CipherValue>\
</CipherData>\
</EncryptedKey>\
</KeyInfo>\
<CipherData>\
<CipherValue>{client_data_b64}</CipherValue>\
</CipherData>\
</EncryptedData>\
</LA>",
        protocol_ns = soap::PROTOCOL_NS,
        client_version = soap::CLIENT_VERSION,
        xmlenc_ns = soap::XMLENC_NS,
        xmldsig_ns = soap::XMLDSIG_NS,
        aes_algorithm = soap::AES128_CBC_ALGORITHM,
        ecc_algorithm = soap::ECC256_ALGORITHM,
    )
}

/**
    Build the `<SignedInfo>` element referencing the LA digest.
*/
fn build_signed_info_element(la_digest: &[u8]) -> String {
    let digest_b64 = BASE64.encode(la_digest);
    format!(
        "<SignedInfo xmlns=\"{xmldsig_ns}\">\
<CanonicalizationMethod Algorithm=\"{c14n_algorithm}\"></CanonicalizationMethod>\
<SignatureMethod Algorithm=\"{ecdsa_algorithm}\"></SignatureMethod>\
<Reference URI=\"#SignedData\">\
<DigestMethod Algorithm=\"{sha256_algorithm}\"></DigestMethod>\
<DigestValue>{digest_b64}</DigestValue>\
</Reference>\
</SignedInfo>",
        xmldsig_ns = soap::XMLDSIG_NS,
        c14n_algorithm = soap::C14N_ALGORITHM,
        ecdsa_algorithm = soap::ECDSA_SHA256_ALGORITHM,
        sha256_algorithm = soap::SHA256_ALGORITHM,
    )
}

/**
    Assemble the complete SOAP envelope.
*/
fn build_soap_envelope(
    la_xml: &str,
    signed_info_xml: &str,
    signature: &[u8; 64],
    signing_public_key: &[u8; 64],
) -> String {
    let signature_b64 = BASE64.encode(signature);
    let pubkey_b64 = BASE64.encode(signing_public_key);

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<soap:Envelope \
xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" \
xmlns:soap=\"{soap_ns}\">\
<soap:Body>\
<AcquireLicense xmlns=\"{protocol_ns}\">\
<challenge>\
<Challenge xmlns=\"{message_ns}\">\
{la_xml}\
<Signature xmlns=\"{xmldsig_ns}\">\
{signed_info_xml}\
<SignatureValue>{signature_b64}</SignatureValue>\
<KeyInfo xmlns=\"{xmldsig_ns}\">\
<KeyValue>\
<ECCKeyValue>\
<PublicKey>{pubkey_b64}</PublicKey>\
</ECCKeyValue>\
</KeyValue>\
</KeyInfo>\
</Signature>\
</Challenge>\
</challenge>\
</AcquireLicense>\
</soap:Body>\
</soap:Envelope>",
        soap_ns = soap::SOAP_NS,
        protocol_ns = soap::PROTOCOL_NS,
        message_ns = soap::MESSAGE_NS,
        xmldsig_ns = soap::XMLDSIG_NS,
    )
}

/**
    Extract base64-encoded license blobs from a SOAP license response.
*/
fn extract_license_blobs(xml: &str) -> CdmResult<Vec<String>> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);

    // Check for SOAP faults first
    check_soap_fault(xml)?;

    let mut licenses = Vec::new();
    let mut in_license = false;
    let mut depth = 0;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if local == b"License" {
                    in_license = true;
                    depth = 1;
                } else if in_license {
                    depth += 1;
                }
            }
            Ok(Event::End(_)) => {
                if in_license {
                    depth -= 1;
                    if depth == 0 {
                        in_license = false;
                    }
                }
            }
            Ok(Event::Text(e)) if in_license && depth == 1 => {
                let text = e
                    .unescape()
                    .map_err(|e| CdmError::InvalidXml(e.to_string()))?;
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    licenses.push(trimmed.to_string());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(CdmError::InvalidXml(e.to_string())),
            _ => {}
        }
    }

    Ok(licenses)
}

/**
    Check for SOAP faults in the response XML.
*/
fn check_soap_fault(xml: &str) -> CdmResult<()> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut in_fault = false;
    let mut in_faultstring = false;
    let mut fault_message = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if local == b"Fault" {
                    in_fault = true;
                } else if in_fault && (local == b"faultstring" || local == b"Text") {
                    in_faultstring = true;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if local == b"Fault" {
                    in_fault = false;
                } else if local == b"faultstring" || local == b"Text" {
                    in_faultstring = false;
                }
            }
            Ok(Event::Text(e)) if in_faultstring => {
                if let Ok(text) = e.unescape() {
                    fault_message = Some(text.to_string());
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    if let Some(msg) = fault_message {
        return Err(CdmError::SoapFault(msg));
    }

    Ok(())
}

/**
    Extract the local name from a possibly namespace-prefixed tag.
*/
fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}

/**
    Extract a content key from an XMR ContentKeyObject.
*/
fn extract_content_key(
    ck_obj: &drm_playready_format::xmr::ContentKeyObject,
    xmr: &XmrLicense,
    device: &Device,
) -> CdmResult<ContentKey> {
    match ck_obj.cipher_type {
        CipherType::Ecc256 | CipherType::Ecc256WithKz => extract_standard_key(ck_obj, xmr, device),
        CipherType::Ecc256ViaSymmetric => extract_scalable_key(ck_obj, xmr, device),
        other => Err(CdmError::UnsupportedCipherType(other.to_string())),
    }
}

/**
    Standard key extraction: ElGamal decrypt → split into CI and CK.
*/
fn extract_standard_key(
    ck_obj: &drm_playready_format::xmr::ContentKeyObject,
    xmr: &XmrLicense,
    device: &Device,
) -> CdmResult<ContentKey> {
    // ElGamal decrypt
    let decrypted =
        elgamal::ecc256_decrypt(&device.encryption_key.private_key, &ck_obj.encrypted_key)?;

    // Split: CI = first 16, CK = last 16
    let integrity_key: [u8; 16] = decrypted[..16].try_into().unwrap();
    let content_key: [u8; 16] = decrypted[16..32].try_into().unwrap();

    // Verify license integrity via AES-CMAC
    verify_license_integrity(xmr, &integrity_key)?;

    // Convert PlayReady GUID key_id to standard UUID byte order
    let kid = kid_to_uuid(&ck_obj.key_id);

    Ok(ContentKey {
        kid,
        key: content_key.to_vec(),
        key_type: KeyType::Content,
    })
}

/**
    Scalable key extraction: ElGamal decrypt → interleaved split → AES-ECB chain.
*/
fn extract_scalable_key(
    ck_obj: &drm_playready_format::xmr::ContentKeyObject,
    xmr: &XmrLicense,
    device: &Device,
) -> CdmResult<ContentKey> {
    if ck_obj.encrypted_key.len() < 144 {
        return Err(CdmError::Format(format!(
            "scalable license encrypted_key too short: {} bytes",
            ck_obj.encrypted_key.len()
        )));
    }

    // ElGamal decrypt first 128 bytes
    let decrypted =
        elgamal::ecc256_decrypt(&device.encryption_key.private_key, &ck_obj.encrypted_key)?;

    // Interleaved byte split: even-indexed → CI, odd-indexed → CK
    let mut ci = [0u8; 16];
    let mut ck = [0u8; 16];
    for i in 0..16 {
        ci[i] = decrypted[i * 2];
        ck[i] = decrypted[i * 2 + 1];
    }

    // AES-ECB derivation chain
    // Step 1: rgb_key = CK XOR MAGIC_CONSTANT_ZERO
    let mut rgb_key = [0u8; 16];
    for i in 0..16 {
        rgb_key[i] = ck[i] ^ MAGIC_CONSTANT_ZERO[i];
    }

    // Step 2: content_key_prime = AES-ECB-encrypt(CK, rgb_key)
    let content_key_prime = aes::aes_ecb_encrypt_block(&ck, &rgb_key);

    // Step 3: Get auxiliary key
    let aux_keys = xmr
        .find_auxiliary_keys()
        .ok_or_else(|| CdmError::Format("scalable license missing AuxKeyObject".into()))?;
    let aux_key = aux_keys
        .keys
        .first()
        .ok_or_else(|| CdmError::Format("AuxKeyObject has no keys".into()))?;

    // Step 4: uplink_x_key = AES-ECB-encrypt(content_key_prime, aux_key)
    let uplink_x_key = aes::aes_ecb_encrypt_block(&content_key_prime, &aux_key.key);

    // Step 5: secondary_key = AES-ECB-encrypt(CK, embedded_root_license[128..144])
    let secondary_block: [u8; 16] = ck_obj.encrypted_key[128..144].try_into().unwrap();
    let secondary_key = aes::aes_ecb_encrypt_block(&ck, &secondary_block);

    // Step 6: Decrypt embedded leaf license (two AES-ECB passes)
    let embedded_leaf = &ck_obj.encrypted_key[144..];
    if embedded_leaf.len() < 32 {
        return Err(CdmError::Format(format!(
            "embedded leaf license too short: {} bytes",
            embedded_leaf.len()
        )));
    }

    // Process in 16-byte blocks.
    // NOTE: AES-ECB *encrypt* is used intentionally here. The server encrypted
    // using AES-ECB decrypt, so the client "decrypts" by encrypting. For single-
    // block operations AES-ECB is its own inverse in this protocol's usage.
    let mut result = Vec::with_capacity(embedded_leaf.len());
    for chunk in embedded_leaf.chunks(16) {
        if chunk.len() == 16 {
            let block: [u8; 16] = chunk.try_into().unwrap();
            let pass1 = aes::aes_ecb_encrypt_block(&uplink_x_key, &block);
            let pass2 = aes::aes_ecb_encrypt_block(&secondary_key, &pass1);
            result.extend_from_slice(&pass2);
        } else {
            result.extend_from_slice(chunk);
        }
    }

    // Final split: CI = first 16, CK = next 16
    if result.len() < 32 {
        return Err(CdmError::Format("decrypted leaf too short".into()));
    }
    let final_ci: [u8; 16] = result[..16].try_into().unwrap();
    let final_ck: [u8; 16] = result[16..32].try_into().unwrap();

    // Verify license integrity
    verify_license_integrity(xmr, &final_ci)?;

    let kid = kid_to_uuid(&ck_obj.key_id);

    Ok(ContentKey {
        kid,
        key: final_ck.to_vec(),
        key_type: KeyType::Content,
    })
}

/**
    Verify XMR license integrity using AES-CMAC.
*/
fn verify_license_integrity(xmr: &XmrLicense, integrity_key: &[u8; 16]) -> CdmResult<()> {
    let sig_obj = xmr.find_signature().ok_or(CdmError::IntegrityCheckFailed)?;

    let message = xmr
        .signature_message_bytes()
        .ok_or(CdmError::IntegrityCheckFailed)?;

    aes::aes_cmac_verify(integrity_key, message, &sig_obj.signature_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_key_generation() {
        let key = XmlKey::generate();
        // Private key should not be all zeros
        assert_ne!(key.private_key, [0u8; 32]);
        // Public key should not be all zeros
        assert_ne!(key.public_key, [0u8; 64]);
        // AES key and IV should be derived from x-coordinate
        assert_ne!(key.aes_key, [0u8; 16]);
    }

    #[test]
    fn session_numbers_are_monotonic() {
        // Use a simple device stub — we only test session numbering
        // Real device tests require a valid PRD file
        let n1 = SESSION_COUNTER.load(Ordering::Relaxed);
        let n2 = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        assert!(n2 >= n1);
    }

    #[test]
    fn local_name_no_prefix() {
        assert_eq!(local_name(b"License"), b"License");
    }

    #[test]
    fn local_name_with_prefix() {
        assert_eq!(local_name(b"soap:Body"), b"Body");
        assert_eq!(
            local_name(b"ns2:AcquireLicenseResponse"),
            b"AcquireLicenseResponse"
        );
    }

    #[test]
    fn build_client_data_produces_valid_xml() {
        let cert = b"test certificate data";
        let xml = build_client_data_xml(cert);
        let xml_str = std::str::from_utf8(&xml).unwrap();
        assert!(xml_str.contains("<CertificateChain>"));
        assert!(xml_str.contains("</CertificateChain>"));
        assert!(xml_str.contains("<Feature Name=\"AESCBC\">"));
        assert!(xml_str.contains("<AESCBCS>"));
        // Certificate should be base64-encoded with surrounding spaces
        let cert_b64 = BASE64.encode(cert);
        assert!(xml_str.contains(&format!(" {cert_b64} ")));
    }

    #[test]
    fn build_la_element_includes_all_fields() {
        let nonce = [0xAA; 16];
        let wrmserver = [0xBB; 128];
        let client_data = vec![0xCC; 32];
        let la = build_la_element(
            5,
            "<WRMHEADER/>",
            &nonce,
            1700000000,
            &wrmserver,
            &client_data,
        );

        assert!(la.contains("<Version>5</Version>"));
        assert!(la.contains("<ContentHeader><WRMHEADER/></ContentHeader>"));
        assert!(la.contains(&format!(
            "<CLIENTVERSION>{}</CLIENTVERSION>",
            soap::CLIENT_VERSION
        )));
        assert!(la.contains("<LicenseNonce>"));
        assert!(la.contains("<ClientTime>1700000000</ClientTime>"));
        assert!(la.contains("WMRMServer"));
        assert!(la.contains(soap::ECC256_ALGORITHM));
        assert!(la.contains(soap::AES128_CBC_ALGORITHM));
    }

    #[test]
    fn build_signed_info_includes_digest() {
        let digest = [0xDD; 32];
        let si = build_signed_info_element(&digest);
        assert!(si.contains(soap::C14N_ALGORITHM));
        assert!(si.contains(soap::ECDSA_SHA256_ALGORITHM));
        assert!(si.contains(soap::SHA256_ALGORITHM));
        assert!(si.contains("<DigestValue>"));
        assert!(si.contains("#SignedData"));
    }

    #[test]
    fn build_soap_envelope_structure() {
        let la = "<LA>test</LA>";
        let si = "<SignedInfo>test</SignedInfo>";
        let sig = [0xEE; 64];
        let pk = [0xFF; 64];
        let envelope = build_soap_envelope(la, si, &sig, &pk);

        assert!(envelope.starts_with("<?xml version=\"1.0\""));
        assert!(envelope.contains("soap:Envelope"));
        assert!(envelope.contains("soap:Body"));
        assert!(envelope.contains("<AcquireLicense"));
        assert!(envelope.contains("<Challenge"));
        assert!(envelope.contains("<LA>test</LA>"));
        assert!(envelope.contains("<SignatureValue>"));
        assert!(envelope.contains("<ECCKeyValue>"));
        assert!(envelope.contains("<PublicKey>"));
        assert!(envelope.contains("</soap:Envelope>"));
    }

    #[test]
    fn check_soap_fault_no_fault() {
        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body><Response>OK</Response></soap:Body></soap:Envelope>"#;
        check_soap_fault(xml).unwrap();
    }

    #[test]
    fn check_soap_fault_detects_fault() {
        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body><soap:Fault><faultstring>Access denied</faultstring></soap:Fault></soap:Body></soap:Envelope>"#;
        let err = check_soap_fault(xml).unwrap_err();
        match err {
            CdmError::SoapFault(msg) => assert!(msg.contains("Access denied")),
            other => panic!("expected SoapFault, got {other:?}"),
        }
    }

    #[test]
    fn extract_licenses_from_response() {
        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <AcquireLicenseResponse>
                    <AcquireLicenseResult>
                        <Response>
                            <LicenseResponse>
                                <Licenses>
                                    <License>AQID</License>
                                    <License>BAUG</License>
                                </Licenses>
                            </LicenseResponse>
                        </Response>
                    </AcquireLicenseResult>
                </AcquireLicenseResponse>
            </soap:Body>
        </soap:Envelope>"#;

        let blobs = extract_license_blobs(xml).unwrap();
        assert_eq!(blobs.len(), 2);
        assert_eq!(blobs[0], "AQID");
        assert_eq!(blobs[1], "BAUG");
    }
}
