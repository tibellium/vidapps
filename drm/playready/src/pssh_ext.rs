/*!
    PlayReady extension trait for `PsshBox`.

    `PlayReadyExt` adds PlayReady-specific methods to `drm_core::PsshBox`:
    - Extract PlayReady Header (PRH) from PSSH data
    - Parse PlayReady Object (PRO) records
    - Extract WRM Header XML from type-1 records
    - Extract KIDs from WRM Header (GUID little-endian byte order)
*/
