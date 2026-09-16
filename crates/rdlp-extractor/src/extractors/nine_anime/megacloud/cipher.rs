//! Custom Megacloud source decryption cipher.
//!
//! Implements a 3-layer reversible cipher used to decrypt encrypted HLS source
//! URLs returned by the Megacloud `getSources` API. Each layer applies (in
//! reverse order):
//!
//! 1. Character substitution via seeded Fisher-Yates shuffle
//! 2. Columnar transposition cipher reversal
//! 3. Seed-based character shift
//!
//! The key for each layer is derived from a combined key (megacloud key +
//! client key) via `keygen`.
//!
//! ## References
//!
//! Based on the `decryptSrc2` / `keygen2` / `seedShuffle2` /
//! `columnarCipher2` functions from the aniwatch project.

use anyhow::Context as _;
use rdlp_core::{RdlpError, Result};
use rdlp_crypto::hash::java_string_hash32;
use rdlp_crypto::prng::lcg_step;
use rdlp_crypto::shuffle::seeded_shuffle;
use rdlp_crypto::transposition::columnar_untranspose;

/// Number of decryption layers to apply (matches the JS `layers = 3`).
const LAYERS: u32 = 3;

/// glibc-style LCG the aniwatch `seedRand` uses: `(s * 1103515245 + 12345) & 0x7FFF_FFFF`.
///
/// The multiplier/increment pair is glibc's `rand()`; the 31-bit mask is the
/// site's, applied on top of the toolkit's `lcg_step`. That is exact, not an
/// approximation: `lcg_step` wraps modulo 2^32 and 2^31 divides 2^32, so the low
/// 31 bits of its result equal the 31-bit state for every seed.
const GLIBC_LCG_MULT: u32 = 1_103_515_245;
const GLIBC_LCG_INC: u32 = 12345;
const GLIBC_LCG_MASK: u32 = 0x7FFF_FFFF;

/// One draw in `0..modulus`, advancing `seed`. The `u64` state and the
/// `% modulus` draw are this cipher's wiring around the toolkit step.
fn glibc_lcg_step(seed: &mut u64, modulus: u64) -> u64 {
    // `as i32` truncates to the low 32 bits, and only the low 31 survive the
    // mask below, so a seed above u32 (never produced here) would still step correctly.
    let next = lcg_step(*seed as i32, GLIBC_LCG_MULT, GLIBC_LCG_INC);
    // `as u32` reinterprets the two's-complement bits so the mask sees them unsigned.
    *seed = u64::from(next as u32 & GLIBC_LCG_MASK);
    *seed % modulus
}

/// The printable ASCII character set used by the cipher (chars 32..=126).
fn char_array() -> Vec<char> {
    (32u8..=126).map(|b| b as char).collect()
}

/// Decrypt an encrypted source string using the custom 3-layer cipher.
///
/// # Arguments
/// * `src` — base64-encoded encrypted source string
/// * `client_key` — client key extracted from the embed page
/// * `megacloud_key` — megacloud key fetched from the keys repository
///
/// # Returns
/// The decrypted JSON string containing video source URLs.
///
/// # Errors
///
/// Returns `Err` when:
/// - Base64 decoding fails (`RdlpError::Extraction`)
/// - The decrypted data length prefix is invalid or missing (`RdlpError::Extraction`)
/// - The decrypted data is too short to contain the claimed length (`RdlpError::Extraction`)
pub fn decrypt_src(src: &str, client_key: &str, megacloud_key: &str) -> Result<String> {
    decrypt_src_impl(src, client_key, megacloud_key).map_err(|e| RdlpError::Extraction {
        message: format!("{e:#}"),
        url: None,
    })
}

fn decrypt_src_impl(src: &str, client_key: &str, megacloud_key: &str) -> anyhow::Result<String> {
    use base64::Engine;
    let gen_key = keygen(megacloud_key, client_key);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(src)
        .context("failed to base64-decode megacloud encrypted sources")?;
    let mut dec_src: Vec<char> = decoded.iter().map(|&b| b as char).collect();
    let chars = char_array();

    // Apply layers in reverse order (3, 2, 1)
    for iteration in (1..=LAYERS).rev() {
        let layer_key = format!("{gen_key}{iteration}");
        reverse_layer(&mut dec_src, &layer_key, &chars);
    }

    let result: String = dec_src.into_iter().collect();

    // First 4 characters encode the data length
    let data_len: usize = result
        .get(..4)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            anyhow::anyhow!("invalid data length prefix in decrypted megacloud source")
        })?;

    result
        .get(4..4 + data_len)
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "decrypted megacloud source too short: expected {data_len} chars after prefix"
            )
        })
}

/// Reverse one layer of the cipher.
fn reverse_layer(dec_src: &mut Vec<char>, layer_key: &str, chars: &[char]) {
    // Step 1: Reverse substitution mapping
    let sub_values = {
        let mut seed = u64::from(java_string_hash32(layer_key));
        seeded_shuffle(chars, |bound| glibc_lcg_step(&mut seed, bound))
    };
    let char_map: std::collections::HashMap<char, char> = sub_values
        .iter()
        .zip(chars.iter())
        .map(|(&shuffled, &original)| (shuffled, original))
        .collect();
    for ch in dec_src.iter_mut() {
        if let Some(&mapped) = char_map.get(ch) {
            *ch = mapped;
        }
    }

    // Step 2: Reverse columnar transposition
    *dec_src = columnar_untranspose(dec_src, layer_key);

    // Step 3: Reverse seed-based character shift
    let alphabet = chars.len() as u64;
    let mut seed = u64::from(java_string_hash32(layer_key));
    for ch in dec_src.iter_mut() {
        if let Some(idx) = chars.iter().position(|&c| c == *ch) {
            let rand_num = glibc_lcg_step(&mut seed, alphabet);
            let new_idx = (idx as i64 - rand_num as i64 + alphabet as i64) % alphabet as i64;
            *ch = chars[new_idx as usize];
        }
    }
}

