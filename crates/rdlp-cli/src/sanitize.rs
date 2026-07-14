//! Neutralize hostile control/format characters in extractor-sourced text.
//!
//! Titles, uploader names, and other strings sourced from a remote site are
//! attacker-controlled. Written **raw** to a TTY or a log they can abuse two
//! distinct Unicode-based vectors:
//!
//! 1. **Terminal escape injection** (CWE-150, category `Cc`) — a title
//!    containing `ESC [ 2 J` clears the user's screen, `ESC ]` (OSC) retitles
//!    the window, and a bare `CR` overwrites the current line to spoof output.
//!    Since #481 the full WHATWG entity set is decodable (`&#27;` → `ESC`) at
//!    more extractor call sites, so this boundary must be guarded.
//! 2. **Bidirectional-override spoofing** ("Trojan Source", CVE-2021-42574) —
//!    bidi formatting controls such as `U+202E` RIGHT-TO-LEFT OVERRIDE reorder
//!    how text renders, so `"invoice\u{202e}gpj.exe"` displays as
//!    `"invoiceexe.jpg"`. These are category `Cf`, not `Cc`, so the
//!    escape-injection filter does not catch them (#485).
//!
//! This mirrors `rdlp_api`'s `Orchestrator::sanitize_filename`, which already
//! strips control characters at the filesystem boundary — here we guard the
//! terminal/log boundary the same way.
//!
//! ## Why the stdlib `char::is_control()` filter and not an ANSI-strip crate
//!
//! No Rust crate is designed as a *security* sanitizer for this — `console`
//! (`strip_ansi_codes`), `strip-ansi-escapes`, `anstream`, and `vte` are all
//! *rendering* helpers (strip color codes for width calculation / dumb-terminal
//! fallback), by their own documentation. Each leaves gaps against this threat
//! model: `console` doesn't touch OSC, bare `CR`, `BEL`, `BS`, or `DEL`;
//! `anstream` keeps `CR` (its "whitespace" exemption) — the line-overwrite
//! spoofing vector. More fundamentally, every one is byte/regex-based and keys
//! C1 detection off the raw byte `0x9B`, which **cannot occur in valid UTF-8**;
//! a C1 introducer arriving as the encoded scalar `U+009B` (`0xC2 0x9B`) sails
//! straight through them. Filtering the Unicode `Cc` category operates on the
//! *decoded scalar value*, so it neutralizes that vector where the crates
//! cannot. CWE-150 endorses this "restrict to printable" approach over matching
//! known-bad sequences. (Verdict from a cited multi-source survey, 2026-07-14.)

