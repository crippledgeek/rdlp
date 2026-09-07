//! # rdlp-types
//!
//! Pure domain types for rdlp (Rust Download Program).
//!
//! This crate contains the core data structures with zero I/O dependencies,
//! making it ideal for compile-time optimization and clear separation of concerns.
//!
//! ## Types
//!
//! - [`InfoDict`] - Central metadata structure for videos
//! - [`Format`] - Video/audio format information
//! - [`Config`] - Application configuration
//! - [`FormatSelector`] - Format selection DSL
//!
//! ## Design Philosophy
//!
//! This crate intentionally has minimal dependencies:
//! - `serde` for serialization
//! - `url` for URL parsing
//! - `regex` for pattern matching
//!
//! No async runtime, HTTP client, or I/O operations are included.

#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

pub mod audio_format;
pub mod boundary;
pub mod browser_emulation;
pub mod browser_type;
pub mod config;
pub mod container;
#[cfg(test)]
mod enum_test_support;
pub mod fixup_policy;
pub mod format;
pub mod info_dict;
pub mod log_targets;
pub mod match_filter;
pub mod media_name;
pub mod parse_error;
pub mod postprocess;
pub mod progress;
pub mod protocol;
pub mod recode_audio_mode;
pub mod rule;
pub mod search;
pub mod subtitle_format;
pub mod subtitle_kind;
pub mod subtitle_selection;
pub mod subtitle_track;
pub mod thumbnail;
pub mod vpx_deadline;

// Re-export main types
pub use audio_format::AudioFormat;
pub use boundary::{Action, Subject};
pub use browser_emulation::BrowserEmulation;
pub use browser_type::BrowserType;
pub use config::{Config, ConfigValidationError};
pub use container::ContainerFormat;
pub use fixup_policy::FixupPolicy;
pub use format::{
    Codec, Format, FormatSelectError, FormatSelector, FormatSorter, Fragment, format_select,
};
pub use info_dict::{Chapter, InfoDict, Subtitle, Thumbnail};
pub use media_name::{
    AudioEncoderName, CodecName, InvalidMediaName, Rfc6381Codec, VideoEncoderName,
};
pub use parse_error::ParseEnumError;
pub use postprocess::{ContainerRequest, ContainerSource, ExplicitContainer, PostProcess};
pub use progress::Progress;
pub use protocol::DownloadProtocol;
pub use recode_audio_mode::RecodeAudioMode;
pub use search::{
    SearchFilter, SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview, SearchSiteInfo,
};
pub use subtitle_format::SubtitleFormat;
pub use subtitle_kind::SubtitleKind;
pub use subtitle_selection::{select_subtitles, subtitle_filename};
pub use subtitle_track::{
    SubtitleDiagnostic, SubtitleReason, SubtitleResult, SubtitleStatus, SubtitleTrack,
    normalize_from_info_dict,
};
pub use thumbnail::{THUMBNAIL_EXTENSIONS, ThumbnailFormat, sniff_thumbnail_format};
pub use vpx_deadline::VpxDeadline;

/// Decode HTML entities in display text. The workspace's single implementation.
///
/// Sites serve human-readable text entity-encoded — in markup, in element
/// attributes, inside JSON-LD and `window.*` script blobs, and (for some
/// APIs) in JSON responses. Left encoded, `&quot;` and `&#039;` reach the
/// output template and land in real filenames (#698).
///
/// Applied at ONE boundary — [`InfoDict::decode_text_fields`] and
/// [`SearchResultPreview::decode_text_fields`], invoked by the orchestrator —
/// rather than at each extraction site, because remembering it per site is
/// precisely what failed.
///
/// Decoding text that carries no entities returns it unchanged, so the
/// boundary is a no-op for the paths where the HTML parser already decoded.
/// Measured across the recorded live pages in this repo — every `title="..."`
/// attribute in `extractors/*/tests/*.html`: 2668 of them, 106 entity-encoded
/// at source, and NONE still entity-like after one decode. So a second pass
/// changes nothing in practice, which is what makes an unconditional boundary
/// safe. (Parsing the same fixtures with a real HTML parser yields 0 encoded
/// title attributes, independently confirming that the parser path is already
/// decoded and the boundary is a no-op there.)
///
/// Delegates to `html_escape` — the WHATWG named set plus decimal and hex
/// numeric references, with one stable behaviour and two that moved between
/// versions. Measured against each exact version, not a caret resolution:
///
/// - STABLE, and load-bearing: it requires the terminating semicolon, so
///   WHATWG's legacy semicolon-less forms are not decoded. That is desirable
///   — it is why a URL's `&copy=` query key survives the tnaflix XML decode.
/// - Fixed in 0.2.14: a bare `&` no longer swallows the entity after it. On
///   0.2.13 `"Wife & Friend&#039;s"` came back unchanged, so the #698 defect
///   survived the boundary for that shape.
/// - Added in 0.2.15: numeric references to C0 controls are rejected, so
///   `&#27;` no longer decodes to ESC. `&#10;` (LF) and the C1 `&#155;`
///   (CSI) still do, which is why display text is sanitized at log sinks
///   rather than trusted because of this.
///
/// Not hand-rolled: a `.replace()` chain was tried in this workspace before
/// and removed for double-decoding.
#[must_use]
pub fn decode_html_entities(text: &str) -> String {
    html_escape::decode_html_entities(text).into_owned()
}

