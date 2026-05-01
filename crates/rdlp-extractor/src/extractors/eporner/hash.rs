//! EPorner hash decoder.
//!
//! The video page embeds a 32-char hex string (regex: `hash[:=] "<32 hex>"`).
//! The XHR endpoint expects that hash transformed via `calc_hash`:
//! split into 4×8-char chunks, parse each as hex → u32, base-36 encode,
//! concatenate with no separator.

/// Transform the raw 32-char hex page hash into the value expected by the XHR endpoint.
///
/// Returns `None` if `raw` is not exactly 32 ASCII hex digits.
#[must_use]
pub fn calc_hash(raw: &str) -> Option<String> {
    if raw.len() != 32 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = String::with_capacity(32);
    for i in (0..32).step_by(8) {
        let chunk_u32 = u32::from_str_radix(&raw[i..i + 8], 16).ok()?;
        out.push_str(&to_base36(chunk_u32));
    }
    Some(out)
}

fn to_base36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut bytes = Vec::with_capacity(8);
    while n > 0 {
        bytes.push(ALPHA[(n % 36) as usize]);
        n /= 36;
    }
    bytes.reverse();
    // INVARIANT: `bytes` is built exclusively from the ASCII-only `ALPHA` slice,
    // so it is always valid UTF-8.
    #[allow(clippy::expect_used)]
    String::from_utf8(bytes).expect("ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36_small_values() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }

    #[test]
    fn calc_hash_rejects_wrong_length() {
        assert_eq!(calc_hash("abcd"), None);
        assert_eq!(calc_hash(&"a".repeat(33)), None);
    }

    #[test]
    fn calc_hash_rejects_non_hex() {
        assert_eq!(calc_hash(&"z".repeat(32)), None);
    }

    #[test]
    fn calc_hash_concatenates_base36_chunks() {
        let raw = "00000001000000010000000100000001";
        assert_eq!(calc_hash(raw).as_deref(), Some("1111"));
    }

    #[test]
    fn calc_hash_full_chunk() {
        // 0xffffffff = 4294967295 → base36 = "1z141z3"
        let raw = "ffffffff00000000ffffffff00000000";
        assert_eq!(calc_hash(raw).as_deref(), Some("1z141z301z141z30"));
    }
}
