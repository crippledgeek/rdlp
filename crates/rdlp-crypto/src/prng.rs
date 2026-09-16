//! Pseudo-random number generator implementations.
//!
//! This module provides PRNG algorithms shaped to match JavaScript-emulation
//! ciphers: seeded byte generators that reproduce a target's `Math.random`-style
//! or hand-rolled PRNG bit-for-bit.
//!
//! Every step here is public as "a number plus its parameters" — `lcg_step`,
//! `weyl_step`, `xorshift`, `rotate_scramble`, `fmix32`, `pcg_xsh_rs`, and
//! `mxs_mix` — so a site wiring a *new* JS-emulating cipher composes them
//! directly instead of re-deriving the arithmetic. [`ByteGenerator`] is the
//! composed 7-algorithm façade a site's cipher wiring drives.
//!
//! ## JavaScript Integer Semantics
//!
//! All arithmetic here intentionally emulates JavaScript's 32-bit signed integer
//! behavior via [`crate::js_int::to_signed_32`] (re-exported here as
//! [`to_signed_32`] for call-site brevity — every function in this module needs
//! it). Clippy warnings about sign-change and truncation casts are suppressed
//! per-function with explanatory comments; they are intentional.

pub use crate::js_int::to_signed_32;

// =============================================================================
// Constants
// =============================================================================

/// Golden ratio constant (2^32 / φ), commonly used in hash functions.
const PHI: u32 = 0x9e37_79b9;

// MurmurHash3 fmix32 constants
// Reference: https://github.com/aappleby/smhasher/blob/master/src/MurmurHash3.cpp
const FMIX32_C1: u32 = 0x85eb_ca77;
const FMIX32_C2: u32 = 0xc2b2_ae3d;

// Numerical Recipes LCG constants (Knuth MMIX)
const LCG_MULT: u32 = 1_664_525;
const LCG_INC: u32 = 1_013_904_223;

// PCG-style LCG constants
const PCG_MULT: u32 = 0x2c92_77b5;
const PCG_INC: u32 = 0xac56_4b05;

// Algorithm-specific constants
const WEYL_ROL_INC: u32 = 0x6d2b_79f5;
const ROL_SCRAMBLE_MULT: u32 = 0x27d4_eb2d;
const XORSHIFT_ADD_CONST: u32 = 0xa5a5_a5a5;
const MXS_MULT1: u32 = 0x7feb_352d;
const MXS_MULT2: u32 = 0x846c_a68b;

/// A left-rotation amount valid for [`rotate_scramble`] (and any `u32::rotate_left` caller).
///
/// `rotate_left` **wraps** its shift at 32 bits rather than rejecting an
/// out-of-range value (`x.rotate_left(35) == x.rotate_left(3)`), which would
/// silently alias two different algorithm parameters onto the same behavior —
/// this newtype rejects that at construction instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotation(u32);

impl Rotation {
    /// Validate `amount` is in `0..32` (`u32::rotate_left`'s wrap boundary).
    /// Returns `None` outside that range.
    #[must_use]
    pub const fn new(amount: u32) -> Option<Self> {
        if amount < 32 {
            Some(Self(amount))
        } else {
            None
        }
    }
}

/// The ROL-7 rotation [`ByteGenerator::weyl_rol7`] scrambles with.
const ROL7: Rotation = match Rotation::new(7) {
    Some(r) => r,
    None => panic!("7 is in 0..32"),
};

/// The three left/right/left shift amounts a [`xorshift`] variant uses.
///
/// Validated once at construction rather than as three loose `u8` parameters
/// (see `limit-function-arguments`: a cohesive triple of shift amounts is
/// exactly the "data clump" the rule asks to be grouped). `u32::rotate`-style
/// wraparound applies here too, hence the same `< 32` bound as [`Rotation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XorshiftShifts {
    left1: u8,
    right: u8,
    left2: u8,
}

impl XorshiftShifts {
    /// Validate all three shifts are in `0..32`. Returns `None` otherwise.
    #[must_use]
    pub const fn new(left1: u8, right: u8, left2: u8) -> Option<Self> {
        if left1 < 32 && right < 32 && left2 < 32 {
            Some(Self {
                left1,
                right,
                left2,
            })
        } else {
            None
        }
    }
}

/// Shift triple for [`ByteGenerator::xorshift32`] (algorithm 2).
const XORSHIFT32_SHIFTS: XorshiftShifts = match XorshiftShifts::new(13, 17, 5) {
    Some(s) => s,
    None => panic!("13, 17, 5 are all in 0..32"),
};

