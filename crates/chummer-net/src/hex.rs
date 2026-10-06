//! Lower-case hex for ids and keys.

pub(crate) fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0xf) as usize] as char);
    }
    s
}

/// Decodes exactly `N` bytes of hex (either case).
pub(crate) fn decode<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return None;
    }
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (nibble(s[2 * i])? << 4) | nibble(s[2 * i + 1])?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let b = [0u8, 1, 0xab, 0xff];
        let s = super::encode(&b);
        assert_eq!(s, "0001abff");
        assert_eq!(super::decode::<4>(&s), Some(b));
        assert_eq!(super::decode::<4>("0001ABFF"), Some(b));
        assert_eq!(super::decode::<4>("0001abf"), None);
        assert_eq!(super::decode::<4>("0001abfg"), None);
    }
}
