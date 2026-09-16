//! JavaScript `|0` integer coercion.

/// Convert a value to signed 32-bit integer matching JavaScript's behavior.
///
/// JavaScript bitwise operators coerce their operands to a signed 32-bit
/// integer (the `|0` idiom). Python's yt-dlp emulates this with
/// `n % (sign * 2^32)`; in Rust we truncate to `i32` via a plain `as` cast,
/// which matches the JS semantics exactly. It is the coercion every PRNG step
/// that computes in `i64` applies to its result, and it is not specific to any
/// one algorithm — hence its own module.
#[allow(clippy::cast_possible_truncation)]
#[inline]
#[must_use]
pub const fn to_signed_32(n: i64) -> i32 {
    n as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_signed_32() {
        assert_eq!(to_signed_32(0), 0);
        assert_eq!(to_signed_32(1), 1);
        assert_eq!(to_signed_32(-1), -1);
        assert_eq!(to_signed_32(0x7FFF_FFFF), 0x7FFF_FFFF);
        // Overflow wraps
        assert_eq!(to_signed_32(0x1_0000_0000), 0);
        assert_eq!(to_signed_32(0x1_0000_0001), 1);
    }
}
