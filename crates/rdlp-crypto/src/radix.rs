//! Base-N integer encoding.

/// A validated encoding base — `2..=36`, the range the `0-9a-z` alphabet can represent.
///
/// Bare `u8` bases would let `to_radix` silently index past the alphabet
/// (base 37+) or divide by an unusable base (0 or 1); validating at
/// construction makes that state unrepresentable instead of a runtime bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Radix(u8);

impl Radix {
    /// Base 36 (`0-9a-z`) — the historical default this module shipped with.
    pub const BASE36: Self = match Self::new(36) {
        Some(r) => r,
        None => panic!("36 is in 2..=36"),
    };

    /// Validate `base` is in `2..=36` (the `0-9a-z` alphabet's range).
    /// Returns `None` outside that range.
    #[must_use]
    pub const fn new(base: u8) -> Option<Self> {
        if base >= 2 && base <= 36 {
            Some(Self(base))
        } else {
            None
        }
    }
}

/// The base-36 alphabet, `0-9` then `a-z`, indexed by remainder. Every
/// supported [`Radix`] (`2..=36`) uses a prefix of this same alphabet.
const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

// If ALPHA's length ever drops below Radix's upper bound, `to_radix` could
// silently index past ALPHA's end for a large base — this keeps them in
// lock-step so that can only ever be a compile error.
const _: () = assert!(ALPHA.len() == 36);

/// Encode `n` as a string in `base`, using the `0-9a-z` alphabet.
///
/// # Panics
///
/// Never panics: `bytes` is built exclusively from the ASCII-only `ALPHA`
/// slice, so the internal UTF-8 conversion always succeeds.
#[must_use]
pub fn to_radix(mut n: u32, base: Radix) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let base = u32::from(base.0);
    let mut bytes = Vec::with_capacity(8);
    while n > 0 {
        let digit = usize::try_from(n % base).unwrap_or(0);
        if let Some(&b) = ALPHA.get(digit) {
            bytes.push(b);
        }
        n /= base;
    }
    bytes.reverse();
    // INVARIANT: `bytes` is built exclusively from the ASCII-only `ALPHA` slice,
    // so it is always valid UTF-8.
    #[allow(clippy::expect_used)]
    String::from_utf8(bytes).expect("ascii")
}

/// Encode `n` as a base-36 string using the `0-9a-z` alphabet.
///
/// # Panics
///
/// Never panics: see [`to_radix`].
#[must_use]
pub fn to_base36(n: u32) -> String {
    to_radix(n, Radix::BASE36)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_validates_the_alphabet_bound() {
        assert!(Radix::new(1).is_none());
        assert!(Radix::new(37).is_none());
        assert!(Radix::new(0).is_none());
        assert!(Radix::new(2).is_some());
        assert!(Radix::new(36).is_some());
    }

    #[test]
    fn to_radix_hex_and_binary() {
        assert_eq!(to_radix(255, Radix::new(16).unwrap()), "ff");
        assert_eq!(to_radix(5, Radix::new(2).unwrap()), "101");
    }

    #[test]
    fn base36_small_values() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }

    #[test]
    fn base36_u32_max() {
        assert_eq!(to_base36(u32::MAX), "1z141z3");
    }
}