/// Shift triple for [`ByteGenerator::xorshift_add`] (algorithm 5).
const XORSHIFT_ADD_SHIFTS: XorshiftShifts = match XorshiftShifts::new(7, 9, 8) {
    Some(s) => s,
    None => panic!("7, 9, 8 are all in 0..32"),
};

/// Xorshift with a configurable, validated shift triple.
///
/// Reference: <https://en.wikipedia.org/wiki/Xorshift>
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
#[inline]
#[must_use]
pub fn xorshift(state: i32, shifts: XorshiftShifts) -> i32 {
    let mut x = state;
    x = to_signed_32(i64::from(x) ^ (i64::from(x) << shifts.left1));
    x = to_signed_32(i64::from(x) ^ ((x as u32 >> shifts.right) as i64));
    to_signed_32(i64::from(x) ^ (i64::from(x) << shifts.left2))
}

/// Linear Congruential Generator step: `s * multiplier + increment`, in JS i32
/// semantics. The multiplier/increment pairs are the caller's — Numerical
/// Recipes, PCG, glibc are all this shape.
///
/// Reference: <https://en.wikipedia.org/wiki/Linear_congruential_generator>
#[allow(clippy::cast_sign_loss, clippy::cast_lossless)]
#[inline]
#[must_use]
pub fn lcg_step(s: i32, multiplier: u32, increment: u32) -> i32 {
    to_signed_32(i64::from(s) * i64::from(multiplier) + i64::from(increment))
}

/// Weyl sequence step: add irrational constant to state.
///
/// Reference: <https://en.wikipedia.org/wiki/Weyl_sequence>
#[allow(clippy::cast_possible_wrap)]
#[inline]
#[must_use]
pub const fn weyl_step(s: i32, increment: u32) -> i32 {
    s.wrapping_add(increment as i32)
}

/// `MurmurHash3` 32-bit finalizer (fmix32).
///
/// Reference: <https://en.wikipedia.org/wiki/MurmurHash>
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]
#[inline]
#[must_use]
pub fn fmix32(mut s: i32) -> i32 {
    s ^= (s as u32 >> 16) as i32;
    s = to_signed_32(i64::from(s) * i64::from(FMIX32_C1));
    s ^= (s as u32 >> 13) as i32;
    s = to_signed_32(i64::from(s) * i64::from(FMIX32_C2));
    s ^ ((s as u32 >> 16) as i32)
}

/// Rotate-left scrambler: ROL + add + xor-shift + multiply.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
#[inline]
#[must_use]
pub const fn rotate_scramble(s: i32, rotation: Rotation) -> i32 {
    let mut x = (s as u32).rotate_left(rotation.0) as i32;
    x = x.wrapping_add(PHI as i32);
    x ^= (x as u32 >> 11) as i32;
    // Intentional: JS-emulation truncation from i64 to i32
    (i64::wrapping_mul(x as i64, ROL_SCRAMBLE_MULT as i64)) as i32
}

/// PCG's XSH-RS output permutation, lifted out of `ByteGenerator::lcg_pcg`.
///
/// Xorshift, then a variable right-shift keyed off the input's high bits, so
/// any LCG-based generator can reuse the same output function.
///
/// Reference: <https://en.wikipedia.org/wiki/Permuted_congruential_generator>
/// and the PCG paper §6.3.1 (O'Neill, 2014).
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
#[inline]
#[must_use]
pub fn pcg_xsh_rs(state: i32) -> i32 {
    let s2 = to_signed_32(i64::from(state) ^ i64::from(state as u32 >> 18));
    let shift = (state as u32 >> 27) & 31;
    (s2 as u32 >> shift) as i32
}

/// Multiply-xor-shift (MXS) mixer, lifted out of `ByteGenerator::weyl_mxs`.
///
/// Xor-shift, multiply, xor-shift, multiply, with the two multipliers as
/// parameters so a different MXS instance can reuse the same mixing shape.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
#[inline]
#[must_use]
pub fn mxs_mix(state: i32, mult1: u32, mult2: u32) -> i32 {
    // All casts are intentional JS-emulation truncation
    let mut x = to_signed_32(i64::from(state) ^ (i64::from(state) << 5));
    x = to_signed_32(i64::from(x) * i64::from(mult1));
    x = to_signed_32(i64::from(x) ^ i64::from(x as u32 >> 15));
    to_signed_32(i64::from(x) * i64::from(mult2))
}

