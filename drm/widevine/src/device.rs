use rsa::{
    RsaPrivateKey,
    pkcs1::{DecodeRsaPrivateKey, EncodeRsaPrivateKey},
};

use drm_widevine_proto::{ClientIdentification, prost::Message};

use crate::error::{CdmError, CdmResult};
use crate::types::{DeviceType, SecurityLevel};

const MAGIC: &[u8] = b"WVD";

/**
    Represents a Widevine Device.
    Can be parsed from a wvd file.
*/
#[derive(Debug, Clone)]
pub struct Device {
    /// Device type as encoded in WVD file byte offset 4.
    pub device_type: DeviceType,
    /// Security level as encoded in WVD file byte offset 5.
    pub security_level: SecurityLevel,
    /// Parsed RSA private key (PKCS#1), validated at load time.
    pub(crate) private_key: RsaPrivateKey,
    /// Parsed ClientIdentification protobuf, validated at load time.
    pub(crate) client_id: ClientIdentification,
}

impl Device {
    /**
        Create a new device from its raw components.
    */
    pub fn new(
        device_type: DeviceType,
        security_level: SecurityLevel,
        private_key: RsaPrivateKey,
        client_id: ClientIdentification,
    ) -> Self {
        Self {
            device_type,
            security_level,
            private_key,
            client_id,
        }
    }

    /**
        Returns the parsed RSA private key.
    */
    pub fn private_key(&self) -> &RsaPrivateKey {
        &self.private_key
    }

    /**
        Returns the parsed client identification metadata.
    */
    pub fn client_id(&self) -> &ClientIdentification {
        &self.client_id
    }

