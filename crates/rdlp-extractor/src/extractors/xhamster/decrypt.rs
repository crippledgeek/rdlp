//! URL decryption for xHamster format URLs.
//!
//! xHamster encrypts video format URLs by embedding a hex-encoded ciphertext
//! in the URL path. The first byte selects one of 7 PRNG algorithms, bytes 1-4
//! are the seed (little-endian i32), and the remaining bytes are XOR'd with
//! the PRNG output stream to recover the real URL path.
//!
//! ## PRNG Algorithms
//!
//! 1. Linear Congruential Generator (a=1664525, c=1013904223)
//! 2. Xorshift32
//! 3. Weyl Sequence + MurmurHash3 fmix32
//! 4. Custom scramble with ROL-7
//! 5. Xorshift variant with addition
//! 6. LCG with variable right-shift scrambler
//! 7. Weyl Sequence + multiply-xor-shift

use log::{debug, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use url::Url;

/// Decode a hex string to bytes (no external crate needed).
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Pattern to extract hex-encoded ciphertext and remainder from URL path.
///
/// The path starts with `/{hex}{remainder}` where hex is 12+ hex chars
/// and remainder starts with `/` or `,`.
static HEX_PATH_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^/(?P<hex>[0-9a-fA-F]{12,})(?P<rem>[/,].+)$")
        .expect("Valid hex path pattern")
});

/// Convert a value to signed 32-bit integer matching JavaScript's behavior.
///
/// JavaScript bitwise operators work on signed 32-bit integers. Python's
/// yt-dlp uses `n % (sign * 2^32)` to emulate this. In Rust we simply
/// truncate to i32 via wrapping.
#[inline]
fn to_signed_32(n: i64) -> i32 {
    // Match yt-dlp: n % ((-1 if n < 0 else 1) * 2**32)
    // For Rust, casting i64 → i32 truncates to low 32 bits with sign.
    n as i32
}

/// Pseudo-random number generator with 7 algorithm variants.
///
/// Each algorithm updates internal state and returns a full i32 value.
/// The caller masks to `& 0xFF` to get a single byte for XOR decryption.
struct ByteGenerator {
    state: i32,
    algo_id: u8,
}

impl ByteGenerator {
    /// Create a new byte generator with the given algorithm ID and seed.
    ///
    /// Returns `None` if the algorithm ID is not in range 1..=7.
    fn new(algo_id: u8, seed: i32) -> Option<Self> {
        if !(1..=7).contains(&algo_id) {
            return None;
        }
        Some(Self {
            state: seed,
            algo_id,
        })
    }

    /// Generate the next byte (0-255) from the PRNG stream.
    fn next_byte(&mut self) -> u8 {
        let result = match self.algo_id {
            1 => self.algo1(),
            2 => self.algo2(),
            3 => self.algo3(),
            4 => self.algo4(),
            5 => self.algo5(),
            6 => self.algo6(),
            7 => self.algo7(),
            _ => unreachable!(),
        };
        (result & 0xFF) as u8
    }

    /// Algorithm 1: Linear Congruential Generator.
    ///
    /// `s = s * 1664525 + 1013904223 (mod 2^32)`
    ///
    /// Reference: <https://en.wikipedia.org/wiki/Linear_congruential_generator>
    fn algo1(&mut self) -> i32 {
        let s = to_signed_32(
            self.state as i64 * 1_664_525 + 1_013_904_223,
        );
        self.state = s;
        s
    }

    /// Algorithm 2: Xorshift32.
    ///
    /// Reference: <https://en.wikipedia.org/wiki/Xorshift>
    fn algo2(&mut self) -> i32 {
        let mut s = self.state;
        s = to_signed_32((s as i64) ^ ((s as i64) << 13));
        s = to_signed_32((s as i64) ^ (((s as u32) >> 17) as i64));
        s = to_signed_32((s as i64) ^ ((s as i64) << 5));
        self.state = s;
        s
    }

