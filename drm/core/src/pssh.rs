use crate::error::PsshError;
use crate::types::SystemId;

/**
    Parsed PSSH box — preserves all ISOBMFF fields for round-trip fidelity.

    ISOBMFF PSSH box layout:
      [0..4]    box_size: u32 big-endian (total box size including this header)
      [4..8]    box_type: "pssh" (0x70737368)
      [8]       version: u8 (0 or 1)
      [9..12]   flags: u24 (typically 0x000000)
      [12..28]  system_id: 16 bytes
      if version == 1:
        [28..32]  key_id_count: u32 big-endian
        [32..]    key_ids: key_id_count * 16 bytes
      [..]      data_size: u32 big-endian
      [..]      data: data_size bytes
*/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsshBox {
    /**
        PSSH box version (0 or 1).
    */
    pub version: u8,
    /**
        3-byte flags field (typically all zeros).
    */
    pub flags: [u8; 3],
    /**
        16-byte DRM system identifier.
    */
    pub system_id: [u8; 16],
    /**
        Key IDs from the box header (v1 only). Empty for v0 boxes.
    */
    pub key_ids: Vec<[u8; 16]>,
    /**
        Raw data payload. For Widevine this is a serialized WidevinePsshData
        protobuf; for PlayReady it is a PlayReady Header Object; etc.
    */
    pub data: Vec<u8>,
}

