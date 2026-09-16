//! Java `String.hashCode`-style folding hash.

/// The `MurmurHash3` finalizer, reachable here as well as at
/// [`crate::prng::fmix32`]: it is a hash primitive in its own right (a site may
/// finalize a key with it and never touch a PRNG), so the `hash` module names it
/// too rather than sending a hash-only caller into `prng`.
pub use crate::prng::fmix32;

/// Java `String.hashCode` folded to 32 bits: `h = h * 31 + byte` over the UTF-8 bytes, `& 0xFFFF_FFFF`.
#[must_use]
pub fn java_string_hash32(key: &str) -> u32 {
    let mut hash: u32 = 0;
    for b in key.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(b));
    }
    hash
}

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
        // "polygenelubricants" is the well-known key whose JDK hashCode is
        // Integer.MIN_VALUE: the fold overflows 32 bits and only the low 32
        // survive. A 33-bit or unwrapped fold cannot produce this value.
        assert_eq!(java_string_hash32("polygenelubricants"), 0x8000_0000);
    }

    #[test]
    fn java_hash_has_the_classic_aa_bb_collision() {
        // 'A'*31 + 'a' == 'B'*31 + 'B' == 2112 — the textbook hashCode
        // collision, which only a multiplier of exactly 31 reproduces.
        assert_eq!(java_string_hash32("Aa"), 2112);
        assert_eq!(java_string_hash32("BB"), 2112);
    }

    #[test]
    fn java_hash_folds_utf8_bytes() {
        // "e-acute" is 0xC3 0xA9: (0*31 + 0xC3)*31 + 0xA9 = 195*31 + 169 = 6214
        assert_eq!(java_string_hash32("\u{e9}"), 6214);
    }
}
