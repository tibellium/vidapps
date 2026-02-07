#![allow(dead_code)]

use hex_literal::hex;

/**
    WMRM Server ECC P-256 public key (64 bytes: X || Y).

    Used to ElGamal-encrypt the session key so only the license server
    can decrypt it and recover the AES key/IV for the client data.
*/
pub const WMRM_SERVER_KEY: [u8; 64] = hex!(
    // X coordinate (32 bytes)
    "c8b6af16ee941aadaa5389b4af2c10e3"
    "56be42af175ef3face93254e7b0b3d9b"
    // Y coordinate (32 bytes)
    "982b27b5cb2341326e56aa857dbfd5c6"
    "34ce2cf9ea74fca8f2af5957efeea562"
);

/**
    Microsoft PlayReady root issuer ECC P-256 public key (64 bytes: X || Y).

    All legitimate PlayReady BCert certificate chains must terminate at
    a root certificate whose signing key matches this public key.
*/
pub const MS_PLAYREADY_ROOT_ISSUER_KEY: [u8; 64] = hex!(
    // X coordinate (32 bytes)
    "864d61cff2256e422c568b3c28001cfb"
    "3e1527658584ba0521b79b1828d936de"
    // Y coordinate (32 bytes)
    "1d826a8fc3e6e7fa7a90d5ca2946f1f6"
    "4a2efb9f5dcffe7e434eb44293fac5ab"
);

/**
    Magic constant for scalable license key derivation.

    XORed with the content key during the multi-step AES-ECB
    key derivation chain for ECC_256_VIA_SYMMETRIC cipher types.
*/
pub const MAGIC_CONSTANT_ZERO: [u8; 16] = hex!("7ee9ed4af773224f00b8ea7efb027cbb");

/**
    Key derivation label for device provisioning key unwrap.

    Used as the label in NIST SP 800-108 counter-mode KDF (AES-CMAC PRF)
    when deriving the wrapping key for device private key unwrapping.
*/
pub const KD_CERT_PRIV_KEYS_WRAP: [u8; 16] = hex!("9ce93432c7d74016ba684763f801e136");

/**
    Base key for CMAC-based KDF in device provisioning.

    Used as the AES-CMAC key in NIST SP 800-108 counter-mode KDF
    when deriving the wrapping key for device private key unwrapping.
*/
pub const CTK_TEST: [u8; 16] = hex!("8b222ffd1e76195659cf2703898c427f");