/// Generate a derived key from the megacloud key and client key.
///
/// Combines both keys, applies a numeric hash, XOR transformation,
/// circular shift, and interleaving to produce the final key.
///
/// Internally works with `Vec<u32>` (JS char codes) because XOR
/// with 247 produces non-ASCII values that would break UTF-8 slicing.
fn keygen(megacloud_key: &str, client_key: &str) -> String {
    const KEYGEN_XOR_VAL: u32 = 247;
    const KEYGEN_SHIFT_VAL: usize = 5;

    let temp_key_str = format!("{megacloud_key}{client_key}");

    // Numeric hash using wrapping BigInt-like arithmetic (matches JS BigInt)
    let mut hash_val: i128 = 0;
    for b in temp_key_str.bytes() {
        hash_val = (i128::from(b))
            .wrapping_add(hash_val.wrapping_mul(31))
            .wrapping_add(hash_val << 7)
            .wrapping_sub(hash_val);
    }
    hash_val = hash_val.abs();
    let l_hash = (hash_val % 0x7FFF_FFFF_FFFF_FFFF) as usize;

    // Apply XOR — work with u32 char codes to avoid UTF-8 issues
    let temp_codes: Vec<u32> = temp_key_str
        .chars()
        .map(|c| (c as u32) ^ KEYGEN_XOR_VAL)
        .collect();

    // Circular shift (operates on char-code indices, not byte indices). An
    // empty `temp_codes` would divide-by-zero on the modulo below — that
    // shouldn't happen with a well-formed megacloud payload, but external
    // input is never trusted.
    let len = temp_codes.len();
    if len == 0 {
        return String::new();
    }
    let pivot = ((l_hash % len) + KEYGEN_SHIFT_VAL) % len;
    let shifted: Vec<u32> = temp_codes[pivot..]
        .iter()
        .chain(temp_codes[..pivot].iter())
        .copied()
        .collect();

    // Interleave with reversed client key char codes
    let leaf: Vec<u32> = client_key.chars().rev().map(|c| c as u32).collect();
    let max_len = shifted.len().max(leaf.len());
    let mut return_codes = Vec::with_capacity(max_len * 2);
    for i in 0..max_len {
        if let Some(&code) = shifted.get(i) {
            return_codes.push(code);
        }
        if let Some(&code) = leaf.get(i) {
            return_codes.push(code);
        }
    }

    // Limit length based on hash (char count, not byte count)
    let limit = 96 + (l_hash % 33);
    return_codes.truncate(limit);

    // Normalize to printable ASCII (32..=126)
    return_codes
        .iter()
        .map(|&code| char::from_u32((code % 95) + 32).unwrap_or(' '))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_array() {
        let chars = char_array();
        assert_eq!(chars.len(), 95);
        assert_eq!(chars[0], ' ');
        assert_eq!(chars[94], '~');
    }

    #[test]
    fn glibc_lcg_step_matches_reference() {
        // Reference values computed independently: seed=(42*1103515245+12345)&0x7FFF_FFFF.
        // `r1 == r2` alone is a tautology (true for any multiplier/increment); pin the
        // literal outputs so a wrong constant (e.g. INC=12346 -> 28 / 1_250_496_028) is
        // caught rather than silently accepted.
        let mut seed = 42;
        let draw = glibc_lcg_step(&mut seed, 100);
        assert_eq!(draw, 27);
        assert_eq!(seed, 1_250_496_027);
    }

    #[test]
    fn decrypt_src_recovers_pinned_fixture() {
        // Characterization pin generated from commit 899369bf's implementation
        // (pre-rewrite `hash_key`/`seed_rand`/`seed_shuffle`/`columnar_cipher`,
        // via a throwaway forward encoder run in a scratch worktree — see the
        // C0-b Task 2 fix-round-1 report). Certifies this rewrite decrypts what
        // the pre-rewrite code encrypted; a captured site ciphertext would
        // upgrade it to a correctness oracle.
        const CLIENT_KEY: &str = "c0b-client-key";
        const MEGACLOUD_KEY: &str = "c0b-megacloud-key";
        const FIXTURE_B64: &str = "LC95eks9JGZjJFg+RSNcWGQ3VTtMXVZAJlhXdWleWCJcZ0lNZ3BAQSU6XjBQQTkiWCshJzkmQHJfT3xsaXxLTCJgfGFCNmtVTVtKej4oL1p1dTk6RlFYSFQ5Nnw=";
        const EXPECTED_PAYLOAD: &str = " !~}Hello, world! This pins C0-b Task 2.~} ";

        let decrypted = decrypt_src(FIXTURE_B64, CLIENT_KEY, MEGACLOUD_KEY).unwrap();
        assert_eq!(decrypted, EXPECTED_PAYLOAD);
    }

    #[test]
    fn keygen_output_is_printable_ascii() {
        // The final `% 95 + 32` normalisation is what keeps the derived key
        // inside the cipher's alphabet; an off-by-one there leaks control
        // characters or DEL into every layer key.
        let key = keygen("megakey", "clientkey");
        assert!(!key.is_empty());
        assert!(key.chars().all(|c| (32..=126).contains(&(c as u32))));
    }

    #[test]
    fn test_keygen_different_inputs() {
        let k1 = keygen("key_a", "client_a");
        let k2 = keygen("key_b", "client_b");
        assert_ne!(k1, k2);
    }
}
