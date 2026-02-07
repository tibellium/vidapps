/*!
    PlayReady CDM session state machine.

    Lifecycle:
    1. Create session from a `Device`
    2. Generate license challenge XML (SOAP envelope) from a WRM header
       - Generates ephemeral XmlKey (ECC P-256 keypair)
       - ElGamal-encrypts XmlKey to WMRM server public key
       - AES-CBC encrypts client cert chain with XmlKey-derived key/IV
       - ECDSA-SHA256 signs the challenge with device signing key
    3. Parse license response
       - Extracts XMR license blobs from SOAP response
       - Verifies encryption key matches session
       - ElGamal-decrypts content keys with session encryption key
       - Verifies AES-CMAC license integrity
       - Handles scalable license (ECC_256_VIA_SYMMETRIC) key derivation
    4. Retrieve decrypted content keys
*/
