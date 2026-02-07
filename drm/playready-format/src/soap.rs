/*!
    SOAP/XML message types for PlayReady license acquisition.

    Challenge (client → server):
    ```xml
    <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
      <soap:Body>
        <AcquireLicense xmlns="http://schemas.microsoft.com/DRM/2007/03/protocols">
          <challenge>
            <Challenge xmlns="http://schemas.microsoft.com/DRM/2007/03/protocols/messages">
              <LA Id="SignedData" xml:space="preserve">
                <!-- client version, WRM header, encrypted cert chain, encrypted key -->
              </LA>
              <Signature xmlns="http://www.w3.org/2000/09/xmldsig#">
                <SignedInfo>...</SignedInfo>
                <SignatureValue><!-- ECDSA-SHA256, base64 --></SignatureValue>
              </Signature>
            </Challenge>
          </challenge>
        </AcquireLicense>
      </soap:Body>
    </soap:Envelope>
    ```

    Response (server → client):
    - SOAP envelope wrapping `AcquireLicenseResponse`
    - `<License>` elements are base64-encoded XMR blobs

    Uses SOAP 1.1. Protocol namespace: `http://schemas.microsoft.com/DRM/2007/03/protocols`.
    Message namespace: `http://schemas.microsoft.com/DRM/2007/03/protocols/messages`.
*/
