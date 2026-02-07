/*!
    PlayReady cryptographic operations.

    All asymmetric crypto uses ECC P-256 (secp256r1):
    - ElGamal encryption/decryption for key exchange
    - ECDSA-SHA256 for challenge signing and license verification

    Symmetric crypto:
    - AES-128-CBC with PKCS7 padding for client data encryption
    - AES-128-ECB (no padding) for scalable license key derivation
    - AES-128-CMAC for license integrity verification
*/

mod aes;
mod elgamal;
mod signing;
