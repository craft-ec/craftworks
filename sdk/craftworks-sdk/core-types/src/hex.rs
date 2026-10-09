//! Hex: the one encoder and the one decoder.

/// Lower-case hex of `bytes`.
pub fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Hex of either case into bytes; `None` for an odd length or a non-hex character.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.chunks_exact(2).map(|p| Some(nibble(p[0])? << 4 | nibble(p[1])?)).collect()
}

/// Exactly `N` bytes of hex (`2N` characters, either case); `None` otherwise.
pub fn decode_array<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N {
        return None;
    }
    decode(s)?.try_into().ok()
}

/// Is `s` exactly `len` bytes in LOWER-case hex: the one spelling of an id whose
/// copies are compared as strings.
pub fn is_lower(s: &str, len: usize) -> bool {
    s.len() == 2 * len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses() {
        let b = [0x00, 0x0f, 0xa5, 0xff];
        assert_eq!(encode(&b), "000fa5ff");
        assert_eq!(decode("000fa5ff").unwrap(), b);
        assert_eq!(decode("000FA5FF").unwrap(), b);
        assert_eq!(decode_array::<4>("000fa5ff"), Some(b));
        assert_eq!(decode_array::<4>("000fa5"), None);
        assert_eq!(decode("0g"), None);
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("é1"), None);
        // STRICTER than the `u8::from_str_radix` copies it replaced, which took a
        // sign: "+f" parsed as 15. Hex has no sign.
        assert_eq!(decode("+f"), None);
        assert_eq!(decode_array::<1>("+f"), None);
        assert!(is_lower("000fa5ff", 4));
        assert!(!is_lower("000FA5FF", 4));
        assert!(!is_lower("000fa5", 4));
    }
}
