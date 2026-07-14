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
/// use rdlp_security::text::is_bidi_control;
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