impl PsshBox {
    /**
        Parse a base64-encoded PSSH box.
    */
    pub fn from_base64(pssh: &str) -> Result<Self, PsshError> {
        let bytes = data_encoding::BASE64
            .decode(pssh.as_bytes())
            .map_err(|e| PsshError::InvalidBase64(format!("PSSH: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /**
        Parse a PSSH box from raw bytes (full ISOBMFF box starting with box_size).
    */
    pub fn from_bytes(input: &[u8]) -> Result<Self, PsshError> {
        // Minimum: 4 (size) + 4 (type) + 1 (ver) + 3 (flags) + 16 (sysid) + 4 (data_size) = 32
        if input.len() < 32 {
            return Err(pssh_err("input too short for PSSH box header"));
        }

        let box_size = read_u32_be(input, 0) as usize;
        if box_size > input.len() {
            return Err(pssh_err("box_size exceeds input length"));
        }

        let box_data = &input[..box_size];
        if &box_data[4..8] != b"pssh" {
            return Err(pssh_err("box_type is not 'pssh'"));
        }

        let version = box_data[8];
        if version > 1 {
            return Err(pssh_err(&format!("unsupported version {version}")));
        }

        let mut flags = [0u8; 3];
        flags.copy_from_slice(&box_data[9..12]);

        let mut system_id = [0u8; 16];
        system_id.copy_from_slice(&box_data[12..28]);

        let mut offset = 28;
        let mut key_ids = Vec::new();

        if version == 1 {
            check_bounds(box_data, offset, 4, "key_id_count")?;
            let kid_count = read_u32_be(box_data, offset) as usize;
            offset += 4;

            check_bounds(box_data, offset, kid_count * 16, "key_ids")?;
            for i in 0..kid_count {
                let start = offset + i * 16;
                let mut kid = [0u8; 16];
                kid.copy_from_slice(&box_data[start..start + 16]);
                key_ids.push(kid);
            }
            offset += kid_count * 16;
        }

        check_bounds(box_data, offset, 4, "data_size")?;
        let data_size = read_u32_be(box_data, offset) as usize;
        offset += 4;

        check_bounds(box_data, offset, data_size, "data")?;
        let data = box_data[offset..offset + data_size].to_vec();
        offset += data_size;

        if offset != box_size {
            return Err(pssh_err(&format!(
                "trailing bytes: consumed {offset}, box_size {box_size}"
            )));
        }

        Ok(PsshBox {
            version,
            flags,
            system_id,
            key_ids,
            data,
        })
    }

    /**
        Serialize back to ISOBMFF PSSH box bytes.

        Produces identical bytes to the original input when round-tripping
        through `from_bytes` / `to_bytes`.
    */
    pub fn to_bytes(&self) -> Vec<u8> {
        // header: 4 (size) + 4 (type) + 1 (ver) + 3 (flags) + 16 (sysid) = 28
        let mut size = 28usize;
        if self.version == 1 {
            size += 4 + self.key_ids.len() * 16; // key_id_count + key_ids
        }
        size += 4 + self.data.len(); // data_size + data

        let mut buf = Vec::with_capacity(size);

        // box_size
        buf.extend_from_slice(&(size as u32).to_be_bytes());
        // box_type
        buf.extend_from_slice(b"pssh");
        // version
        buf.push(self.version);
        // flags
        buf.extend_from_slice(&self.flags);
        // system_id
        buf.extend_from_slice(&self.system_id);

        if self.version == 1 {
            // key_id_count
            buf.extend_from_slice(&(self.key_ids.len() as u32).to_be_bytes());
            // key_ids
            for kid in &self.key_ids {
                buf.extend_from_slice(kid);
            }
        }

        // data_size
        buf.extend_from_slice(&(self.data.len() as u32).to_be_bytes());
        // data
        buf.extend_from_slice(&self.data);

        buf
    }

    /**
        Serialize to a base64-encoded PSSH box string.
    */
    pub fn to_base64(&self) -> String {
        data_encoding::BASE64.encode(&self.to_bytes())
    }

    /**
        Key IDs from the box header (v1 only).

        Returns the key IDs stored in the PSSH box header. For v0 boxes this
        is always empty — DRM-specific code must parse the data payload to
        extract key IDs (e.g. from a WidevinePsshData protobuf).
    */
    pub fn key_ids(&self) -> &[[u8; 16]] {
        &self.key_ids
    }

    /**
        Raw init data payload (the `data` field inside the PSSH box).
    */
    pub fn init_data(&self) -> &[u8] {
        &self.data
    }

    /**
        Identify the DRM system from the PSSH box's system ID.
    */
    pub fn system_id(&self) -> SystemId {
        SystemId::from_bytes(self.system_id)
    }

    /**
        Check that this PSSH box belongs to the given DRM system.
        Returns `Err(PsshError::SystemIdMismatch)` if it does not.
    */
    pub fn ensure_system_id(&self, expected: SystemId) -> Result<(), PsshError> {
        let actual = self.system_id();
        if actual == expected {
            Ok(())
        } else {
            Err(PsshError::SystemIdMismatch(actual, expected))
        }
    }
}

fn read_u32_be(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn check_bounds(data: &[u8], offset: usize, need: usize, field: &str) -> Result<(), PsshError> {
    if offset + need > data.len() {
        Err(pssh_err(&format!("truncated {field}")))
    } else {
        Ok(())
    }
}

fn pssh_err(msg: &str) -> PsshError {
    PsshError::Malformed(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;

    const WV_SYSID: [u8; 16] = hex!("edef8ba979d64acea3c827dcd51d21ed");

    /// Build a minimal v0 PSSH box with the given data payload.
    fn build_v0_pssh(data: &[u8]) -> Vec<u8> {
        // header(28) + data_size(4) + data
        let box_size = (32 + data.len()) as u32;
        let mut buf = Vec::new();
        buf.extend_from_slice(&box_size.to_be_bytes());
        buf.extend_from_slice(b"pssh");
        buf.push(0); // version 0
        buf.extend_from_slice(&[0, 0, 0]); // flags
        buf.extend_from_slice(&WV_SYSID);
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(data);
        buf
    }

    /// Build a v1 PSSH box with key IDs and a data payload.
    fn build_v1_pssh(key_ids: &[[u8; 16]], data: &[u8]) -> Vec<u8> {
        let box_size = (32 + 4 + key_ids.len() * 16 + data.len()) as u32;
        let mut buf = Vec::new();
        buf.extend_from_slice(&box_size.to_be_bytes());
        buf.extend_from_slice(b"pssh");
        buf.push(1); // version 1
        buf.extend_from_slice(&[0, 0, 0]); // flags
        buf.extend_from_slice(&WV_SYSID);
        buf.extend_from_slice(&(key_ids.len() as u32).to_be_bytes());
        for kid in key_ids {
            buf.extend_from_slice(kid);
        }
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(data);
        buf
    }

    #[test]
    fn parse_v0_round_trip() {
        let data = b"test-pssh-data";
        let raw = build_v0_pssh(data);
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        assert_eq!(pssh.version, 0);
        assert_eq!(pssh.system_id, WV_SYSID);
        assert!(pssh.key_ids.is_empty());
        assert_eq!(pssh.data, data);
        assert_eq!(pssh.to_bytes(), raw);
    }

    #[test]
    fn parse_v1_with_key_ids() {
        let kid1 = hex!("00000000000000000000000000000001");
        let kid2 = hex!("00000000000000000000000000000002");
        let data = b"payload";
        let raw = build_v1_pssh(&[kid1, kid2], data);
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        assert_eq!(pssh.version, 1);
        assert_eq!(pssh.key_ids.len(), 2);
        assert_eq!(pssh.key_ids[0], kid1);
        assert_eq!(pssh.key_ids[1], kid2);
        assert_eq!(pssh.data, data);
        // v1 key_ids() returns box header kids
        assert_eq!(pssh.key_ids(), &[kid1, kid2]);
    }

    #[test]
    fn v1_round_trip() {
        let kid = hex!("aabbccddaabbccddaabbccddaabbccdd");
        let raw = build_v1_pssh(&[kid], b"");
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        assert_eq!(pssh.to_bytes(), raw);
    }

    #[test]
    fn base64_round_trip() {
        let raw = build_v0_pssh(b"hello");
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        let b64 = pssh.to_base64();
        let pssh2 = PsshBox::from_base64(&b64).unwrap();
        assert_eq!(pssh, pssh2);
    }

    #[test]
    fn any_system_id_accepted() {
        let mut raw = build_v0_pssh(b"data");
        // Change system ID to something unknown
        raw[12] = 0xFF;
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        assert!(pssh.system_id().is_unknown());
    }

    #[test]
    fn ensure_system_id_mismatch() {
        let mut raw = build_v0_pssh(b"data");
        raw[12] = 0xFF;
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        let err = pssh.ensure_system_id(SystemId::Widevine).unwrap_err();
        assert!(matches!(err, PsshError::SystemIdMismatch(_, _)));
    }

    #[test]
    fn ensure_system_id_match() {
        let raw = build_v0_pssh(b"data");
        let pssh = PsshBox::from_bytes(&raw).unwrap();
        pssh.ensure_system_id(SystemId::Widevine).unwrap();
    }

    #[test]
    fn not_pssh_box_type() {
        let mut raw = build_v0_pssh(b"data");
        raw[4..8].copy_from_slice(b"moof");
        let err = PsshBox::from_bytes(&raw).unwrap_err();
        assert!(matches!(err, PsshError::Malformed(_)));
    }

    #[test]
    fn truncated_input() {
        let err = PsshBox::from_bytes(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, PsshError::Malformed(_)));
    }

    #[test]
    fn unsupported_version() {
        let mut raw = build_v0_pssh(b"data");
        raw[8] = 2; // version 2 not supported
        let err = PsshBox::from_bytes(&raw).unwrap_err();
        assert!(matches!(err, PsshError::Malformed(_)));
    }
}