/// Decode entities across a list of display strings.
fn decode_each(values: &mut [String]) {
    for v in values {
        *v = decode_html_entities(v);
    }
}

#[cfg(test)]
mod decode_html_entities_tests {
    use super::decode_html_entities;

    /// WP-REST punctuation entities, which is where this decoder's behaviour
    /// was first pinned (these assertions moved here from koreanpornmovie's
    /// local copy when that copy was deleted).
    #[test]
    fn decodes_wp_rest_punctuation_entities() {
        assert_eq!(decode_html_entities("A &#8211; B"), "A \u{2013} B");
        assert_eq!(decode_html_entities("A &#8212; B"), "A \u{2014} B");
        assert_eq!(decode_html_entities("plain title"), "plain title");
    }

    /// Curly quotes decode to their true WHATWG code points (U+2019), not an
    /// ASCII-apostrophe approximation.
    #[test]
    fn decodes_curly_quotes_to_true_code_points() {
        assert_eq!(decode_html_entities("it&#8217;s"), "it\u{2019}s");
        assert_eq!(decode_html_entities("&#8216;q&#8217;"), "\u{2018}q\u{2019}");
    }

    #[test]
    fn test_decode_html_entities() {
        assert_eq!(decode_html_entities("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(decode_html_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_html_entities("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(decode_html_entities("it&#39;s"), "it's");
        assert_eq!(decode_html_entities("&#60;"), "<");
        assert_eq!(decode_html_entities("&#x3C;"), "<");
    }

    /// Regression: entities must be decoded in a single pass. An already-escaped
    /// input like `&amp;lt;` (literal text `&lt;`) must NOT be recursively
    /// re-decoded into `<` — that is the OWASP double-encoding class and diverges
    /// from `yt-dlp` / `CPython` `html.unescape`. Fails against the old sequential
    /// `.replace()` decoder, which yielded `<`.
    #[test]
    fn test_decode_html_entities_no_double_decode() {
        assert_eq!(decode_html_entities("&amp;lt;"), "&lt;");
        assert_eq!(decode_html_entities("&amp;amp;"), "&amp;");
        assert_eq!(decode_html_entities("&amp;#39;"), "&#39;");
        assert_eq!(decode_html_entities("&amp;#8211;"), "&#8211;");
    }

    /// `&nbsp;` decodes to U+00A0 NO-BREAK SPACE (WHATWG / yt-dlp parity), not an
    /// ASCII space. Fails against the old decoder, which emitted `0x20`.
    #[test]
    fn test_decode_html_entities_nbsp_is_no_break_space() {
        assert_eq!(decode_html_entities("a&nbsp;b"), "a\u{00A0}b");
    }

    /// The full WHATWG named-entity set is decoded, not just the old 8-entity
    /// subset. Fails against the old decoder, which left `&mdash;`/`&hellip;`
    /// untouched.
    #[test]
    fn test_decode_html_entities_full_named_set() {
        assert_eq!(decode_html_entities("a &mdash; b"), "a \u{2014} b");
        assert_eq!(decode_html_entities("wait&hellip;"), "wait\u{2026}");
    }

    /// Unknown/malformed entities are left verbatim (no decode, no panic) — pins
    /// passthrough behavior against a future decoder swap.
    #[test]
    fn test_decode_html_entities_unknown_left_verbatim() {
        assert_eq!(decode_html_entities("a &bogus; b"), "a &bogus; b");
        assert_eq!(decode_html_entities("bare & amp"), "bare & amp");
    }

    /// A bare `&` must not swallow the entity that follows it.
    ///
    /// html-escape 0.2.13 — which this workspace pinned until this change —
    /// accumulated a named entity from the first `&` and scanned forward to
    /// the next `;`, consuming the following reference's terminator. All three
    /// inputs below came back UNCHANGED on 0.2.13 and decode correctly on
    /// 0.2.15, so this is a RED/GREEN pair against the version, not a
    /// restatement of behaviour.
    ///
    /// It matters because a bare `&` in a title is ordinary, and the first
    /// case is the exact shape of a real abxxx search result: the #698 defect
    /// survived the boundary for it.
    #[test]
    fn a_bare_ampersand_does_not_swallow_the_next_entity() {
        assert_eq!(
            decode_html_entities("Wife & Friend&#039;s Pov Handjob"),
            "Wife & Friend's Pov Handjob"
        );
        assert_eq!(decode_html_entities("AT&T &amp; Co"), "AT&T & Co");
        assert_eq!(
            decode_html_entities("R&D &mdash; notes"),
            "R&D \u{2014} notes"
        );
    }

    /// Text with nothing to decode comes back unchanged — the property that
    /// lets the boundary run unconditionally over parser-decoded titles.
    #[test]
    fn leaves_plain_text_unchanged() {
        assert_eq!(
            decode_html_entities("Tom & Jerry: 100% <fun>"),
            "Tom & Jerry: 100% <fun>"
        );
    }
}
