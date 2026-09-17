//! Download archive for tracking completed downloads.
//!
//! Records completed downloads as `{extractor} {id}` lines in a text file.
//! On subsequent runs, already-present entries are skipped. Compatible with
//! yt-dlp's `--download-archive` format.
//!
//! # Case-insensitive extractor token
//!
//! The extractor token is written lowercased and matched case-insensitively,
//! mirroring yt-dlp's `make_archive_id` (`f'{ie_key.lower()} {video_id}'`).
//! The id half stays case-sensitive. [`archive_key`] is the single place the
//! line is formatted; [`load_archive`] normalises the extractor token of
//! every line it reads so a legacy cased line (written by an older rdlp
//! build) still matches. This also covers plugin extractors: `InfoDict`'s
//! extractor field may carry a manifest `display_name` (e.g. `XHamster`)
//! instead of the canonical lowercase `name`, and folding it here is what
//! keeps that display casing from ever affecting archive matching — and
//! [`archive_token_for`] prefers `InfoDict::extractor_key` (a plugin's
//! manifest `name`) over the display name in the first place, so a
//! renamed `display_name` never splits an archive either.
//!
//! # Concurrency
//!
//! Read and write paths take an advisory file lock via [`fs4::fs_std::FileExt`]
//! so concurrent rdlp processes (multiple terminals, automated pipelines, the
//! desktop app and a CLI run side-by-side) cannot interleave their writes and
//! corrupt entries. The lock is exclusive on writes and shared on reads, and
//! is released when the file handle drops.

use fs4::fs_std::FileExt;
use rdlp_redact::text::sanitize_for_line;
use rdlp_types::InfoDict;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// The extractor half of `info`'s archive line: `extractor_key` (yt-dlp
/// `extractor_key`; a plugin's manifest `name`) when set, else `extractor`
/// (an in-tree extractor's one name). The one place the choice is made —
/// every archive read and write goes through it, so a plugin's
/// `display_name` (which IS `extractor`) can never become an archive
/// token.
#[must_use]
pub fn archive_token_for(info: &InfoDict) -> &str {
    info.extractor_key.as_deref().unwrap_or(&info.extractor)
}

/// Build the canonical archive line for an `{extractor} {id}` pair.
///
/// The extractor token is ASCII-lowercased before formatting — safe because
/// plugin extractor names are constrained to `[a-z0-9][a-z0-9-]{0,63}` by
/// `validate_plugin_name`, and every built-in `ExtractorName` spelling is
/// plain ASCII too, so no non-ASCII casing rule is ever in play. The id's
/// case is left untouched: unlike extractor names, ids are opaque site
/// identifiers that may be legitimately case-sensitive. Its control
/// characters are not: a plugin-supplied id is untrusted, and a line break
/// in it would end this record early and write a second one under whatever
/// extractor token follows (`"1\nxvideos 456"` marks `xvideos 456` as
/// already downloaded). [`sanitize_for_line`] maps every control character
/// to `_` — here, at the one formatting point every read and write shares,
/// so a lookup and the record it is compared against sanitise identically;
/// the plugin boundary (`rdlp_plugin::convert::info_dict_from_wit`) applies
/// the same helper first, so this is the defence in depth.
pub fn archive_key(extractor: &str, id: &str) -> String {
    format!(
        "{} {}",
        extractor.to_ascii_lowercase(),
        sanitize_for_line(id)
    )
}

