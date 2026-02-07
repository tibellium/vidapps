/*!
    WRM (Windows Rights Management) Header XML format.

    Found inside PlayReady Object (PRO) records (type 1) within PSSH boxes.
    Encoded as UTF-16 LE XML.

    Versions: 4.0, 4.1, 4.2, 4.3

    Key elements:
    - `<WRMHEADER>` root with `version` attribute
    - `<DATA>` container
    - `<PROTECTINFO>` → `<KIDS>` → `<KID>` (base64-encoded GUID, little-endian byte order)
    - `<LA_URL>` — license acquisition URL
    - `<LUI_URL>` — license UI URL
    - `<CHECKSUM>` — content key checksum
    - `<ALGID>` — encryption algorithm (AESCTR, AESCBC, COCKTAIL)

    PlayReady Header (PRH) wrapping:
    - length (u32 LE) + record_count (u16 LE) + records[]
    - Each record: type (u16 LE) + length (u16 LE) + data

    IMPORTANT: PSSH box framing is big-endian (ISO BMFF), but all PRH/PRO
    fields are little-endian.
*/