/// Pseudo-random number generator with 7 algorithm variants.
///
/// Each algorithm updates internal state and returns a full i32 value.
/// The caller masks to `& 0xFF` to get a single byte for XOR decryption.
/// `algo_id` uses the same numbering 1..=7 as the originating scheme; a
/// caller passes the id it read from its payload.
///
/// ## Algorithms
///
/// 1. `lcg` - Linear Congruential Generator (Numerical Recipes)
/// 2. `xorshift32` - Xorshift32 (shifts: 13, 17, 5)
/// 3. `weyl_fmix32` - Weyl Sequence + `MurmurHash3` fmix32
/// 4. `weyl_rol7` - Weyl Sequence + ROL-7 scrambling
/// 5. `xorshift_add` - Xorshift variant with constant addition
/// 6. `lcg_pcg` - LCG with PCG-style variable right-shift scrambler
/// 7. `weyl_mxs` - Weyl Sequence + multiply-xor-shift
pub struct ByteGenerator {
    state: i32,
    algo_id: u8,
}

impl ByteGenerator {
    /// Create a new byte generator with the given algorithm ID and seed.
    ///
    /// Returns `None` if the algorithm ID is not in range 1..=7.
    ///
    /// # Arguments
    ///
    /// * `algo_id` - Algorithm identifier (1-7)
    /// * `seed` - Initial PRNG state (little-endian i32 from ciphertext)
    #[must_use]
    pub fn new(algo_id: u8, seed: i32) -> Option<Self> {
        if !(1..=7).contains(&algo_id) {
            return None;
        }
        Some(Self {
            state: seed,
            algo_id,
        })
    }

