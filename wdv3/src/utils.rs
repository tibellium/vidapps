/// Const-compatible byte slice equality.
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

/// Const-compatible ASCII whitespace trimming (both ends).
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

/// Decode a single ASCII hex digit to its 4-bit value.
/// Returns `None` for non-hex characters.
pub(crate) const fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Const-compatible case-insensitive ASCII byte comparison.
/// Both slices must have the same length (caller must check).
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

#[cfg(test)]
mod tests {
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
}
