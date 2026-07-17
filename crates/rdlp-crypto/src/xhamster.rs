//! URL decryption for `XHamster` format URLs.
//!
//! `XHamster` encrypts video format URLs by embedding a hex-encoded ciphertext
//! in the URL path. The first byte selects one of 7 PRNG algorithms, bytes 1-4
//! are the seed (little-endian i32), and the remaining bytes are XOR'd with
//! the PRNG output stream to recover the real URL path.
//!
//! ## PRNG Algorithms
//!
//! 1. Linear Congruential Generator (a=1664525, c=1013904223)
//! 2. Xorshift32
//! 3. Weyl Sequence + `MurmurHash3` fmix32
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

use log::{debug, trace};
use regex::Regex;
use std::sync::LazyLock;
use url::Url;

use crate::prng::ByteGenerator;

/// Minimum decoded length of a valid ciphertext payload: a 1-byte algorithm ID
/// plus a 4-byte little-endian seed (the header) plus at least one ciphertext
/// byte. Inputs that hex-decode to fewer bytes cannot be valid ciphertext for
/// *any* algorithm — there is no room for the header — so decryption rejects
/// them before an algorithm is ever selected. A bare-hex string therefore needs
/// at least `MIN_CIPHERTEXT_BYTES * 2` hex characters.
const MIN_CIPHERTEXT_BYTES: usize = 6;

/// Pattern to extract hex-encoded ciphertext and remainder from URL path.
///
/// The path starts with `/{hex}{remainder}` where hex is 12+ hex chars
/// and remainder starts with `/` or `,`.
#[allow(clippy::expect_used)] // LazyLock<Regex>: hardcoded literal, panic = programming error caught at first use
static HEX_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/(?P<hex>[0-9a-fA-F]{12,})(?P<rem>[/,].+)$").expect("Valid hex path pattern")
});

/// Decipher an encrypted `XHamster` format URL.
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
    if let Ok(parsed) = Url::parse(format_url)
        && let Some(result) = decipher_url_with_hex_path(&parsed)
    {
        return Some(result);
    }

    // Fallback: treat the entire string as bare hex-encoded ciphertext
    decipher_bare_hex(format_url)
}

/// Returns `true` if `format_url` is *structurally* capable of being valid
/// `XHamster` ciphertext.
///
/// Plausible means it carries a hex payload that decodes to at least
/// [`MIN_CIPHERTEXT_BYTES`] bytes (algorithm ID + seed + ≥1 ciphertext byte),
/// via either accepted input form (full URL with hex path, or bare hex).
///
/// This is deliberately weaker than [`decipher_format_url`]: it does **not**
/// inspect the algorithm ID, so a genuinely-rotated ciphertext whose algorithm
/// this port does not yet know still returns `true`. It exists so callers can
/// cheaply skip an expensive JS-engine decrypt fallback for inputs too short to
/// be ciphertext under *any* algorithm — e.g. the malformed sub-header
/// `fallback` fields in `xplayerSettings.sources` (issue #514) — where the JS
/// path would only fail after evaluating a large player bundle.
#[must_use]
pub fn could_be_ciphertext(format_url: &str) -> bool {
    // Path 1: a full URL whose path carries a hex segment (12+ hex chars, i.e.
    // ≥ MIN_CIPHERTEXT_BYTES bytes) — the rotated-algorithm URL a JS fallback
    // could still self-heal.
    if let Ok(parsed) = Url::parse(format_url)
        && HEX_PATH_PATTERN.is_match(parsed.path())
    {
        return true;
    }

    // Path 2: a bare hex string long enough to carry the algo+seed header.
    is_plausible_bare_hex(format_url)
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
    if !is_plausible_bare_hex(hex_str) {
        return None;
    }

    let deciphered = decipher_hex_bytes(hex_str)?;
    debug!(len = deciphered.len(); "[XHamster] Deciphered bare hex to plaintext");

    Some(deciphered)
}