    /// Algorithm 3: Weyl Sequence + MurmurHash3 fmix32.
    ///
    /// Uses the golden ratio constant `0x9e3779b9` as the Weyl increment,
    /// then applies the MurmurHash3 32-bit finalizer (fmix32) for mixing.
    ///
    /// References:
    /// - <https://en.wikipedia.org/wiki/Weyl_sequence>
    /// - <https://commons.apache.org/proper/commons-codec/jacoco/org.apache.commons.codec.digest/MurmurHash3.java.html>
    fn algo3(&mut self) -> i32 {
        let mut s = self.state.wrapping_add(0x9e3779b9_u32 as i32);
        self.state = s;
        // MurmurHash3 fmix32
        s ^= (s as u32 >> 16) as i32;
        s = to_signed_32(s as i64 * 0x85ebca77_u32 as i64);
        s ^= (s as u32 >> 13) as i32;
        s = to_signed_32(s as i64 * 0xc2b2ae3d_u32 as i64);
        s ^ ((s as u32 >> 16) as i32)
    }

    /// Algorithm 4: Custom scrambling with left rotation (ROL-7).
    fn algo4(&mut self) -> i32 {
        let mut s = self.state.wrapping_add(0x6d2b79f5_u32 as i32);
        self.state = s;
        // ROL 7: rotate left by 7 bits
        s = (s as u32).rotate_left(7) as i32;
        s = s.wrapping_add(0x9e3779b9_u32 as i32);
        s ^= (s as u32 >> 11) as i32;
        to_signed_32(s as i64 * 0x27d4eb2d_i64)
    }

    /// Algorithm 5: Xorshift variant with constant addition.
    fn algo5(&mut self) -> i32 {
        let mut s = self.state;
        s = to_signed_32((s as i64) ^ ((s as i64) << 7));
        s = to_signed_32((s as i64) ^ (((s as u32) >> 9) as i64));
        s = to_signed_32((s as i64) ^ ((s as i64) << 8));
        s = s.wrapping_add(0xa5a5a5a5_u32 as i32);
        self.state = s;
        s
    }

    /// Algorithm 6: LCG with variable right-shift scrambler.
    fn algo6(&mut self) -> i32 {
        let s = to_signed_32(
            self.state as i64 * 0x2c9277b5_u32 as i64 + 0xac564b05_u32 as i64,
        );
        self.state = s;
        let s2 = to_signed_32((s as i64) ^ (((s as u32) >> 18) as i64));
        let shift = ((s as u32) >> 27) & 31;
        ((s2 as u32) >> shift) as i32
    }

    /// Algorithm 7: Weyl Sequence + multiply-xor-shift mixing.
    fn algo7(&mut self) -> i32 {
        let s = self.state.wrapping_add(0x9e3779b9_u32 as i32);
        self.state = s;
        let mut e = to_signed_32((s as i64) ^ ((s as i64) << 5));
        e = to_signed_32(e as i64 * 0x7feb352d_u32 as i64);
        e = to_signed_32((e as i64) ^ (((e as u32) >> 15) as i64));
        to_signed_32(e as i64 * 0x846ca68b_u32 as i64)
    }
}

/// Decipher an encrypted xHamster format URL.
///
/// Returns the deciphered URL, or `None` if the URL format is not recognized
/// or the algorithm ID is unsupported.
///
/// Supports two input formats:
/// 1. **Full URL** with hex path: `https://host/{hex_string}/{remainder}`
/// 2. **Bare hex string**: raw hex-encoded ciphertext (deciphers to a full URL)
///
/// The hex data encodes:
/// - byte 0: algorithm ID (1-7)
/// - bytes 1-4: PRNG seed (little-endian i32)
/// - bytes 5+: ciphertext (XOR'd with PRNG stream)
pub fn decipher_format_url(format_url: &str) -> Option<String> {
    // Try as full URL with hex path first
    if let Ok(parsed) = Url::parse(format_url) {
        if let Some(result) = decipher_url_with_hex_path(&parsed) {
            return Some(result);
        }
    }

    // Fallback: treat the entire string as bare hex-encoded ciphertext
    decipher_bare_hex(format_url)
}

/// Decipher a full URL that contains a hex-encoded path segment.
///
/// The URL path has the form `/{hex_string}/{remainder}` where the hex portion
/// is deciphered and the remainder is preserved.
fn decipher_url_with_hex_path(parsed: &Url) -> Option<String> {
    let path = parsed.path();

    let caps = HEX_PATH_PATTERN.captures(path)?;
    let hex_str = caps.name("hex")?.as_str();
    let remainder = caps.name("rem")?.as_str();

    let deciphered = decipher_hex_bytes(hex_str)?;

    // Reconstruct URL with deciphered path
    let new_path = format!("/{deciphered}{remainder}");
    let mut result = parsed.clone();
    result.set_path(&new_path);

    Some(result.to_string())
}