/// Load archive entries from a file into a `HashSet`.
///
/// Returns an empty set if the file does not exist. Blank lines and lines
/// starting with `#` are ignored. Each surviving line's extractor token
/// (the text before the first space) is lowercased so a line written by an
/// older, case-preserving rdlp build still canonicalises to the form
/// [`archive_key`] produces — see the module docs. A line with no space is
/// malformed (never produced by [`record_in_archive`]) and is kept verbatim;
/// it can never match a real query, since every valid key contains a space.
/// The same holds for any other whitespace separator (a tab, e.g.
/// `"XHamster\t123"`): rdlp and yt-dlp both always write a literal space, so
/// a tab-separated line is malformed too — kept verbatim, never normalised,
/// never matched.
/// Takes a shared lock for the duration of the read so a concurrent writer
/// cannot append a half-line under us.
pub fn load_archive(path: &Path) -> HashSet<String> {
    // Safe: sync helper invoked only via tokio::task::spawn_blocking (see orchestrator/mod.rs::load_archive_if_configured).
    #[allow(clippy::disallowed_methods)]
    let Ok(file) = std::fs::File::open(path) else {
        return HashSet::new();
    };

    // Shared lock: many readers may proceed concurrently; an exclusive
    // writer (record_in_archive) blocks until we release. If locking
    // fails (rare; e.g. NFS without lock support) fall back to the
    // unlocked read — better than a hard error here, since the archive
    // is best-effort tracking, not security-critical state. Named through
    // the fs4 trait because std 1.89 added an inherent `File::lock_shared`
    // that shadows it; naming the trait keeps the call resolving to fs4 on
    // every toolchain the workspace floor (`rust-version`, root Cargo.toml)
    // admits, rather than flipping implementation with the compiler.
    let _lock = FileExt::lock_shared(&file).ok();

    BufReader::new(&file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|line| {
            let trimmed = line.trim();
            match trimmed.split_once(' ') {
                Some((token, rest)) => format!("{} {rest}", token.to_ascii_lowercase()),
                None => trimmed.to_owned(),
            }
        })
        .collect()
    // `_lock` (when Some) drops here — the underlying `file` drop releases
    // the OS-level lock.
}

/// Check whether a video is already recorded in the archive.
pub fn is_in_archive(archive: &HashSet<String>, extractor: &str, id: &str) -> bool {
    archive.contains(&archive_key(extractor, id))
}

