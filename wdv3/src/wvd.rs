use crate::error::{CdmError, CdmResult};
use crate::types::{DeviceType, SecurityLevel};

const MAGIC: &[u8] = b"WVD";

/// Represents a parsed WVD (Widevine Device) file.
#[derive(Debug)]
pub struct WvdDevice {
    /// Device type as encoded in WVD file byte offset 4.
    pub device_type: DeviceType,
    /// Security level as encoded in WVD file byte offset 5.
    pub security_level: SecurityLevel,
    /// DER-encoded RSA private key (PKCS#1 RSAPrivateKey) starting at offset 9, length from uint16be at offset 7.
    pub private_key_der: Vec<u8>,
    /// Raw serialized ClientIdentification protobuf starting at offset 11 + private_key_len,
    /// length from uint16be at offset 9 + private_key_len.
    pub client_id_blob: Vec<u8>,
}

impl WvdDevice {
    /// Creates a new WvdDevice instance
    fn new(
        device_type: DeviceType,
        security_level: SecurityLevel,
        private_key_der: impl Into<Vec<u8>>,
        client_id_blob: impl Into<Vec<u8>>,
    ) -> Self {
        WvdDevice {
            device_type,
            security_level,
            private_key_der: private_key_der.into(),
            client_id_blob: client_id_blob.into(),
        }
    }

    /// Parse a base64-encoded WVD v2 file.
    pub fn from_base64(wvd: &str) -> CdmResult<Self> {
        let bytes = data_encoding::BASE64
            .decode(wvd.as_bytes())
            .map_err(|e| CdmError::InvalidBase64(format!("WVD: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /// Parse a WVD v2 file from raw bytes
    pub fn from_bytes(data: impl AsRef<[u8]>) -> CdmResult<Self> {
        // Check for correct magic bytes
        let data: &[u8] = data.as_ref();
        if data.len() < MAGIC.len() || &data[0..MAGIC.len()] != MAGIC {
            return Err(CdmError::WvdBadMagic);
        }

        // Check version
        let version = *data.get(4).ok_or(CdmError::WvdTruncated)?;
        let device_type = *data.get(5).ok_or(CdmError::WvdTruncated)?;
        let security_level = *data.get(6).ok_or(CdmError::WvdTruncated)?;

        if version != 2 {
            return Err(CdmError::WvdUnsupportedVersion(version));
        }

        // Parse device type and security level
        let device_type =
            DeviceType::from_u8(device_type).ok_or(CdmError::WvdBadDeviceType(device_type))?;
        let security_level = SecurityLevel::from_u8(security_level)
            .ok_or(CdmError::WvdBadSecurityLevel(security_level))?;

        // Parse private key length
        let private_key_len =
            u16::from_be_bytes(data[7..9].try_into().map_err(|_| CdmError::WvdTruncated)?);

        // Check there's enough data for the private key
        if 9 + private_key_len as usize > data.len() {
            return Err(CdmError::WvdTruncated);
        }

        let private_key_der = &data[9..9 + private_key_len as usize];

        // Parse client ID blob length
        let client_id_blob_len = u16::from_be_bytes(
            data[(9 + private_key_len as usize)..(11 + private_key_len as usize)]
                .try_into()
                .map_err(|_| CdmError::WvdTruncated)?,
        );

        // Check there's enough data for the client ID blob
        if 11 + private_key_len as usize + client_id_blob_len as usize > data.len() {
            return Err(CdmError::WvdTruncated);
        }

        let client_id_blob = &data[(11 + private_key_len as usize)
            ..(11 + private_key_len as usize + client_id_blob_len as usize)];

        Ok(WvdDevice::new(
            device_type,
            security_level,
            private_key_der,
            client_id_blob,
        ))
    }

    /// Serialize a WvdDevice instance back into its original WVD file format
    pub fn to_bytes(&self) -> CdmResult<Vec<u8>> {
        let mut buffer = Vec::new();

        // Write magic bytes
        buffer.extend(MAGIC);

        // Write version byte (2)
        buffer.push(2u8);

        // Write device type and security level
        buffer.push(self.device_type.to_u8());
        buffer.push(self.security_level.to_u8());

        // Write flags byte (reserved, always 0x00)
        buffer.push(0x00);

        // Write private key length as big-endian u16
        let private_key_len = self.private_key_der.len() as u16;
        buffer.extend(&private_key_len.to_be_bytes());

        // Write private key DER data
        buffer.extend(&self.private_key_der);

        // Write client ID blob length as big-endian u16
        let client_id_blob_len = self.client_id_blob.len() as u16;
        buffer.extend(&client_id_blob_len.to_be_bytes());

        // Write client ID blob data
        buffer.extend(&self.client_id_blob);

        Ok(buffer)
    }

    /// Serialize to a base64-encoded WVD string.
    pub fn to_base64(&self) -> CdmResult<String> {
        self.to_bytes().map(|b| data_encoding::BASE64.encode(&b))
    }
}