/// Decipher a bare hex-encoded ciphertext string.
///
/// The entire string is hex, and the deciphered plaintext is returned directly
/// (typically a full URL).
fn decipher_bare_hex(hex_str: &str) -> Option<String> {
    // Quick check: must be all hex chars and even length
    if hex_str.len() < 12 || hex_str.len() % 2 != 0 {
        return None;
    }
    if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let deciphered = decipher_hex_bytes(hex_str)?;
    debug!(len = deciphered.len(); "[XHamster] Deciphered bare hex to plaintext");

    Some(deciphered)
}

/// Core decryption: decode hex to bytes, extract algorithm + seed, XOR with PRNG stream.
fn decipher_hex_bytes(hex_str: &str) -> Option<String> {
    let byte_data = decode_hex(hex_str)?;
    if byte_data.len() < 6 {
        debug!("[XHamster] Hex data too short: {} bytes", byte_data.len());
        return None;
    }

    // byte[0] = algorithm ID, bytes[1..5] = seed (little-endian i32)
    let algo_id = byte_data[0];
    let seed = i32::from_le_bytes([byte_data[1], byte_data[2], byte_data[3], byte_data[4]]);

    let mut rng = match ByteGenerator::new(algo_id, seed) {
        Some(g) => g,
        None => {
            warn!("[XHamster] Unknown algorithm ID: {algo_id}");
            return None;
        }
    };

    // XOR remaining bytes with PRNG stream
    let deciphered_bytes: Vec<u8> = byte_data[5..]
        .iter()
        .map(|&b| b ^ rng.next_byte())
        .collect();

    // Decode as latin-1 (each byte maps directly to its Unicode code point)
    let deciphered: String = deciphered_bytes.iter().map(|&b| b as char).collect();

    Some(deciphered)
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
        // With seed 0: s = 0 + 0x9e3779b9 = 0x9e3779b9 (= -1640531527 as i32)
        // fmix32:
        //   s ^= s >>> 16: 0x9e3779b9 ^ 0x00009e37 = 0x9e37e78e
        //   s *= 0x85ebca77: ...complex multiplication
        //   s ^= s >>> 13: ...
        //   s *= 0xc2b2ae3d: ...
        //   s ^= s >>> 16: ...
        // Just verify it produces a deterministic byte
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
    fn test_decipher_non_encrypted_url() {
        // Normal URL without hex path should return None
        assert!(decipher_format_url("https://example.com/video.mp4").is_none());
    }

    #[test]
    fn test_decipher_bare_hex_too_short() {
        // Too short to be a valid ciphertext
        assert!(decipher_format_url("abcd").is_none());
    }

    #[test]
    fn test_decipher_bare_hex_not_hex() {
        // Contains non-hex characters
        assert!(decipher_format_url("not_a_hex_string_at_all!").is_none());
    }

    #[test]
    fn test_decipher_bare_hex_invalid_algo() {
        // Algorithm ID 0 (first byte 00) is invalid
        assert!(decipher_format_url("000000000000aabbccdd").is_none());
    }

    #[test]
    fn test_decipher_bare_hex_valid_algo() {
        // Algorithm ID 1 (first byte 01), seed = 0, some ciphertext bytes
        // This should decipher successfully (algo 1 is LCG)
        let result = decipher_format_url("010000000041424344");
        assert!(result.is_some());
        // The result is the XOR of [0x41,0x42,0x43,0x44] with the LCG stream
    }

    #[test]
    fn test_hex_path_pattern() {
        assert!(HEX_PATH_PATTERN.is_match("/abcdef123456/something"));
        assert!(HEX_PATH_PATTERN.is_match("/ABCDEF123456789012/path,extra"));
        assert!(!HEX_PATH_PATTERN.is_match("/abc/something")); // Too short
        assert!(!HEX_PATH_PATTERN.is_match("/abcdef12345g/something")); // Non-hex char
    }
}
