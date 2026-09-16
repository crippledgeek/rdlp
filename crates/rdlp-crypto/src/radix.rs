//! Base-N integer encoding.

/// Number of digits in the base-36 alphabet (`0-9a-z`).
const BASE36: u32 = 36;

/// The base-36 alphabet, `0-9` then `a-z`, indexed by remainder.
const ALPHA: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Encode `n` as a base-36 string using the `0-9a-z` alphabet.
///
/// # Panics
///
/// Never panics: `bytes` is built exclusively from the ASCII-only `ALPHA`
/// slice, so the internal UTF-8 conversion always succeeds.
#[must_use]
pub fn to_base36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut bytes = Vec::with_capacity(8);
    while n > 0 {
        let digit = usize::try_from(n % BASE36).unwrap_or(0);
        if let Some(&b) = ALPHA.get(digit) {
            bytes.push(b);
        }
        n /= BASE36;
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
    fn base36_u32_max() {
        assert_eq!(to_base36(u32::MAX), "1z141z3");
    }
}