    /**
        Parse a base64-encoded WVD v2 file.
    */
    pub fn from_base64(wvd: impl AsRef<[u8]>) -> CdmResult<Self> {
        let bytes = data_encoding::BASE64
            .decode(wvd.as_ref())
            .map_err(|e| CdmError::InvalidBase64(format!("WVD: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /**
        Parse a WVD v2 file from raw bytes.
    */
    pub fn from_bytes(data: impl AsRef<[u8]>) -> CdmResult<Self> {
        let data: &[u8] = data.as_ref();

        // Check for correct magic bytes
        if data.len() < MAGIC.len() || &data[0..MAGIC.len()] != MAGIC {
            return Err(CdmError::WvdBadMagic);
        }

        // Header layout (matches to_bytes):
        //   [0..3]  magic "WVD"
        //   [3]     version
        //   [4]     device_type
        //   [5]     security_level
        //   [6]     flags (reserved, ignored)
        //   [7..9]  private_key_len (u16 big-endian)
        let version = *data.get(3).ok_or(CdmError::WvdTruncated)?;
        let device_type = *data.get(4).ok_or(CdmError::WvdTruncated)?;
        let security_level = *data.get(5).ok_or(CdmError::WvdTruncated)?;

        if version != 2 {
            return Err(CdmError::WvdUnsupportedVersion(version));
        }

        let device_type =
            DeviceType::from_u8(device_type).ok_or(CdmError::WvdBadDeviceType(device_type))?;
        let security_level = SecurityLevel::from_u8(security_level)
            .ok_or(CdmError::WvdBadSecurityLevel(security_level))?;

        // Parse private key length (offset 7..9, big-endian u16)
        let private_key_len =
            u16::from_be_bytes(data[7..9].try_into().map_err(|_| CdmError::WvdTruncated)?);

        if 9 + private_key_len as usize > data.len() {
            return Err(CdmError::WvdTruncated);
        }

        let private_key_der = &data[9..9 + private_key_len as usize];

        // Parse and validate the RSA private key
        let private_key = RsaPrivateKey::from_pkcs1_der(private_key_der)
            .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;

        // Parse client ID blob
        let cid_offset = 9 + private_key_len as usize;
        let client_id_len = u16::from_be_bytes(
            data[cid_offset..cid_offset + 2]
                .try_into()
                .map_err(|_| CdmError::WvdTruncated)?,
        );

        let cid_start = cid_offset + 2;
        if cid_start + client_id_len as usize > data.len() {
            return Err(CdmError::WvdTruncated);
        }

        let client_id_bytes = &data[cid_start..cid_start + client_id_len as usize];

        // Parse and validate the ClientIdentification protobuf
        let client_id = ClientIdentification::decode(client_id_bytes)?;

        Ok(Device {
            device_type,
            security_level,
            private_key,
            client_id,
        })
    }

    /**
        Serialize back into WVD v2 file format bytes.
    */
    pub fn to_bytes(&self) -> CdmResult<Vec<u8>> {
        let private_key_der = self
            .private_key
            .to_pkcs1_der()
            .map_err(|e| CdmError::RsaKeyParse(e.to_string()))?;
        let private_key_bytes = private_key_der.as_bytes();
        let client_id_bytes = self.client_id.encode_to_vec();

        let mut buffer = Vec::new();

        // Magic + version
        buffer.extend(MAGIC);
        buffer.push(2u8);

        // Device type + security level
        buffer.push(self.device_type.to_u8());
        buffer.push(self.security_level.to_u8());

        // Flags byte (reserved, always 0x00)
        buffer.push(0x00);

        // Private key
        let private_key_len: u16 = private_key_bytes
            .len()
            .try_into()
            .map_err(|_| CdmError::WvdFieldTooLarge(private_key_bytes.len()))?;
        buffer.extend(&private_key_len.to_be_bytes());
        buffer.extend(private_key_bytes);

        // Client ID
        let client_id_len: u16 = client_id_bytes
            .len()
            .try_into()
            .map_err(|_| CdmError::WvdFieldTooLarge(client_id_bytes.len()))?;
        buffer.extend(&client_id_len.to_be_bytes());
        buffer.extend(&client_id_bytes);

        Ok(buffer)
    }

    /**
        Serialize to a base64-encoded WVD string.
    */
    pub fn to_base64(&self) -> CdmResult<String> {
        self.to_bytes().map(|b| data_encoding::BASE64.encode(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_WVD: &[u8] = include_bytes!("../testfiles/device.wvd");

    #[test]
    fn parse_test_device() {
        let device = Device::from_bytes(TEST_WVD).expect("failed to parse test WVD");
        assert_eq!(device.device_type, DeviceType::Android);
        assert_eq!(device.security_level, SecurityLevel::L3);
    }

    #[test]
    fn round_trip() {
        let device = Device::from_bytes(TEST_WVD).unwrap();
        let serialized = device.to_bytes().unwrap();
        let device2 = Device::from_bytes(&serialized).unwrap();
        assert_eq!(device2.device_type, device.device_type);
        assert_eq!(device2.security_level, device.security_level);
        assert_eq!(device2.private_key, device.private_key);
        assert_eq!(
            device2.client_id.encode_to_vec(),
            device.client_id.encode_to_vec()
        );
    }

    #[test]
    fn base64_round_trip() {
        let device = Device::from_bytes(TEST_WVD).unwrap();
        let b64 = device.to_base64().unwrap();
        let device2 = Device::from_base64(&b64).unwrap();
        assert_eq!(device2.device_type, device.device_type);
        assert_eq!(device2.security_level, device.security_level);
    }

    #[test]
    fn client_id_has_metadata() {
        let device = Device::from_bytes(TEST_WVD).unwrap();
        // The test device should have client_info populated
        assert!(
            !device.client_id.client_info.is_empty(),
            "client_info should not be empty"
        );
    }

    #[test]
    fn bad_magic() {
        let mut data = TEST_WVD.to_vec();
        data[0] = b'X';
        let err = Device::from_bytes(&data).unwrap_err();
        assert!(matches!(err, CdmError::WvdBadMagic));
    }

    #[test]
    fn wrong_version() {
        let mut data = TEST_WVD.to_vec();
        data[3] = 99; // version byte
        let err = Device::from_bytes(&data).unwrap_err();
        assert!(matches!(err, CdmError::WvdUnsupportedVersion(99)));
    }

    #[test]
    fn bad_device_type() {
        let mut data = TEST_WVD.to_vec();
        data[4] = 0; // invalid device type
        let err = Device::from_bytes(&data).unwrap_err();
        assert!(matches!(err, CdmError::WvdBadDeviceType(0)));
    }

    #[test]
    fn bad_security_level() {
        let mut data = TEST_WVD.to_vec();
        data[5] = 9; // invalid security level
        let err = Device::from_bytes(&data).unwrap_err();
        assert!(matches!(err, CdmError::WvdBadSecurityLevel(9)));
    }

    #[test]
    fn truncated_input() {
        let err = Device::from_bytes(b"WVD").unwrap_err();
        assert!(matches!(err, CdmError::WvdTruncated));
    }

    #[test]
    fn empty_input() {
        let err = Device::from_bytes(b"").unwrap_err();
        assert!(matches!(err, CdmError::WvdBadMagic));
    }
}
