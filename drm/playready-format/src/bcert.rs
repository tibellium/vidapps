/*!
    BCert (Binary Certificate) chain format.

    Structure:
    - Chain: `CHAI` magic + version (u32) + total_length (u32) + flags (u32) + cert_count (u32) + certs[]
    - Cert:  `CERT` magic + version (u32) + total_length (u32) + certificate_length (u32) + attributes[]
    - Attribute: flags (u16) + tag (u16) + length (u32, includes 8-byte header) + payload

    Attribute tags:
    - 0x01: BASIC (security level, cert type, public key digest, client ID, expiry)
    - 0x04: DEVICE (max HDCP, max digital/analog OPL)
    - 0x05: FEATURE (features bitfield)
    - 0x06: KEY (type + length + key bytes; encryption and signing keys)
    - 0x07: MANUFACTURER (name, make, model, null-padded)
    - 0x08: SIGNATURE (type + signature bytes; covers [0..certificate_length) of each cert)

    Chain validation: each cert is signed by the issuer above it; root cert must
    match the hardcoded Microsoft root public key (64-byte ECC P-256 point).
*/
