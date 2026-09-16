//! A `log::Log` sink that captures `(target, message)` pairs, for tests
//! that assert what a code path logged — a refusal on a plugin's target, a
//! playlist page failure carrying its page number in the message text.
//!
//! One copy for the workspace: this crate's own unit tests use it directly
//! and `rdlp-plugin`'s `test_support::unit` re-exports it (that crate's
//! former private copy was the second one; a third in this crate is what
//! `extract-before-you-duplicate` forbids). Compiled into the library
//! behind the `test-support` feature rather than `cfg(test)` because a
//! sibling crate's tests link this lib as an external crate and cannot see
//! `cfg(test)` items; `scripts/check-test-only-features-not-in-release.sh`
//! proves no release binary enables the feature.
//!
//! `log::set_logger` accepts one logger per process, so the buffer is
//! process-global and never cleared — each assertion looks for its own
//! distinctive message (a URL, a count, a bound) instead of a before/after
//! length, which a concurrently running test's own line would flip.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// `(target, message)` pairs captured from the `log` facade.
pub type LogEntries = Arc<Mutex<Vec<(String, String)>>>;

struct CapturingLogger {
    entries: LogEntries,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((record.target().to_string(), record.args().to_string()));
    }
    fn flush(&self) {}
}

/// The process-global capture buffer, installing the logger on first use.
///
/// # Panics
///
/// If another logger was installed first in this test binary — the
/// capture would silently see nothing, so it fails loudly instead.
#[must_use]
pub fn captured_logs() -> LogEntries {
    static CAPTURED: OnceLock<LogEntries> = OnceLock::new();
    Arc::clone(CAPTURED.get_or_init(|| {
        let entries: LogEntries = Arc::new(Mutex::new(Vec::new()));
        let logger: &'static CapturingLogger = Box::leak(Box::new(CapturingLogger {
            entries: Arc::clone(&entries),
        }));
        log::set_logger(logger).unwrap_or_else(|e| panic!("another logger is installed: {e}"));
        // `Debug`, not `Warn`: page-fetch and per-call budget lines the
        // playlist tests count are `debug!` — `Warn` would silently drop
        // them before they ever reached this logger.
        log::set_max_level(log::LevelFilter::Debug);
        entries
    }))
}

/// Every captured entry whose message contains `needle` and — when `target`
/// is `Some` — whose target equals it, cloned out so the lock is released
/// before any assertion panics. The one filter behind every helper here:
/// the buffer is shared with every other test in the binary, so a needle
/// must be distinctive enough to select one test's lines, and filtering on
/// the target as well is how a test whose message text is shared with a
/// sibling (the same URL through two fixtures) keeps its count honest.
fn captured_matching(
    logs: &LogEntries,
    target: Option<&str>,
    needle: &str,
) -> Vec<(String, String)> {
    logs.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(t, m)| target.is_none_or(|want| t == want) && m.contains(needle))
        .cloned()
        .collect()
}

/// First captured entry whose message contains `needle`. Not `must_use`:
/// the panic below is an assertion in its own right, and many callers want
/// only that.
///
/// # Panics
///
/// When no entry contains `needle`, naming every captured entry.
pub fn captured_entry_containing(logs: &LogEntries, needle: &str) -> (String, String) {
    captured_matching(logs, None, needle)
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            let entries = logs.lock().unwrap_or_else(PoisonError::into_inner).clone();
            panic!("no entry containing {needle:?} among {entries:?}")
        })
}

/// How many captured entries contain `needle` — for the "exactly once"
/// assertions (one warning per refusal class, one fetch per page, one
/// runner call per entry).
#[must_use]
pub fn captured_count_containing(logs: &LogEntries, needle: &str) -> usize {
    captured_matching(logs, None, needle).len()
}

/// [`captured_count_containing`], restricted to entries logged on `target`
/// — for a message text another test in the binary also produces, where
/// only the target tells the two apart.
#[must_use]
pub fn captured_count_on_target(logs: &LogEntries, target: &str, needle: &str) -> usize {
    captured_matching(logs, Some(target), needle).len()
}