/// Return a copy of `s` with hostile control and bidi-formatting characters
/// removed, rendering any embedded terminal escape sequence or visual-reorder
/// spoof inert before the text is written to a TTY or log.
///
/// Two independent filters run in one pass:
///
/// - **[`char::is_control`]** — the Unicode general-category `Cc` set: the C0
///   range `U+0000..=U+001F` (including `ESC` `U+001B`, `CR`, `BEL`, `BS`),
///   `DEL` `U+007F`, and the C1 range `U+0080..=U+009F` (which includes the
///   single-byte `CSI` `U+009B` / `OSC` `U+009D` introducers). Stripping the
///   introducer leaves the remaining bytes as harmless literal text
///   (`"\x1b[31mX"` → `"[31mX"`).
/// - [`rdlp_security::text::is_bidi_control`] — the bidi embedding/override/
///   isolate controls (`U+202A..=U+202E`, `U+2066..=U+2069`) that drive the
///   Trojan-Source visual-reordering attack. This predicate is shared with the
///   filesystem boundary (`rdlp-api`'s `sanitize_filename`) so both sites
///   classify the same 9 code points (#487).
///
/// Ordinary printable text — including non-ASCII letters, spaces, and the
/// zero-width joiner `U+200D` that legitimate emoji sequences depend on —
/// passes through unchanged.
#[must_use]
pub fn sanitize_for_terminal(s: &str) -> String {
    s.chars()
        .filter(|&c| !c.is_control() && !rdlp_security::text::is_bidi_control(c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_terminal;

    #[test]
    fn ordinary_title_passes_through_unchanged() {
        let title = "Café — 日本語 Video (2024) [HD]";
        assert_eq!(sanitize_for_terminal(title), title);
    }

    #[test]
    fn ascii_space_and_punctuation_preserved() {
        let s = "A B\tC"; // tab is a control char and is stripped
        assert_eq!(sanitize_for_terminal(s), "A BC");
    }

    #[test]
    fn esc_csi_sequence_is_rendered_inert() {
        let malicious = "\u{1b}[31mHACKED\u{1b}[0m";
        let out = sanitize_for_terminal(malicious);
        assert!(!out.contains('\u{1b}'), "ESC must be stripped: {out:?}");
        assert_eq!(out, "[31mHACKED[0m");
    }

    #[test]
    fn decoded_numeric_entity_esc_is_neutralized() {
        // Post-#481 an extractor can decode `&#27;` into a raw ESC; the decoded
        // title reaching the terminal must be inert.
        let decoded_title = "Watch \u{1b}]0;pwned\u{7}now";
        let out = sanitize_for_terminal(decoded_title);
        assert!(!out.chars().any(char::is_control), "no controls: {out:?}");
        assert_eq!(out, "Watch ]0;pwnednow");
    }

    #[test]
    fn carriage_return_is_stripped() {
        let out = sanitize_for_terminal("real title\rFAKE STATUS");
        assert!(!out.contains('\r'));
        assert_eq!(out, "real titleFAKE STATUS");
    }

    #[test]
    fn bel_backspace_del_and_nul_are_stripped() {
        let out = sanitize_for_terminal("a\u{7}b\u{8}c\u{7f}d\0e");
        assert_eq!(out, "abcde");
    }

    #[test]
    fn c1_single_byte_csi_is_stripped() {
        // U+009B is the single-byte CSI introducer; some terminals honor it.
        let out = sanitize_for_terminal("x\u{9b}31mY");
        assert!(!out.chars().any(char::is_control), "no controls: {out:?}");
        assert_eq!(out, "x31mY");
    }

    #[test]
    fn c1_upper_boundary_stops_at_nbsp() {
        // Boundary: U+009F is the last C1 control (stripped); U+00A0 NO-BREAK
        // SPACE is the first non-control after it (preserved). Guards against a
        // future range refactor sliding the ceiling from 0x9F to 0xA0.
        let out = sanitize_for_terminal("a\u{9f}\u{a0}b");
        assert_eq!(out, "a\u{a0}b");
    }

    #[test]
    fn bidi_rlo_override_is_stripped() {
        // The canonical "Trojan Source" vector: RIGHT-TO-LEFT OVERRIDE (U+202E)
        // reorders the visual rendering of the following text. It must not
        // survive to the terminal.
        let out = sanitize_for_terminal("invoice\u{202e}gpj.exe");
        assert!(!out.contains('\u{202e}'), "RLO must be stripped: {out:?}");
        assert_eq!(out, "invoicegpj.exe");
    }

    #[test]
    fn zwj_emoji_sequence_is_preserved() {
        // U+200D ZERO WIDTH JOINER is category `Cf` like the bidi controls, but
        // it is load-bearing for legitimate emoji (a family emoji is
        // person-ZWJ-person-ZWJ-child). Stripping the whole `Cf` category would
        // corrupt real titles; only the bidi-control block may be removed.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(sanitize_for_terminal(family), family);
    }

    #[test]
    fn bidi_embedding_isolate_block_stripped_boundaries_preserved() {
        // First range U+202A..=U+202E (LRE,RLE,PDF,LRO,RLO): pin both edges —
        // U+2029 (below) and U+202F NARROW NO-BREAK SPACE (above) are kept.
        let out = sanitize_for_terminal("\u{2029}\u{202a}\u{202e}\u{202f}");
        assert_eq!(out, "\u{2029}\u{202f}");

        // Second range U+2066..=U+2069 (LRI,RLI,FSI,PDI): U+2065 (below) and
        // U+206A (above, a distinct `Cf` char we intentionally keep) preserved.
        let out = sanitize_for_terminal("\u{2065}\u{2066}\u{2069}\u{206a}");
        assert_eq!(out, "\u{2065}\u{206a}");
    }
}
