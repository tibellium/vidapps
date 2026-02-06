use hex_literal::hex;

/**
    Widevine DRM System ID: `edef8ba9-79d6-4ace-a3c8-27dcd51d21ed`
*/
pub const WIDEVINE_SYSTEM_ID: [u8; 16] = hex!(
    "edef8ba9"
    "79d6"
    "4ace"
    "a3c8"
    "27dcd51d21ed"
);

/**
    PlayReady DRM System ID: `9a04f079-9840-4286-ab92-e65be0885f95`
*/
pub const PLAYREADY_SYSTEM_ID: [u8; 16] = hex!(
    "9a04f079"
    "9840"
    "4286"
    "ab92"
    "e65be0885f95"
);

/**
    Apple FairPlay DRM System ID: `94ce86fb-07ff-4f43-adb8-93d2fa968ca2`
*/
pub const FAIRPLAY_SYSTEM_ID: [u8; 16] = hex!(
    "94ce86fb"
    "07ff"
    "4f43"
    "adb8"
    "93d2fa968ca2"
);

/**
    W3C ClearKey System ID: `1077efec-c0b2-4d02-ace3-3c1e52e2fb4b`
*/
pub const CLEARKEY_SYSTEM_ID: [u8; 16] = hex!(
    "1077efec"
    "c0b2"
    "4d02"
    "ace3"
    "3c1e52e2fb4b"
);

/**
    The Widevine root DrmCertificate, used to verify service certificate signatures.

    This is a hardcoded constant extracted from a SignedDrmCertificate blob that is
    embedded in every Widevine CDM binary. Only the public_key field (field 4) is
    used — for RSA-PSS-SHA1 signature verification of service certificates.
*/
pub const ROOT_PUBLIC_KEY_N: [u8; 384] = hex!(
    "915f33d2508264b4783f5596a6ceb5f7"
    "12e812a76f03e5073e51d4f8b9dc1cfe"
    "c53d416d88d212ac3c9358ec23b81112"
    "2747e42be7e718fd08a5ff8415687d4c"
    "8a947c811c31977f4bea3c47e4370d59"
    "e024b3111fec35c88844560d82019ff2"
    "b219ed2514ad13398c695e0629e4bf4c"
    "6082dc8f78b07fbedc6d19d26fef75dc"
    "175b77485e4ffa30aab7d2fb003d111a"
    "607cba53c3ebdc11ff33455e52799802"
    "e012e6b48eb8f9b1338cca3474e4366b"
    "ff116cc8f5650e9218aa8448889bb827"
    "1f89ba4bec7db933b2b72b4882fdfc63"
    "193e178ae9b07e729ccbb4c15c824db4"
    "29bdc1faa0723ebc6f9325e22750407e"
    "fd202670208288a8ccd784eb979a539c"
    "852519e1d7d645719da91022d9baa976"
    "aedf4cd6920f8f1376a7fd09fd5f473e"
    "536948b54bec725b53ab8b2334be2280"
    "35b0fbab39848acb430e462f5d681615"
    "789821c5df66beb87f722695a9409c3f"
    "d236b3db78a67d356df64c530357a035"
    "9ffbdcdf6587db10b1234de7f29b5ec3"
    "f2cd68e80997113cdb039065c339feb4"
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
