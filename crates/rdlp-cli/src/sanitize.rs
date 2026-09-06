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

// The implementation moved to `rdlp-security` when `rdlp-types`'s boundary
// record needed the same filter: rdlp-types cannot depend on this binary
// crate, and the character classes it composes (`char::is_control` plus
// `is_bidi_control`) already lived there. Re-exported rather than relocated
// at every call site so `rdlp_cli::sanitize::sanitize_for_terminal` keeps
// naming the CLI's terminal boundary.
pub use rdlp_security::text::sanitize_for_terminal;
