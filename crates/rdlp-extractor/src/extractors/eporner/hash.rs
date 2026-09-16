//! EPorner hash decoder.
//!
//! The video page embeds a 32-char hex string (regex: `hash[:=] "<32 hex>"`).
//! The XHR endpoint expects that hash transformed via `calc_hash`:
//! split into 4×8-char chunks, parse each as hex → u32, base-36 encode,
//! concatenate with no separator.

use rdlp_crypto::radix::to_base36;

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

#[cfg(test)]
mod tests {
    use super::*;

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