    /// Generate the next byte (0-255) from the PRNG stream.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn next_byte(&mut self) -> u8 {
        let result = match self.algo_id {
            1 => self.lcg(),
            2 => self.xorshift32(),
            3 => self.weyl_fmix32(),
            4 => self.weyl_rol7(),
            5 => self.xorshift_add(),
            6 => self.lcg_pcg(),
            7 => self.weyl_mxs(),
            _ => unreachable!(),
        };
        (result & 0xFF) as u8
    }

    /// Linear Congruential Generator (Numerical Recipes).
    ///
    /// Reference: <https://en.wikipedia.org/wiki/Linear_congruential_generator>
    fn lcg(&mut self) -> i32 {
        self.state = lcg_step(self.state, LCG_MULT, LCG_INC);
        self.state
    }

    /// Xorshift32 (shifts: 13, 17, 5).
    ///
    /// Reference: <https://en.wikipedia.org/wiki/Xorshift>
    fn xorshift32(&mut self) -> i32 {
        self.state = xorshift(self.state, XORSHIFT32_SHIFTS);
        self.state
    }

    /// Weyl Sequence + `MurmurHash3` fmix32.
    ///
    /// Uses the golden ratio constant (`PHI`) as the Weyl increment,
    /// then applies the `MurmurHash3` 32-bit finalizer (fmix32) for mixing.
    fn weyl_fmix32(&mut self) -> i32 {
        self.state = weyl_step(self.state, PHI);
        fmix32(self.state)
    }

    /// Weyl Sequence + ROL-7 scrambling.
    const fn weyl_rol7(&mut self) -> i32 {
        self.state = weyl_step(self.state, WEYL_ROL_INC);
        rotate_scramble(self.state, ROL7)
    }

    /// Xorshift variant with constant addition (shifts: 7, 9, 8).
    #[allow(clippy::cast_possible_wrap)]
    fn xorshift_add(&mut self) -> i32 {
        self.state =
            xorshift(self.state, XORSHIFT_ADD_SHIFTS).wrapping_add(XORSHIFT_ADD_CONST as i32);
        self.state
    }

    /// LCG with PCG-style variable right-shift scrambler.
    fn lcg_pcg(&mut self) -> i32 {
        self.state = lcg_step(self.state, PCG_MULT, PCG_INC);
        pcg_xsh_rs(self.state)
    }

    /// Weyl Sequence + multiply-xor-shift (MXS) mixing.
    fn weyl_mxs(&mut self) -> i32 {
        self.state = weyl_step(self.state, PHI);
        mxs_mix(self.state, MXS_MULT1, MXS_MULT2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_validates_the_rotate_left_wrap_boundary() {
        assert!(Rotation::new(0).is_some());
        assert!(Rotation::new(31).is_some());
        assert!(Rotation::new(32).is_none());
        assert!(Rotation::new(u32::MAX).is_none());
    }

    #[test]
    fn xorshift_shifts_validates_all_three_amounts() {
        assert!(XorshiftShifts::new(0, 0, 0).is_some());
        assert!(XorshiftShifts::new(31, 31, 31).is_some());
        assert!(XorshiftShifts::new(32, 0, 0).is_none());
        assert!(XorshiftShifts::new(0, 32, 0).is_none());
        assert!(XorshiftShifts::new(0, 0, 32).is_none());
    }

    #[test]
    fn pcg_xsh_rs_matches_reference_value() {
        // Reference value computed independently (Python, matching Rust's
        // i32/u32 wrapping semantics) for state = 0x1234_5678, before this
        // function existed — pcg_xsh_rs is a pure extraction of lcg_pcg's
        // output-permutation body, so this pins the composition is unchanged.
        assert_eq!(pcg_xsh_rs(0x1234_5678), 76_354_749);
    }

    #[test]
    fn mxs_mix_matches_reference_value() {
        // Reference value computed independently for state = 0x1234_5678 with
        // this module's own MXS_MULT1/MXS_MULT2 — mxs_mix is a pure extraction
        // of weyl_mxs's mixing body.
        assert_eq!(mxs_mix(0x1234_5678, MXS_MULT1, MXS_MULT2), -1_133_639_945);
    }

    #[test]
    fn test_byte_generator_algo1_lcg() {
        // LCG: s = s * 1664525 + 1013904223
        let mut rng = ByteGenerator::new(1, 0).unwrap();
        // With seed 0: 0 * 1664525 + 1013904223 = 1013904223
        let byte = rng.next_byte();
        // 1013904223 & 0xFF = 0x3C6EF35F & 0xFF = 0x5F = 95
        assert_eq!(byte, 0x5F);
    }

    #[test]
    fn test_byte_generator_algo2_xorshift() {
        let mut rng = ByteGenerator::new(2, 1).unwrap();
        // xorshift32 starting from seed 1
        let byte = rng.next_byte();
        // s = 1 ^ (1 << 13) = 0x2001
        // s = 0x2001 ^ (0x2001 >> 17) = 0x2001 ^ 0 = 0x2001
        // s = 0x2001 ^ (0x2001 << 5) = 0x2001 ^ 0x40020 = 0x42021
        // 0x42021 & 0xFF = 0x21 = 33
        assert_eq!(byte, 0x21);
    }

    #[test]
    fn test_byte_generator_algo3_murmurhash3() {
        let mut rng = ByteGenerator::new(3, 0).unwrap();
        let byte = rng.next_byte();
        // Just verify it produces a deterministic value
        let _ = byte;
    }

    #[test]
    fn test_byte_generator_invalid_algo() {
        assert!(ByteGenerator::new(0, 0).is_none());
        assert!(ByteGenerator::new(8, 0).is_none());
        assert!(ByteGenerator::new(255, 0).is_none());
    }

    #[test]
    fn test_byte_generator_valid_algos() {
        for algo_id in 1..=7 {
            let rng = ByteGenerator::new(algo_id, 42);
            assert!(rng.is_some(), "Algorithm {algo_id} should be valid");
        }
    }

    #[test]
    fn test_byte_generator_deterministic() {
        // Same algo + seed must produce same sequence
        for algo_id in 1..=7 {
            let mut rng1 = ByteGenerator::new(algo_id, 12345).unwrap();
            let mut rng2 = ByteGenerator::new(algo_id, 12345).unwrap();
            for _ in 0..100 {
                assert_eq!(
                    rng1.next_byte(),
                    rng2.next_byte(),
                    "Algorithm {algo_id} not deterministic"
                );
            }
        }
    }

    #[test]
    fn test_byte_generator_reference_bytes_per_algorithm() {
        // One concrete byte per algorithm from seed 12345, pinned from the
        // pre-refactor implementation (before rotate_scramble/xorshift/
        // pcg_xsh_rs/mxs_mix were extracted as public compositions) — proves
        // the extraction changed no algorithm's output.
        let expected: [(u8, u8); 7] = [
            (1, 68),
            (2, 122),
            (3, 154),
            (4, 249),
            (5, 84),
            (6, 197),
            (7, 229),
        ];
        for (algo_id, expected_byte) in expected {
            let mut rng = ByteGenerator::new(algo_id, 12345).unwrap();
            assert_eq!(
                rng.next_byte(),
                expected_byte,
                "algorithm {algo_id} byte changed"
            );
        }
    }
}
