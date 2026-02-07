use drm_widevine_proto::prost::Message;

use drm_core::{PsshBox, PsshError, SystemId};

use crate::error::{CdmError, CdmResult};

/**
    Widevine-specific extensions for [`PsshBox`].
*/
pub trait WidevineExt {
    /**
        Decode the data payload as a WidevinePsshData protobuf.
    */
    fn widevine_pssh_data(&self) -> CdmResult<drm_widevine_proto::WidevinePsshData>;

    /**
        Extract key IDs, preferring the box header (v1) over protobuf parsing (v0).

        - v1: returns the key IDs stored in the box header directly.
        - v0: decodes `self.data` as a WidevinePsshData protobuf and extracts
          the `key_id` repeated field.
    */
    fn widevine_key_ids(&self) -> CdmResult<Vec<[u8; 16]>>;

    /**
        Check that this PSSH box is a Widevine box.
    */
    fn ensure_widevine(&self) -> CdmResult<()>;
}

impl WidevineExt for PsshBox {
    fn widevine_pssh_data(&self) -> CdmResult<drm_widevine_proto::WidevinePsshData> {
        drm_widevine_proto::WidevinePsshData::decode(self.data.as_slice()).map_err(CdmError::from)
    }

    fn widevine_key_ids(&self) -> CdmResult<Vec<[u8; 16]>> {
        let header_kids = self.key_ids();
        if !header_kids.is_empty() {
            return Ok(header_kids.to_vec());
        }

        let pssh_data = self.widevine_pssh_data()?;
        let mut kids = Vec::with_capacity(pssh_data.key_ids.len());
        for raw_kid in &pssh_data.key_ids {
            if raw_kid.len() != 16 {
                return Err(CdmError::PsshCore(PsshError::Malformed(format!(
                    "key_id length {} (expected 16)",
                    raw_kid.len()
                ))));
            }
            let mut kid = [0u8; 16];
            kid.copy_from_slice(raw_kid);
            kids.push(kid);
        }
        Ok(kids)
    }

    fn ensure_widevine(&self) -> CdmResult<()> {
        self.ensure_system_id(SystemId::Widevine)
            .map_err(CdmError::PsshCore)
    }
}
