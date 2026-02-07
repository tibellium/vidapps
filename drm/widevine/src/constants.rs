use hex_literal::hex;

/**
    The Widevine root DrmCertificate, used to verify service certificate signatures.

    This is a hardcoded constant extracted from a SignedDrmCertificate blob that is
    embedded in every Widevine CDM binary. Only the public_key field (field 4) is
    used — for RSA-PSS-SHA1 signature verification of service certificates.
*/
pub const ROOT_PUBLIC_KEY_N: [u8; 384] = hex!(
    "b4fe39c3659003db3c119709e868cdf2"
    "c35e9bf2e74d23b110db8765dfdcfb9f"
    "35a05703534cf66d357da678dbb336d2"
    "3f9c40a99526727fb8be66dfc5219878"
    "1516685d2f460e43cb8a8439abfbb035"
    "8022be34238bab535b72ec4bb5486953"
    "3e475ffd09fda776138f0f92d64cdfae"
    "76a9bad92210a99d7145d6d7e1192585"
    "9c539a97eb84d7cca8888220702620fd"
    "7e405027e225936fbc3e72a0fac1bd29"
    "b44d825cc1b4cb9c727eb0e98a173e19"
    "63fcfd82482bb7b233b97dec4bba891f"
    "27b89b884884aa18920e65f5c86c11ff"
    "6b36e47434ca8c33b1f9b88eb4e612e0"
    "029879525e4533ff11dcebc353ba7c60"
    "1a113d00fbd2b7aa30fa4f5e48775b17"
    "dc75ef6fd2196ddcbe7fb0788fdc8260"
    "4cbfe429065e698c3913ad1425ed19b2"
    "f29f01820d564488c835ec1f11b324e0"
    "590d37e4473cea4b7f97311c817c948a"
    "4c7d681584ffa508fd18e7e72be44727"
    "1211b823ec58933cac12d2886d413dc5"
    "fe1cdcb9f8d4513e07e5036fa712e812"
    "f7b5cea696553f78b4648250d2335f91"
);

pub const ROOT_PUBLIC_KEY_E: [u8; 3] = hex!("010001");

/**
    Provider ID for the Widevine production environment.
*/
pub const LICENSE_PRODUCTION_PROVIDER_ID: &str = "license.widevine.com";

/**
    Serial number for the Widevine production environment.
*/
pub const LICENSE_PRODUCTION_SERIAL: [u8; 16] = hex!("1705b917cc1204868b06333a2f772a8c");

/**
    First part of RSA public key for the Widevine production environment.
*/
pub const LICENSE_PRODUCTION_N: [u8; 256] = hex!(
    "095a9f9c015012cf1b71b408d3fb64df"
    "6e5efcb05d9f6b0b2f58e24328e8590c"
    "012f4baf37ec4ea7904413f3c54a2cd8"
    "c6676f0d6882707024ceed59830b1296"
    "b982a0735cc5d76ce7d0e264f5ba5bf5"
    "eefc9a9260bdee97bfa420954cbac4d1"
    "04c6b040bfe131fd4264fb6f3df19233"
    "decaf1badd1882435daa7ea40c4947ca"
    "104abdec4efb213a985d7033ebcd7cd6"
    "a837b15784ac4fe0dc7a60a858800ee6"
    "143d26465fa4e881571e9e01e177eafe"
    "fbbf217e8c878c156f0b610830397912"
    "a9380eafe1a7234058581d2995079e4a"
    "5e5a724e8cb81bb1ade38cad41045140"
    "dfb876d814b845063e5037cbbcd50a52"
    "98b5952ab6c3ef245eab7d323b5bed99"
);

/**
    Second part of RSA public key for the Widevine production environment.
*/
pub const LICENSE_PRODUCTION_E: [u8; 3] = hex!("010001");

/**
    Provider ID for the Widevine staging environment.
*/
pub const LICENSE_STAGING_PROVIDER_ID: &str = "staging.google.com";

/**
    Serial number for the Widevine staging environment.
*/
pub const LICENSE_STAGING_SERIAL: [u8; 16] = hex!("28703454c008f63618ade7443db6c4c8");

/**
    First part of RSA public key for the Widevine staging environment.
*/
pub const LICENSE_STAGING_N: [u8; 256] = hex!(
    "43d99a7fa067fd24af9dbc8694133836"
    "4c3303476001ef3c99a0d0c0a0604df7"
    "a2bcc293d8450d0868d6f10858e5be90"
    "935872ab54424f3d28f63ef367674842"
    "efefdfb7563692905e90bd507821ac2b"
    "53001fc08c490e4af70151adad066a64"
    "dc7dca920f98915a674df1d8dcee40c7"
    "bb090bc540a0a380ffef81f0414c5ac0"
    "8a215a5b18d3a134f16d17147e2aba4d"
    "adf5aab6f91e5e7f891827604c3e0d63"
    "664f1c17aa627985b9f294b8a6b9e126"
    "0d1d81ef665b076f51b294ea5ad4897a"
    "c00a5fbb67e0f5c7a222b374629a5e81"
    "0754e9df08dc5fd54699b78231bc2a3d"
    "1e66de4367b05b35efbed2d87c17b449"
    "c6c151c2e2955dcc3f025dd0b81221b5"
);

/**
    Second part of RSA public key for the Widevine staging environment.
*/
pub const LICENSE_STAGING_E: [u8; 3] = hex!("010001");
