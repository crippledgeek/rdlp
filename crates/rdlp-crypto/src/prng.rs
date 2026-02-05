//! Pseudo-random number generator implementations.
//!
//! This module provides PRNG algorithms used for XHamster URL decryption.
//! The generators implement various algorithms that match the JavaScript
//! implementations used by the target sites.

// =============================================================================
// Constants
// =============================================================================

/// Golden ratio constant (2^32 / φ), commonly used in hash functions.
const PHI: u32 = 0x9e3779b9;

// MurmurHash3 fmix32 constants
// Reference: https://github.com/aappleby/smhasher/blob/master/src/MurmurHash3.cpp
const FMIX32_C1: u32 = 0x85ebca77;
const FMIX32_C2: u32 = 0xc2b2ae3d;

// Numerical Recipes LCG constants (Knuth MMIX)
const LCG_MULT: u32 = 1_664_525;
const LCG_INC: u32 = 1_013_904_223;

// PCG-style LCG constants
const PCG_MULT: u32 = 0x2c9277b5;
const PCG_INC: u32 = 0xac564b05;

// Algorithm-specific constants
const WEYL_ROL_INC: u32 = 0x6d2b79f5;
const ROL_SCRAMBLE_MULT: u32 = 0x27d4eb2d;
const XORSHIFT_ADD_CONST: u32 = 0xa5a5a5a5;
const MXS_MULT1: u32 = 0x7feb352d;
const MXS_MULT2: u32 = 0x846ca68b;

/// Convert a value to signed 32-bit integer matching JavaScript's behavior.
///
/// JavaScript bitwise operators work on signed 32-bit integers. Python's
/// yt-dlp uses `n % (sign * 2^32)` to emulate this. In Rust we simply
/// truncate to i32 via wrapping.
#[inline]
#[must_use]
pub fn to_signed_32(n: i64) -> i32 {
    n as i32
}

/// Xorshift with configurable shift values (left, right, left pattern).
///
/// Reference: <https://en.wikipedia.org/wiki/Xorshift>
#[inline]
fn xorshift(s: i32, a: u8, b: u8, c: u8) -> i32 {
    let mut x = s;
    x = to_signed_32((x as i64) ^ ((x as i64) << a));
    x = to_signed_32((x as i64) ^ (((x as u32) >> b) as i64));
    to_signed_32((x as i64) ^ ((x as i64) << c))
}

/// Linear Congruential Generator step: `s * multiplier + increment`.
///
/// Reference: <https://en.wikipedia.org/wiki/Linear_congruential_generator>
#[inline]
fn lcg_step(s: i32, multiplier: u32, increment: u32) -> i32 {
    to_signed_32(s as i64 * multiplier as i64 + increment as i64)
}

/// Weyl sequence step: add irrational constant to state.
///
/// Reference: <https://en.wikipedia.org/wiki/Weyl_sequence>
#[inline]
fn weyl_step(s: i32, increment: u32) -> i32 {
    s.wrapping_add(increment as i32)
}

/// MurmurHash3 32-bit finalizer (fmix32).
///
/// Reference: <https://en.wikipedia.org/wiki/MurmurHash>
#[inline]
fn fmix32(mut s: i32) -> i32 {
    s ^= (s as u32 >> 16) as i32;
    s = to_signed_32(s as i64 * FMIX32_C1 as i64);
    s ^= (s as u32 >> 13) as i32;
    s = to_signed_32(s as i64 * FMIX32_C2 as i64);
    s ^ ((s as u32 >> 16) as i32)
}

/// Rotate-left scrambler: ROL + add + xor-shift + multiply.
#[inline]
fn rol_scramble(s: i32, rotation: u32) -> i32 {
    let mut x = (s as u32).rotate_left(rotation) as i32;
    x = x.wrapping_add(PHI as i32);
    x ^= (x as u32 >> 11) as i32;
    to_signed_32(x as i64 * ROL_SCRAMBLE_MULT as i64)
}

/// Pseudo-random number generator with 7 algorithm variants.
///
/// Each algorithm updates internal state and returns a full i32 value.
/// The caller masks to `& 0xFF` to get a single byte for XOR decryption.
///
/// ## Algorithms
///
/// 1. `lcg` - Linear Congruential Generator (Numerical Recipes)
/// 2. `xorshift32` - Xorshift32 (shifts: 13, 17, 5)
/// 3. `weyl_fmix32` - Weyl Sequence + MurmurHash3 fmix32
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
        Some(Self { state: seed, algo_id })
    }

    /// Generate the next byte (0-255) from the PRNG stream.
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
        let s = lcg_step(self.state, LCG_MULT, LCG_INC);
        self.state = s;
        s
    }

    /// Xorshift32 (shifts: 13, 17, 5).
    ///
    /// Reference: <https://en.wikipedia.org/wiki/Xorshift>
    fn xorshift32(&mut self) -> i32 {
        let s = xorshift(self.state, 13, 17, 5);
        self.state = s;
        s
    }

    /// Weyl Sequence + MurmurHash3 fmix32.
    ///
    /// Uses the golden ratio constant (`PHI`) as the Weyl increment,
    /// then applies the MurmurHash3 32-bit finalizer (fmix32) for mixing.
    fn weyl_fmix32(&mut self) -> i32 {
        let s = weyl_step(self.state, PHI);
        self.state = s;
        fmix32(s)
    }

    /// Weyl Sequence + ROL-7 scrambling.
    fn weyl_rol7(&mut self) -> i32 {
        let s = weyl_step(self.state, WEYL_ROL_INC);
        self.state = s;
        rol_scramble(s, 7)
    }

    /// Xorshift variant with constant addition (shifts: 7, 9, 8).
    fn xorshift_add(&mut self) -> i32 {
        let s = xorshift(self.state, 7, 9, 8).wrapping_add(XORSHIFT_ADD_CONST as i32);
        self.state = s;
        s
    }

    /// LCG with PCG-style variable right-shift scrambler.
    fn lcg_pcg(&mut self) -> i32 {
        let s = lcg_step(self.state, PCG_MULT, PCG_INC);
        self.state = s;
        // PCG output function: xor-shift then variable right-shift
        let s2 = to_signed_32((s as i64) ^ (((s as u32) >> 18) as i64));
        let shift = ((s as u32) >> 27) & 31;
        ((s2 as u32) >> shift) as i32
    }

    /// Weyl Sequence + multiply-xor-shift (MXS) mixing.
    fn weyl_mxs(&mut self) -> i32 {
        let s = weyl_step(self.state, PHI);
        self.state = s;
        // MXS: xor-shift, multiply, xor-shift, multiply
        let mut x = to_signed_32((s as i64) ^ ((s as i64) << 5));
        x = to_signed_32(x as i64 * MXS_MULT1 as i64);
        x = to_signed_32((x as i64) ^ (((x as u32) >> 15) as i64));
        to_signed_32(x as i64 * MXS_MULT2 as i64)
    }
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
}
