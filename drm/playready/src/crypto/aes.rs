/*!
    AES symmetric cipher operations for PlayReady.

    - AES-128-CBC with PKCS7 padding: encrypts client data in the license challenge
    - AES-128-ECB (no padding): used in scalable license key derivation chain
    - AES-128-CMAC: verifies license integrity signatures in XMR blobs
*/
