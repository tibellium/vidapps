/*!
    PlayReady CDM error types.

    `CdmError` covers:
    - Device file parsing errors (invalid PRD format, version mismatch)
    - Certificate chain errors (invalid BCert, signature verification failure)
    - PSSH / WRM header errors
    - License errors (invalid XMR, signature mismatch, unsupported cipher type)
    - Session errors (invalid session ID, too many sessions)
    - Crypto errors (ElGamal, ECDSA, AES failures)
*/
