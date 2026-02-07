/*!
    PlayReady key and cipher type enums.

    These define the content key encryption algorithm and the key wrapping
    cipher used in XMR licenses.

    Key types (content encryption algorithm):
    - Invalid (0x0000)
    - Aes128Ctr (0x0001) — most common
    - Rc4Cipher (0x0002) — legacy
    - Aes128Ecb (0x0003)
    - Cocktail (0x0004) — legacy
    - Aes128Cbc (0x0005)
    - KeyExchange (0x0006)

    Cipher types (key wrapping cipher):
    - Invalid (0x0000)
    - Rsa1024 (0x0001) — legacy
    - ChainedLicense (0x0002)
    - Ecc256 (0x0003) — standard ElGamal on P-256
    - Ecc256WithKz (0x0004)
    - TeeTransient (0x0005)
    - Ecc256ViaSymmetric (0x0006) — scalable license, multi-layer AES-ECB derivation
*/
