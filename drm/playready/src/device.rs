/*!
    PlayReady device (PRD file) loading and management.

    A device contains:
    - Group key (ECC P-256 keypair) — for provisioning
    - Encryption key (ECC P-256 keypair) — for ElGamal key exchange
    - Signing key (ECC P-256 keypair) — for ECDSA challenge signing
    - Group certificate chain (BCert)

    Supports PRD format versions 2 and 3.
*/
