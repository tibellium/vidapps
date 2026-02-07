/**
    Const-compatible byte slice equality.
*/
pub(crate) const fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/**
    Const-compatible ASCII whitespace trimming (both ends).
*/
pub(crate) const fn trim_ascii(s: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < s.len() && s[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = s.len();
    while end > start && s[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    // SAFETY: start <= end <= s.len(), but we use manual slicing for const.
    // Unfortunately &s[start..end] isn't const-stable, so we use from_raw_parts.
    unsafe { std::slice::from_raw_parts(s.as_ptr().add(start), end - start) }
}

/**
    Decode a single ASCII hex digit to its 4-bit value.
    Returns `None` for non-hex characters.
*/
pub(crate) const fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/**
    Const-compatible case-insensitive ASCII byte comparison.
    Both slices must have the same length (caller must check).
*/
pub(crate) const fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        let ca = if a[i].is_ascii_uppercase() {
            a[i] + 32
        } else {
            a[i]
        };
        let cb = if b[i].is_ascii_uppercase() {
            b[i] + 32
        } else {
            b[i]
        };
        if ca != cb {
            return false;
        }
        i += 1;
    }
    true
}

/**
    Parse a key ID from various input formats.

    String types (`&str`, `String`) are treated as hex and decoded into 16 bytes.
    Byte types (`&[u8]`, `[u8; 16]`, `&[u8; 16]`) are used directly when exactly
    16 bytes, or decoded as hex when exactly 32 bytes.

    Returns `None` if the input cannot be interpreted as a 16-byte key ID.
*/
pub fn parse_kid(input: impl ParseKid) -> Option<[u8; 16]> {
    input.parse_kid()
}

/**
    Trait for types that can be interpreted as a 16-byte key ID.

    See [`parse_kid`] for details.
*/
pub trait ParseKid {
    fn parse_kid(self) -> Option<[u8; 16]>;
}

/**
    Decode exactly 32 hex digits into 16 bytes. Returns `None` on invalid hex or wrong length.
*/
fn decode_hex_kid(s: &[u8]) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        let hi = hex_digit(s[i * 2])?;
        let lo = hex_digit(s[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
        i += 1;
    }
    Some(out)
}

impl ParseKid for &str {
    fn parse_kid(self) -> Option<[u8; 16]> {
        decode_hex_kid(self.as_bytes())
    }
}

impl ParseKid for String {
    fn parse_kid(self) -> Option<[u8; 16]> {
        decode_hex_kid(self.as_bytes())
    }
}

impl ParseKid for &[u8] {
    fn parse_kid(self) -> Option<[u8; 16]> {
        match self.len() {
            16 => {
                let mut out = [0u8; 16];
                out.copy_from_slice(self);
                Some(out)
            }
            32 => decode_hex_kid(self),
            _ => None,
        }
    }
}

impl ParseKid for [u8; 16] {
    fn parse_kid(self) -> Option<[u8; 16]> {
        Some(self)
    }
}

impl<T: ParseKid + Clone> ParseKid for &T {
    fn parse_kid(self) -> Option<[u8; 16]> {
        self.clone().parse_kid()
    }
}

#[cfg(test)]
mod tests {
    use hex_literal::hex;

    use super::*;

    #[test]
    fn eq_ignore_case_matching() {
        assert!(eq_ignore_ascii_case(b"hello", b"HELLO"));
        assert!(eq_ignore_ascii_case(b"Hello", b"hELLO"));
        assert!(eq_ignore_ascii_case(b"", b""));
        assert!(eq_ignore_ascii_case(b"123", b"123"));
    }

    #[test]
    fn eq_ignore_case_mismatch() {
        assert!(!eq_ignore_ascii_case(b"a", b"b"));
        assert!(!eq_ignore_ascii_case(b"ab", b"a"));
        assert!(!eq_ignore_ascii_case(b"a", b"ab"));
    }

    #[test]
    fn trim_empty_and_whitespace_only() {
        assert_eq!(trim_ascii(b""), b"");
        assert_eq!(trim_ascii(b"   "), b"");
        assert_eq!(trim_ascii(b"\t\n\r "), b"");
    }

    #[test]
    fn trim_leading_and_trailing() {
        assert_eq!(trim_ascii(b"  hello  "), b"hello");
        assert_eq!(trim_ascii(b"\thello\n"), b"hello");
        assert_eq!(trim_ascii(b"hello"), b"hello");
    }

    #[test]
    fn trim_preserves_inner_whitespace() {
        assert_eq!(trim_ascii(b"  a b  "), b"a b");
    }

    #[test]
    fn parse_kid_from_hex_str() {
        let kid = parse_kid("00000000000000000000000000000001").unwrap();
        assert_eq!(kid, hex!("00000000000000000000000000000001"));
    }

    #[test]
    fn parse_kid_from_hex_str_mixed_case() {
        let kid = parse_kid("aaBBccDD11223344aaBBccDD11223344").unwrap();
        assert_eq!(kid, hex!("aabbccdd11223344aabbccdd11223344"));
    }

    #[test]
    fn parse_kid_from_string() {
        let s = String::from("abcdef01234567890abcdef012345678");
        assert_eq!(
            parse_kid(&s).unwrap(),
            hex!("abcdef01234567890abcdef012345678")
        );
        assert_eq!(
            parse_kid(s).unwrap(),
            hex!("abcdef01234567890abcdef012345678")
        );
    }

    #[test]
    fn parse_kid_from_raw_bytes() {
        let bytes = hex!("00112233445566778899aabbccddeeff");
        let kid = parse_kid(bytes.as_slice()).unwrap();
        assert_eq!(kid, bytes);
    }

    #[test]
    fn parse_kid_from_hex_bytes() {
        let kid = parse_kid(b"00112233445566778899aabbccddeeff".as_slice()).unwrap();
        assert_eq!(kid, hex!("00112233445566778899aabbccddeeff"));
    }

    #[test]
    fn parse_kid_from_array() {
        let arr: [u8; 16] = hex!("00112233445566778899aabbccddeeff");
        assert_eq!(parse_kid(arr).unwrap(), arr);
        #[allow(clippy::needless_borrows_for_generic_args)]
        {
            assert_eq!(parse_kid(&arr).unwrap(), arr);
        }
    }

    #[test]
    fn parse_kid_invalid() {
        assert_eq!(parse_kid("too_short"), None);
        assert_eq!(parse_kid(""), None);
        assert_eq!(parse_kid("zz000000000000000000000000000000"), None);
        assert_eq!(parse_kid([0u8; 15].as_slice()), None);
        assert_eq!(parse_kid([0u8; 17].as_slice()), None);
    }
}
