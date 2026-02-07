/*!
    PRD (PlayReady Device) file format.

    Magic: `PRD` (3 bytes) + version (u8)

    Version 3 (current):
    - group_key: 96 bytes (32B private + 64B public ECC P-256)
    - encryption_key: 96 bytes (32B private + 64B public)
    - signing_key: 96 bytes (32B private + 64B public)
    - group_certificate_length: u32 BE
    - group_certificate: BCert chain bytes

    Version 2:
    - group_certificate_length (u32 BE) + group_certificate first
    - encryption_key: 96 bytes
    - signing_key: 96 bytes
    - No group_key field

    ECC keys are serialized as: private_scalar (32B big-endian)
    followed by uncompressed public point X (32B) || Y (32B).
*/
