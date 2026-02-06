use prost::Message;

use crate::constants::WIDEVINE_SYSTEM_ID;
use crate::error::{CdmError, CdmResult};

/// Parsed PSSH box — preserves all ISOBMFF fields for round-trip fidelity.
///
/// ISOBMFF PSSH box layout:
///   [0..4]    box_size: u32 big-endian (total box size including this header)
///   [4..8]    box_type: "pssh" (0x70737368)
///   [8]       version: u8 (0 or 1)
///   [9..12]   flags: u24 (typically 0x000000)
///   [12..28]  system_id: 16 bytes
///   if version == 1:
///     [28..32]  key_id_count: u32 big-endian
///     [32..]    key_ids: key_id_count * 16 bytes
///   [..]      data_size: u32 big-endian
///   [..]      data: data_size bytes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsshBox {
    /// PSSH box version (0 or 1).
    pub version: u8,
    /// 3-byte flags field (typically all zeros).
    pub flags: [u8; 3],
    /// 16-byte DRM system identifier. For Widevine this must match
    /// `WIDEVINE_SYSTEM_ID` (EDEF8BA9-79D6-4ACE-A3C8-27DCD51D21ED).
    pub system_id: [u8; 16],
    /// Key IDs from the box header (v1 only). Empty for v0 boxes.
    pub key_ids: Vec<[u8; 16]>,
    /// Raw data payload — for Widevine, a serialized WidevinePsshData protobuf.
    pub data: Vec<u8>,
}

impl PsshBox {
    /// Parse a base64-encoded PSSH box.
    pub fn from_base64(pssh: &str) -> CdmResult<Self> {
        let bytes = data_encoding::BASE64
            .decode(pssh.as_bytes())
            .map_err(|e| CdmError::InvalidBase64(format!("PSSH: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /// Parse a PSSH box from raw bytes (full ISOBMFF box starting with box_size).
    pub fn from_bytes(input: &[u8]) -> CdmResult<Self> {
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

        if system_id != WIDEVINE_SYSTEM_ID {
            return Err(CdmError::PsshSystemIdMismatch);
        }

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

    /// Serialize back to ISOBMFF PSSH box bytes.
    ///
    /// Produces identical bytes to the original input when round-tripping
    /// through `from_bytes` / `to_bytes`.
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

    /// Serialize to a base64-encoded PSSH box string.
    pub fn to_base64(&self) -> String {
        data_encoding::BASE64.encode(&self.to_bytes())
    }

    /// Extract key IDs, preferring the box header (v1) over protobuf parsing (v0).
    ///
    /// - v1: returns the key IDs stored in the box header directly.
    /// - v0: decodes `self.data` as a WidevinePsshData protobuf and extracts
    ///   the `key_id` repeated field.
    pub fn key_ids(&self) -> CdmResult<Vec<[u8; 16]>> {
        if self.version == 1 {
            return Ok(self.key_ids.clone());
        }

        let pssh_data = self.widevine_pssh_data()?;
        let mut kids = Vec::with_capacity(pssh_data.key_id.len());
        for raw_kid in &pssh_data.key_id {
            if raw_kid.len() != 16 {
                return Err(pssh_err(&format!(
                    "key_id length {} (expected 16)",
                    raw_kid.len()
                )));
            }
            let mut kid = [0u8; 16];
            kid.copy_from_slice(raw_kid);
            kids.push(kid);
        }
        Ok(kids)
    }

    /// Raw init data payload (the `data` field inside the PSSH box).
    pub fn init_data(&self) -> &[u8] {
        &self.data
    }

    /// Decode the data payload as a WidevinePsshData protobuf.
    pub fn widevine_pssh_data(&self) -> CdmResult<wdv3_proto::WidevinePsshData> {
        wdv3_proto::WidevinePsshData::decode(self.data.as_slice())
            .map_err(|e| CdmError::ProtobufDecode(e.to_string()))
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

fn check_bounds(data: &[u8], offset: usize, need: usize, field: &str) -> CdmResult<()> {
    if offset + need > data.len() {
        Err(pssh_err(&format!("truncated {field}")))
    } else {
        Ok(())
    }
}

fn pssh_err(msg: &str) -> CdmError {
    CdmError::PsshMalformed(msg.into())
}