/// Structural check for a bare hex ciphertext string: long enough to carry the
/// algo+seed header ([`MIN_CIPHERTEXT_BYTES`]), even length (whole bytes), and
/// all ASCII hex digits. Shared by [`decipher_bare_hex`] (the decrypt path) and
/// [`could_be_ciphertext`] (the plausibility gate) so both agree on exactly what
/// counts as a decodable bare-hex payload.
fn is_plausible_bare_hex(hex_str: &str) -> bool {
    hex_str.len() >= MIN_CIPHERTEXT_BYTES * 2
        && hex_str.len().is_multiple_of(2)
        && hex_str.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Core decryption: decode hex to bytes, extract algorithm + seed, XOR with PRNG stream.
// The `byte_data.len() < MIN_CIPHERTEXT_BYTES` guard above each index/slice makes
// these infallible; `indexing_slicing` cannot see through the early-return guard.
#[allow(clippy::indexing_slicing)]
fn decipher_hex_bytes(hex_str: &str) -> Option<String> {
    let byte_data = hex::decode(hex_str).ok()?;
    if byte_data.len() < MIN_CIPHERTEXT_BYTES {
        trace!("[XHamster] Hex data too short: {} bytes", byte_data.len());
        return None;
    }

    // byte[0] = algorithm ID, bytes[1..5] = seed (little-endian i32)
    let algo_id = byte_data[0];
    let seed = i32::from_le_bytes([byte_data[1], byte_data[2], byte_data[3], byte_data[4]]);

    let Some(mut rng) = ByteGenerator::new(algo_id, seed) else {
        debug!("[XHamster] Unknown algorithm ID: {algo_id}");
        return None;
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

    // =========================================================================
    // could_be_ciphertext — plausibility gate for the JS decrypt fallback (#514)
    // =========================================================================

    #[test]
    fn test_could_be_ciphertext_rejects_sub_header_fallback_field() {
        // The real malformed `fallback` field this gate targets: 10 hex chars =
        // 5 bytes, one short of the 6-byte algo+seed+cipher minimum. No algorithm
        // can decrypt it, so the JS fallback must be skipped.
        assert!(!could_be_ciphertext("02e7b7d11b"));
    }

    #[test]
    fn test_could_be_ciphertext_boundary_five_vs_six_bytes() {
        // Boundary of MIN_CIPHERTEXT_BYTES (6). 5 bytes (10 hex) is rejected;
        // exactly 6 bytes (12 hex) is accepted — a `>=`/`>` off-by-one flips one.
        assert!(!could_be_ciphertext("0011223344")); // 10 hex chars = 5 bytes
        assert!(could_be_ciphertext("001122334455")); // 12 hex chars = 6 bytes
    }

    #[test]
    fn test_could_be_ciphertext_unknown_algo_still_plausible() {
        // Algorithm ID 0 is unknown to this port, so decipher_format_url returns
        // None — but the input IS structurally valid ciphertext (9 bytes), so the
        // gate keeps the JS self-heal path open. Fallback must stay intact.
        assert!(decipher_format_url("000000000041424344").is_none());
        assert!(could_be_ciphertext("000000000041424344"));
    }

    #[test]
    fn test_could_be_ciphertext_rejects_odd_length_and_non_hex() {
        assert!(!could_be_ciphertext("00112233445")); // 11 chars = odd, not whole bytes
        assert!(!could_be_ciphertext("001122334z55")); // non-hex digit
        assert!(!could_be_ciphertext("")); // empty
    }

    #[test]
    fn test_could_be_ciphertext_full_url_with_hex_path() {
        // A rotated-algorithm URL form (12+ hex path segment) stays plausible.
        assert!(could_be_ciphertext(
            "https://cdn.example.com/000000000041424344/,720p.mp4"
        ));
        // A plain URL with no hex path segment is not ciphertext.
        assert!(!could_be_ciphertext("https://example.com/video.mp4"));
    }
}
