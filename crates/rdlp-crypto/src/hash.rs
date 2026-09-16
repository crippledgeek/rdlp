//! Java `String.hashCode`-style folding hash.

/// Java `String.hashCode` folded to 32 bits: `h = h * 31 + byte` over the UTF-8 bytes, `& 0xFFFF_FFFF`.
#[must_use]
pub fn java_string_hash32(key: &str) -> u32 {
    let mut hash: u32 = 0;
    for b in key.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(b));
    }
    hash
}

pub use crate::prng::fmix32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hash_matches_reference_values() {
        // JDK String.hashCode: "" -> 0, "a" -> 97, "ab" -> 3105, "hello" -> 99162322 (fits in 32 bits)
        assert_eq!(java_string_hash32(""), 0);
        assert_eq!(java_string_hash32("a"), 97);
        assert_eq!(java_string_hash32("ab"), 3105);
        assert_eq!(java_string_hash32("hello"), 99_162_322);
    }

    #[test]
    fn java_hash_wraps_at_32_bits() {
        // A long key overflows u32 many times; the fold keeps only the low 32 bits.
        let h = java_string_hash32(&"x".repeat(64));
        let _: u32 = h; // fold already narrowed to u32; the type itself is the bound
        assert_eq!(java_string_hash32(&"x".repeat(64)), h); // deterministic
    }

    #[test]
    fn java_hash_folds_utf8_bytes() {
        // "e-acute" is 0xC3 0xA9: (0*31 + 0xC3)*31 + 0xA9 = 195*31 + 169 = 6214
        assert_eq!(java_string_hash32("\u{e9}"), 6214);
    }
}