/// Append a completed download entry to the archive file.
///
/// Creates the file (and parent directories) if they don't exist. Holds an
/// exclusive advisory lock for the duration of the append so concurrent
/// `record_in_archive` calls (across processes or threads) cannot interleave
/// writes and produce a torn line.
pub fn record_in_archive(path: &Path, extractor: &str, id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        // Safe: sync helper invoked only via tokio::task::spawn_blocking (see orchestrator/mod.rs::record_in_archive).
        #[allow(clippy::disallowed_methods)]
        std::fs::create_dir_all(parent)?;
    }

    // Windows note: opening with `.append(true)` alone produces a handle with
    // only FILE_APPEND_DATA access. LockFileEx (what fs4 calls underneath)
    // requires GENERIC_READ or GENERIC_WRITE on the handle and fails with
    // ERROR_ACCESS_DENIED otherwise. Adding `.read(true)` widens the desired
    // access mask without changing the append semantic — the kernel still
    // adjusts the file offset to end-of-file on every write thanks to
    // `.append(true)`.
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;

    // Exclusive lock — blocks until any concurrent writer releases. NFS
    // / non-POSIX filesystems may return an error; in that environment
    // we surface it because skipping the lock would silently allow
    // interleaved writes. Filesystems that don't support advisory locks
    // are not a supported deployment target for the download archive.
    file.lock_exclusive()?;

    let result = writeln!(file, "{}", archive_key(extractor, id));

    // Explicit unlock (also released on drop, but this makes the
    // ordering with the write flush obvious).
    let _ = FileExt::unlock(&file);

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn load_empty_set_when_file_missing() {
        let archive = load_archive(Path::new("/nonexistent/archive.txt"));
        assert!(archive.is_empty());
    }

    #[test]
    fn load_skips_blank_and_comment_lines() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "# rdlp download archive").unwrap();
        writeln!(tmp).unwrap();
        writeln!(tmp, "SiteA abc123").unwrap();
        writeln!(tmp, "  # another comment  ").unwrap();
        writeln!(tmp, "SiteB xyz789").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert_eq!(archive.len(), 2);
        // Loaded lines are normalised to a lowercase extractor token (see
        // `archive_key`), so the canonical set holds "sitea"/"siteb", not
        // the on-disk casing.
        assert!(archive.contains("sitea abc123"));
        assert!(archive.contains("siteb xyz789"));
    }

    /// A cased line written before this fix (or by an older rdlp build)
    /// must still match a lowercase query — the extractor token is
    /// case-insensitive, matching yt-dlp's `make_archive_id`
    /// (`f'{ie_key.lower()} {video_id}'`).
    #[test]
    fn legacy_cased_line_matches_lowercase_query() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "XHamster 123").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert!(is_in_archive(&archive, "xhamster", "123"));
        assert!(is_in_archive(&archive, "XHamster", "123"));
    }

    /// The same id under a different extractor token must not collide —
    /// only the extractor token is case-folded, never merged across sites.
    #[test]
    fn same_id_other_site_does_not_match() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "xhamster 123").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert!(!is_in_archive(&archive, "xvideos", "123"));
    }

    /// Only the extractor token is case-folded; the id stays case-sensitive
    /// (ids may be case-sensitive site identifiers, unlike extractor names).
    #[test]
    fn id_is_case_sensitive() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "xhamster abc").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert!(!is_in_archive(&archive, "xhamster", "ABC"));
        assert!(is_in_archive(&archive, "xhamster", "abc"));
    }

    #[tokio::test]
    async fn record_writes_lowercase_token() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        record_in_archive(&path, "XHamster", "9").unwrap();

        // `load_archive` already lowercases on read, which would mask a
        // record-time regression — read the raw file contents directly
        // (via the async `tokio::fs` reader, since the crate bans
        // `std::fs::read_to_string`/`File::open` outside the two
        // pre-existing allowed sites) to prove `record_in_archive` itself
        // wrote the lowercase token, not just that a reader normalises it.
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("xhamster 9\n"));
        assert!(!contents.contains("XHamster"));
    }

    /// A malformed line (no space — never written by `record_in_archive`,
    /// but may appear in a hand-edited archive file) survives `load_archive`
    /// verbatim rather than panicking, and can never match a real query
    /// since `archive_key` always produces `"{extractor} {id}"`.
    #[test]
    fn malformed_line_without_space_survives_and_never_matches() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "NoSpaceHere").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert!(archive.contains("NoSpaceHere"));
        assert!(!is_in_archive(&archive, "nospacehere", ""));
        assert!(!is_in_archive(&archive, "NoSpaceHere", ""));
    }

    /// A tab (or any other whitespace) separator is equally malformed: rdlp
    /// and yt-dlp both always write a literal space, so `split_once(' ')`
    /// finds no space and the whole line is kept verbatim, never matching.
    #[test]
    fn malformed_line_with_tab_separator_survives_and_never_matches() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "XHamster\t123").unwrap();
        tmp.flush().unwrap();

        let archive = load_archive(tmp.path());
        assert!(archive.contains("XHamster\t123"));
        assert!(!is_in_archive(&archive, "xhamster", "123"));
        assert!(!is_in_archive(&archive, "XHamster", "123"));
    }

    /// An id carrying a line break must not split the record: the
    /// archive is line-oriented, so `"1\nxvideos 456"` written verbatim
    /// would put `xvideos 456` on its own line and mark ANOTHER
    /// extractor's video as already downloaded. `archive_key` maps every
    /// control character to `_` (`rdlp_redact::text::sanitize_for_line`),
    /// so exactly one line is written and the injected key never matches
    /// — while the sanitised id still matches its own lookup, since the
    /// lookup goes through the same `archive_key`.
    #[tokio::test]
    async fn id_with_a_line_break_writes_one_record_and_injects_nothing() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let injected = "1\nxvideos 456";

        record_in_archive(&path, "p", injected).unwrap();

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(contents.lines().count(), 1, "one record: {contents:?}");
        assert_eq!(contents, "p 1_xvideos 456\n");
        let archive = load_archive(&path);
        assert!(
            !is_in_archive(&archive, "xvideos", "456"),
            "the injected second record must not exist"
        );
        assert!(
            is_in_archive(&archive, "p", injected),
            "the sanitised id still matches its own lookup"
        );
    }

    /// CR and TAB are `Cc` too and get the same placeholder; a space is
    /// not (`load_archive` splits on the FIRST space, so an id with one
    /// round-trips today and must keep doing so).
    #[tokio::test]
    async fn id_control_characters_become_placeholders_but_a_space_survives() {
        for (id, expected_line) in [
            ("1\rxvideos 456", "p 1_xvideos 456\n"),
            ("1\txvideos 456", "p 1_xvideos 456\n"),
            ("1 two", "p 1 two\n"),
        ] {
            let tmp = NamedTempFile::new().unwrap();
            let path = tmp.path().to_path_buf();
            record_in_archive(&path, "p", id).unwrap();
            let contents = tokio::fs::read_to_string(&path).await.unwrap();
            assert_eq!(contents, expected_line, "id {id:?}");
            assert!(is_in_archive(&load_archive(&path), "p", id), "id {id:?}");
        }
    }

    /// A plugin's `InfoDict` carries its manifest `name` as
    /// `extractor_key` and its `display_name` as `extractor`; the archive
    /// token is the key, so renaming the display never splits an archive.
    /// An in-tree extractor has no key and the token is `extractor`.
    #[test]
    fn archive_token_prefers_extractor_key_over_the_display_name() {
        let mut plugin = InfoDict::new("9", "t", "XHamster Display", "https://x.test/9");
        plugin.extractor_key = Some("xhamster".to_string());
        assert_eq!(archive_token_for(&plugin), "xhamster");

        let in_tree = InfoDict::new("9", "t", "XHamster", "https://x.test/9");
        assert_eq!(archive_token_for(&in_tree), "XHamster");
    }

    #[test]
    fn is_in_archive_checks_extractor_and_id() {
        // The set holds the canonical (lowercased-token) form `load_archive`
        // produces — `archive_key` is the single place that shape is built.
        let mut archive = HashSet::new();
        archive.insert(archive_key("SiteA", "abc123"));

        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(!is_in_archive(&archive, "SiteA", "other"));
        assert!(!is_in_archive(&archive, "SiteB", "abc123"));
    }

    #[test]
    fn record_appends_entry() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        record_in_archive(&path, "SiteA", "abc123").unwrap();
        record_in_archive(&path, "SiteB", "xyz789").unwrap();

        let archive = load_archive(&path);
        assert_eq!(archive.len(), 2);
        assert!(is_in_archive(&archive, "SiteA", "abc123"));
        assert!(is_in_archive(&archive, "SiteB", "xyz789"));
    }

    #[test]
    fn record_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("archive.txt");

        record_in_archive(&path, "SiteA", "id1").unwrap();

        let archive = load_archive(&path);
        assert!(is_in_archive(&archive, "SiteA", "id1"));
    }

    /// Regression: verify that two writers contending for the same archive
    /// produce a complete, well-formed file rather than interleaved bytes.
    /// This is the failing test (against the pre-flock implementation) that
    /// the bug-fix-requires-failing-test rule asks for.
    ///
    /// We launch many threads that each write a long marker line. With
    /// `O_APPEND` alone, glibc is allowed to split `write()` into multiple
    /// system calls under load — historically this is the source of torn
    /// lines. With `flock`, every line lands atomically.
    #[test]
    fn record_appends_are_atomic_under_thread_contention() {
        use std::sync::Arc;
        use std::thread;

        const THREADS: usize = 32;
        const PER_THREAD: usize = 50;

        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("archive.txt"));
        // Long-ish per-line payload increases the chance that an unprotected
        // write would tear. Using `extractor` as a fixed prefix and `id` as
        // a marker makes interleavings detectable: every recorded line MUST
        // be `SiteA <numeric-id>` exactly.

        let mut handles = Vec::with_capacity(THREADS);
        for t in 0..THREADS {
            let path = path.clone();
            handles.push(thread::spawn(move || {
                for i in 0..PER_THREAD {
                    let id = format!("{t:02}-{i:04}-padding-padding-padding-padding");
                    record_in_archive(&path, "SiteA", &id).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let archive = load_archive(&path);
        assert_eq!(
            archive.len(),
            THREADS * PER_THREAD,
            "expected {} unique entries, got {} (interleaved write detected)",
            THREADS * PER_THREAD,
            archive.len()
        );

        // Every entry MUST start with `sitea ` (lowercased by `load_archive`
        // normalisation) and be followed by a well-formed marker. A torn
        // write would leave entries that don't match this shape.
        for entry in &archive {
            let suffix = entry
                .strip_prefix("sitea ")
                .unwrap_or_else(|| panic!("malformed entry: {entry:?}"));
            assert!(
                suffix.contains("-padding-padding-padding-padding"),
                "torn line detected: {entry:?}"
            );
        }
    }
}
