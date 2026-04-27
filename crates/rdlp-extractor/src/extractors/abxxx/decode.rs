//! Decoder for ABXXX's obfuscated `video_url` field.
//!
//! ABXXX (and several other KVS deployments) returns the playable URL from
//! `/api/videofile.php` in a lightly obfuscated form:
//!
//! 1. **Cyrillic homoglyph substitution.** Some Latin characters (`E`, `M`, `C`,
//!    `A`, `P`, `B`, `X`, `K`, `H`, `T`, `O`) are replaced by visually identical
//!    Cyrillic codepoints to defeat naïve scrapers that grep for base64 alphabet.
//!    The substitutions are reversible.
//! 2. **Comma-separated halves.** The string contains a single `,` which
//!    separates the base64-encoded path from the base64-encoded query string.
//!    After decoding both, the canonical URL is `<host><path>?<query>`.
//!
//! This module owns the decode and is intentionally narrow — no HTTP, no I/O.

use base64::{Engine, engine::general_purpose::STANDARD};

/// Reverse Cyrillic homoglyph substitution used in ABXXX's `video_url`.
fn deobfuscate(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            // Cyrillic uppercase that visually matches Latin uppercase
            '\u{0410}' => 'A', // А
            '\u{0412}' => 'B', // В
            '\u{0421}' => 'C', // С
            '\u{0415}' => 'E', // Е
            '\u{041D}' => 'H', // Н
            '\u{041A}' => 'K', // К
            '\u{041C}' => 'M', // М
            '\u{041E}' => 'O', // О
            '\u{0420}' => 'P', // Р
            '\u{0422}' => 'T', // Т
            '\u{0425}' => 'X', // Х
            other => other,
        })
        .collect()
}

/// Decode an obfuscated `video_url` into `(path, query)`.
///
/// Returns `None` if either half is not valid base64 or not valid UTF-8.
/// `query` is `None` when the input contains no `,` separator (older
/// single-half encodings).
#[must_use]
pub fn decode_video_url(encoded: &str) -> Option<(String, Option<String>)> {
    let cleaned = deobfuscate(encoded);
    let (path_b64, query_b64) = match cleaned.split_once(',') {
        Some((p, q)) => (p, Some(q)),
        None => (cleaned.as_str(), None),
    };

    let path = decode_b64(path_b64)?;
    let query = match query_b64 {
        Some(q) => Some(decode_b64(q)?),
        None => None,
    };
    Some((path, query))
}

fn decode_b64(s: &str) -> Option<String> {
    let trimmed = s.trim();
    let mut padded = trimmed.to_string();
    let pad = (4 - padded.len() % 4) % 4;
    for _ in 0..pad {
        padded.push('=');
    }
    let bytes = STANDARD.decode(padded.as_bytes()).ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real fixture captured from `/api/videofile.php?video_id=129452`
    /// on 2026-04-26. The signed query (`ti=…`) rotates, but the path is stable.
    const FIXTURE_ENCODED: &str = "L2dldF9maWxlLz\u{0415}vNmViZjFm\u{041C}mY3ODU1Yj\u{041C}yODhlNGYxN2ViYTU1NWY3ZTNlYzFjNDZj\u{041C}GRmLz\u{0415}yOT\u{0410}w\u{041C}\u{0421}8x\u{041C}jk0NTIv\u{041C}TI5NDUyLm1wN\u{0421}8,ZD01NDQzJmJyPTIzNQ==";

    #[test]
    fn decodes_canonical_fixture() {
        let (path, query) = decode_video_url(FIXTURE_ENCODED).expect("decode succeeds");
        assert_eq!(
            path,
            "/get_file/1/6ebf1f2f7855b3288e4f17eba555f7e3ec1c46c0df/129000/129452/129452.mp4/"
        );
        assert_eq!(query.as_deref(), Some("d=5443&br=235"));
    }

    #[test]
    fn handles_input_without_query_half() {
        // Plain base64 of "/foo/bar" with no comma separator
        let enc = STANDARD.encode("/foo/bar");
        let (path, query) = decode_video_url(&enc).expect("decode succeeds");
        assert_eq!(path, "/foo/bar");
        assert!(query.is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_video_url("not~valid~base64!@#").is_none());
    }

    #[test]
    fn deobfuscate_replaces_all_known_homoglyphs() {
        let input = "\u{0410}\u{0412}\u{0421}\u{0415}\u{041D}\u{041A}\u{041C}\u{041E}\u{0420}\u{0422}\u{0425}";
        assert_eq!(deobfuscate(input), "ABCEHKMOPTX");
    }
}
