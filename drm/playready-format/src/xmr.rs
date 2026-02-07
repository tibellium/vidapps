/*!
    XMR (eXtensible Media Rights) binary license format.

    XMR licenses use a TLV (Type-Length-Value) structure:
    - Magic: `XMR\x00`
    - Version: u32
    - Rights ID: 16 bytes
    - Nested TLV objects with flags, type (u16), length (u32)

    Key object types:
    - 0x000A: ContentKeyObject (key_id, key_type, cipher_type, encrypted_key)
    - 0x000B: SignatureObject (AES-CMAC signature)
    - 0x002A: EccKeyObject (device encryption public key)
    - 0x0051: AuxiliaryKeysObject (for scalable licenses)

    Container objects (flags & 0x02) contain nested children;
    leaf objects contain raw data fields.
*/
