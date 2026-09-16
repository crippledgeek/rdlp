//! Character-class predicates for neutralizing hostile Unicode in
//! attacker-controlled text (extractor-sourced titles, uploader names, …).
//!
//! These are the shared *building blocks* — not a one-size sanitizer. Each
//! boundary composes them into its own policy: the terminal/log boundary
//! (`rdlp-cli`'s `sanitize_for_terminal`) strips both control and bidi-control
//! characters; the filesystem boundary (`rdlp-api`'s `Orchestrator::sanitize_filename`)
//! additionally replaces filesystem-reserved characters and normalizes
//! whitespace. Forcing a single `sanitize()` would hide those legitimate
//! differences behind a policy object; sharing the *character-class test*
//! instead keeps each site's policy explicit while the load-bearing
//! classification lives in one audited place.
//!
//! This mirrors how `rustc`'s own Trojan-Source defense
//! (`rustc_lint::hidden_unicode_codepoints`) is structured: a bare list of the
//! same code points plus a free predicate, with no `unicode-bidi` /
//! `unicode-security` dependency (those solve heavier adjacent problems — full
//! `BidiClass` resolution and identifier-spoofing detection — that this
//! byte-inert-ing task does not need).
//!
//! ## Why this lives in `rdlp-redact`, not `rdlp-security`
//!
//! This module moved from `rdlp-security` (#695): `rdlp-types` needed
//! `sanitize_for_terminal` for its boundary-record `Display` impl, and pulling
//! in `rdlp-security` for one string filter dragged `ipnet`, `url`,
//! `thiserror`, and `log` into a crate documented as "pure data types … no
//! I/O" for a function that does no networking and validates no URL.
//!
//! `rdlp-redact` was already `rdlp-types`'s dependency, and is the closer
//! fit: both this module and [`redact_str`](crate::redact_str) exist to make
//! attacker- or operator-influenced text **safe to write to a boundary
//! sink** — a log line or a terminal — before it gets there. `redact_str`
//! strips *credentials*; this module strips *control and bidi-formatting
//! characters* (CWE-117 log injection, CWE-150 terminal escape injection,
//! CVE-2021-42574 Trojan Source). Different threat, same job: the last thing
//! standing between attacker-controlled text and the sink. `rdlp-security`
//! keeps the concern that actually needs its dependencies — SSRF/URL
//! validation — and stays free of this one.

/// Return `true` for the Unicode bidi controls used in "Trojan Source" spoofing.
///
/// The hostile set (CVE-2021-42574) is the embeddings/overrides
/// `U+202A..=U+202E` (LRE, RLE, PDF, LRO, RLO) and the isolates
/// `U+2066..=U+2069` (LRI, RLI, FSI, PDI).
///
/// This is exactly the set `rustc`'s deny-by-default
/// `text_direction_codepoint_in_literal` lint treats as hostile
/// (`TEXT_FLOW_CONTROL_CHARS`).
///
/// It is deliberately **not** the whole `Cf` (format) general category. `Cf`
/// also contains the zero-width joiner `U+200D`, which is load-bearing for
/// legitimate emoji sequences (a family emoji is person-ZWJ-person-ZWJ-child);
/// stripping all of `Cf` would corrupt real titles. Only this bidi-control
/// block is classified as hostile here.
///
/// # Examples
///
/// ```
/// use rdlp_redact::text::is_bidi_control;
///
/// assert!(is_bidi_control('\u{202e}')); // RIGHT-TO-LEFT OVERRIDE
/// assert!(is_bidi_control('\u{2066}')); // LEFT-TO-RIGHT ISOLATE
/// assert!(!is_bidi_control('\u{200d}')); // ZERO WIDTH JOINER — kept
/// assert!(!is_bidi_control('a'));
/// ```
#[must_use]
pub const fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::is_bidi_control;

    #[test]
    fn override_block_u202a_to_u202e_is_bidi_control() {
        for c in '\u{202A}'..='\u{202E}' {
            assert!(
                is_bidi_control(c),
                "{c:?} (U+{:04X}) should match",
                c as u32
            );
        }
    }

    #[test]
    fn isolate_block_u2066_to_u2069_is_bidi_control() {
        for c in '\u{2066}'..='\u{2069}' {
            assert!(
                is_bidi_control(c),
                "{c:?} (U+{:04X}) should match",
                c as u32
            );
        }
    }

    #[test]
    fn range_boundaries_are_excluded() {
        // Just outside each range must NOT match, guarding against a range
        // refactor sliding an edge.
        assert!(!is_bidi_control('\u{2029}')); // below the override block
        assert!(!is_bidi_control('\u{202F}')); // above the override block (NNBSP)
        assert!(!is_bidi_control('\u{2065}')); // below the isolate block
        assert!(!is_bidi_control('\u{206A}')); // above the isolate block
    }

    #[test]
    fn zero_width_joiner_is_not_bidi_control() {
        // U+200D is `Cf` like the bidi controls but must survive — legitimate
        // emoji depend on it.
        assert!(!is_bidi_control('\u{200D}'));
    }

    #[test]
    fn ordinary_characters_are_not_bidi_control() {
        for c in ['a', 'Z', '9', ' ', '日', 'é', '\u{1F600}'] {
            assert!(!is_bidi_control(c), "{c:?} should not match");
        }
    }
}

