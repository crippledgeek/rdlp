//! URL decryption for XHamster format URLs.
//!
//! XHamster encrypts video format URLs by embedding a hex-encoded ciphertext
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
//!
//! ## Example
//!
//! ```rust
//! use rdlp_crypto::xhamster::decipher_format_url;
//!
//! // Bare hex-encoded ciphertext
//! if let Some(decrypted) = decipher_format_url("010000000041424344") {
//!     println!("Decrypted: {decrypted}");
//! }
//!
//! // Full URL with hex path
//! let url = "https://cdn.example.com/010000000041424344/,720p.mp4";
//! if let Some(decrypted) = decipher_format_url(url) {
//!     println!("Decrypted URL: {decrypted}");
//! }
//! ```

use log::{debug, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use url::Url;

use crate::prng::ByteGenerator;

/// Pattern to extract hex-encoded ciphertext and remainder from URL path.
///
/// The path starts with `/{hex}{remainder}` where hex is 12+ hex chars
/// and remainder starts with `/` or `,`.
static HEX_PATH_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^/(?P<hex>[0-9a-fA-F]{12,})(?P<rem>[/,].+)$").expect("Valid hex path pattern")
});

/// Decipher an encrypted XHamster format URL.
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
///
/// # Example
///
/// ```rust
/// use rdlp_crypto::xhamster::decipher_format_url;
///
/// // Algorithm ID 1 (LCG), seed = 0, some ciphertext
/// let result = decipher_format_url("010000000041424344");
/// assert!(result.is_some());
/// ```
#[must_use]
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
    let byte_data = hex::decode(hex_str).ok()?;
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
