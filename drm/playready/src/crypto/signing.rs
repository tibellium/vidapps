/*!
    ECDSA-SHA256 signing and verification on P-256.

    - Sign: SHA256(data) → ECDsaSigner.sign(hash) → raw R||S (64 bytes)
    - Verify: SHA256(data) → ECDsaSigner.verify(hash, R, S)

    Signatures use the raw 64-byte R||S format (not DER-encoded).
    Each component is 32 bytes, big-endian, zero-padded.
*/