// ## Why the stdlib `char::is_control()` filter and not an ANSI-strip crate
//
// No Rust crate is designed as a *security* sanitizer for this — `console`
// (`strip_ansi_codes`), `strip-ansi-escapes`, `anstream`, and `vte` are all
// *rendering* helpers (strip color codes for width calculation / dumb-terminal
// fallback), by their own documentation. Each leaves gaps against this threat
// model: `console` doesn't touch OSC, bare `CR`, `BEL`, `BS`, or `DEL`;
// `anstream` keeps `CR` (its "whitespace" exemption) — the line-overwrite
// spoofing vector. More fundamentally, every one is byte/regex-based and keys
// C1 detection off the raw byte `0x9B`, which **cannot occur in valid UTF-8**;
// a C1 introducer arriving as the encoded scalar `U+009B` (`0xC2 0x9B`) sails
// straight through them. Filtering the Unicode `Cc` category operates on the
// *decoded scalar value*, so it neutralizes that vector where the crates
// cannot. CWE-150 endorses this "restrict to printable" approach over matching
// known-bad sequences. (Verdict from a cited multi-source survey, 2026-07-14.)

/// Return a copy of `s` with hostile control and bidi-formatting characters removed.
///
/// This renders any embedded terminal escape sequence or visual-reorder spoof
/// inert before the text is written to a TTY or log.
///
/// Two independent filters run in one pass:
///
/// - **[`char::is_control`]** — the Unicode general-category `Cc` set: the C0
///   range `U+0000..=U+001F` (including `ESC` `U+001B`, `CR`, `BEL`, `BS`),
///   `DEL` `U+007F`, and the C1 range `U+0080..=U+009F` (which includes the
///   single-byte `CSI` `U+009B` / `OSC` `U+009D` introducers). Stripping the
///   introducer leaves the remaining bytes as harmless literal text
///   (`"\x1b[31mX"` → `"[31mX"`).
/// - [`is_bidi_control`] — the bidi embedding/override/
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
        .filter(|&c| !c.is_control() && !is_bidi_control(c))
        .collect()
}

#[cfg(test)]
mod sanitize_for_terminal_tests {
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

/// What every `Cc` character in a line-record token becomes under
/// [`sanitize_for_line`]: a placeholder rather than nothing, so an id whose
/// only content was a control character does not collapse to the empty
/// string and collide with another entry's.
const LINE_CONTROL_PLACEHOLDER: char = '_';

/// Return a copy of `s` fit to be ONE token of a line-oriented text record,
/// with every control character (the Unicode general-category `Cc` set —
/// see [`sanitize_for_terminal`] for the ranges) replaced by
/// `LINE_CONTROL_PLACEHOLDER` (`_`).
///
/// The sink this exists for is the download archive: one `{extractor} {id}`
/// record per line, read back with `BufRead::lines` and split on the first
/// space. An `id` is plugin-supplied text, and a LF or CR inside it would
/// end the record early and start another — `"1\nxvideos 456"` writes
/// `xvideos 456` as a second record, so another extractor's video is
/// reported as already downloaded (CWE-93, record injection). TAB and the
/// rest of `Cc` cannot split a line but are replaced too, so the token stays
/// printable and a hand-edited archive shows exactly what was recorded.
///
/// Only `Cc` is touched. A space is kept — the archive's reader splits on
/// the FIRST space, so an id containing one round-trips intact today and
/// rewriting it would silently change every existing entry's key. Bidi
/// controls are kept too: they are a rendering threat, not a line-structure
/// one, and the record is never rendered.
///
/// # Examples
///
/// ```
/// use rdlp_redact::text::sanitize_for_line;
///
/// assert_eq!(sanitize_for_line("1\nxvideos 456"), "1_xvideos 456");
/// assert_eq!(sanitize_for_line("a b"), "a b");
/// ```
#[must_use]
pub fn sanitize_for_line(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                LINE_CONTROL_PLACEHOLDER
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod sanitize_for_line_tests {
    use super::sanitize_for_line;

    /// The record-injection shape: a LF inside an id would end the archive
    /// line early and begin another.
    #[test]
    fn line_feed_is_replaced() {
        assert_eq!(sanitize_for_line("1\nxvideos 456"), "1_xvideos 456");
    }

    #[test]
    fn carriage_return_and_tab_are_replaced() {
        assert_eq!(sanitize_for_line("1\rxvideos 456"), "1_xvideos 456");
        assert_eq!(sanitize_for_line("1\txvideos 456"), "1_xvideos 456");
        assert_eq!(sanitize_for_line("1\r\n2"), "1__2");
    }

    #[test]
    fn other_controls_are_replaced_not_dropped() {
        // ESC, NUL, DEL, and a C1 control each become one placeholder, so
        // an id made only of controls is not the empty string.
        assert_eq!(sanitize_for_line("\u{1b}[2J"), "_[2J");
        assert_eq!(sanitize_for_line("a\0b\u{7f}c\u{85}d"), "a_b_c_d");
        assert_eq!(sanitize_for_line("\n"), "_");
    }

    #[test]
    fn space_and_printable_non_ascii_are_kept() {
        assert_eq!(sanitize_for_line("a b"), "a b");
        assert_eq!(sanitize_for_line("Café 日本語"), "Café 日本語");
        assert_eq!(sanitize_for_line(""), "");
    }

    /// Line structure, not rendering, is the concern: bidi controls are
    /// `Cf`, not `Cc`, and pass through.
    #[test]
    fn bidi_controls_are_kept() {
        assert_eq!(sanitize_for_line("a\u{202e}b"), "a\u{202e}b");
    }
}
