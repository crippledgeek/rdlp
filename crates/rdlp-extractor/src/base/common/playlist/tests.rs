use super::*;

use crate::hls::test_support::test_ctx_with;
use rdlp_types::Config;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

fn entry(i: usize) -> PlaylistEntry {
    PlaylistEntry {
        url: format!("https://x.test/v{i}"),
        id: Some(format!("v{i}")),
        title: None,
    }
}

/// Entries numbered `start..start + n` (1-based listing positions), so
/// consecutive pages carry distinct URLs.
fn entries(start: usize, n: usize) -> Vec<PlaylistEntry> {
    (start..start + n).map(entry).collect()
}

fn page(entries: Vec<PlaylistEntry>, has_more: bool) -> PlaylistPage {
    PlaylistPage {
        entries,
        has_more,
        playlist_id: Some("pl".to_string()),
        playlist_title: Some("The List".to_string()),
        total_estimate: None,
    }
}

type PageFn = Box<dyn Fn(u32) -> Result<PlaylistPage> + Send + Sync>;
type BehaviorFn = Box<dyn Fn(&PlaylistEntry) -> Behavior + Send + Sync>;

/// `n_pages` pages of `per_page` entries; the last page has `has_more = false`.
fn scripted(n_pages: u32, per_page: usize) -> PageFn {
    Box::new(move |p| {
        if p > n_pages {
            return Err(RdlpError::extraction("past script", "https://x.test"));
        }
        let start = (p as usize - 1) * per_page + 1;
        Ok(page(entries(start, per_page), p < n_pages))
    })
}

/// How the mock resolves one entry: wait `delay`, then succeed or fail.
/// The wait runs under the request's budget the way a real implementor
/// enforces it (the plugin runner's own timeout), unless
/// `ignores_budget` — the implementor the loop's guard exists for.
#[derive(Debug, Clone, Copy, Default)]
struct Behavior {
    delay: Duration,
    fail: bool,
    ignores_budget: bool,
}

fn failing() -> Behavior {
    Behavior {
        fail: true,
        ..Behavior::default()
    }
}

fn slow(ms: u64) -> Behavior {
    Behavior {
        delay: Duration::from_millis(ms),
        ..Behavior::default()
    }
}

/// The error an honouring mock returns when its budget runs out —
/// the implementor's own timeout error, distinct from the loop's guard.
const MOCK_BUDGET_EXCEEDED: &str = "mock budget exceeded";

/// Decrements the mock's in-flight counter when the resolve future
/// ends — including when `timeout` DROPS it mid-`sleep`, which a
/// decrement after the `.await` would miss and leave `max_in_flight`
/// inflated for the rest of the run.
struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct MockPlaylist {
    pages: PageFn,
    behavior: BehaviorFn,
    rate_limit: Duration,
    fetches: AtomicUsize,
    resolves: AtomicUsize,
    // In-flight accounting is atomic so nothing is held across the
    // mock's `sleep` (tokio-audit: no lock across an `.await`).
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    resolved_urls: Mutex<Vec<String>>,
}

impl MockPlaylist {
    fn new(pages: PageFn) -> Self {
        Self {
            pages,
            behavior: Box::new(|_| Behavior::default()),
            rate_limit: Duration::ZERO,
            fetches: AtomicUsize::new(0),
            resolves: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            resolved_urls: Mutex::new(Vec::new()),
        }
    }

    fn with_behavior(mut self, behavior: BehaviorFn) -> Self {
        self.behavior = behavior;
        self
    }

    fn with_rate_limit(mut self, rate_limit: Duration) -> Self {
        self.rate_limit = rate_limit;
        self
    }

    fn resolved_urls(&self) -> Vec<String> {
        self.resolved_urls
            .lock()
            .expect("test mutex is never poisoned")
            .clone()
    }
}

impl PagedPlaylist for MockPlaylist {
    fn name(&self) -> &str {
        "Mock"
    }
    fn page_rate_limit(&self) -> Duration {
        self.rate_limit
    }
    async fn fetch_playlist_page(
        &self,
        _url: &str,
        page: u32,
        _ctx: &ExtractionContext,
    ) -> Result<PlaylistPage> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        (self.pages)(page)
    }
    async fn resolve_entry(
        &self,
        request: ResolveRequest<'_>,
        _ctx: &ExtractionContext,
    ) -> Result<InfoDict> {
        let entry = request.entry;
        self.resolves.fetch_add(1, Ordering::SeqCst);
        self.resolved_urls
            .lock()
            .expect("test mutex is never poisoned")
            .push(entry.url.clone());
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let _guard = InFlight(&self.in_flight);
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        let behavior = (self.behavior)(entry);
        let work = tokio::time::sleep(behavior.delay);
        if behavior.ignores_budget {
            work.await;
        } else {
            tokio::time::timeout(request.budget, work)
                .await
                .map_err(|_| RdlpError::extraction(MOCK_BUDGET_EXCEEDED, &entry.url))?;
        }
        if behavior.fail {
            return Err(RdlpError::extraction("mock resolve failure", &entry.url));
        }
        let id = entry.id.clone().unwrap_or_default();
        Ok(InfoDict::new(id.clone(), id, "mock", &entry.url))
    }
}

fn cfg(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config {
        verbose: false,
        ..Config::default()
    };
    f(&mut c);
    c
}

async fn run(mock: &MockPlaylist, config: Config) -> Result<Vec<InfoDict>> {
    mock.extract_all_entries("https://x.test/list", &test_ctx_with(config))
        .await
}

/// The URL an `Extraction` error names — the failed entry's own URL
/// under the abort policy.
fn error_url(err: &RdlpError) -> String {
    match err {
        RdlpError::Extraction { url, .. } => url
            .as_ref()
            .expect("abort policy errors name the entry")
            .expose()
            .to_string(),
        other => panic!("expected an Extraction error, got {other}"),
    }
}

fn indices(out: &[InfoDict]) -> Vec<usize> {
    out.iter()
        .map(|i| i.playlist_index.expect("index stamped"))
        .collect()
}

#[tokio::test]
async fn stops_when_has_more_is_false_and_stamps_index_count() {
    let mock = MockPlaylist::new(scripted(2, 2));
    let out = run(&mock, cfg(|_| {})).await.expect("all entries resolve");
    assert_eq!(indices(&out), vec![1, 2, 3, 4]);
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 2);
    for info in &out {
        assert_eq!(info.playlist_count, Some(4));
        assert_eq!(info.playlist_id.as_deref(), Some("pl"));
        // yt-dlp: `playlist` is the title when the site gives one.
        assert_eq!(info.playlist.as_deref(), Some("The List"));
        assert_eq!(info.playlist_title.as_deref(), Some("The List"));
    }
}

/// `playlist` falls back to the id when page one carries no title
/// (yt-dlp `playlist = playlist_title or playlist_id`).
#[tokio::test]
async fn playlist_field_falls_back_to_the_id_without_a_title() {
    let untitled: PageFn = Box::new(|_| {
        let mut pg = page(entries(1, 2), false);
        pg.playlist_title = None;
        Ok(pg)
    });
    let mock = MockPlaylist::new(untitled);
    let out = run(&mock, cfg(|_| {})).await.expect("all resolve");
    assert_eq!(out.len(), 2);
    for info in &out {
        assert_eq!(info.playlist.as_deref(), Some("pl"));
        assert_eq!(info.playlist_title, None);
    }
}

#[tokio::test]
async fn range_is_applied_before_any_resolve_and_only_needed_pages_are_fetched() {
    let mock = MockPlaylist::new(scripted(3, 2));
    let out = run(&mock, cfg(|c| c.playlist_items = Some("2-3".to_string())))
        .await
        .expect("selected entries resolve");
    assert_eq!(indices(&out), vec![2, 3]);
    assert_eq!(
        mock.fetches.load(Ordering::SeqCst),
        2,
        "max_index 3 needs pages 1-2 only"
    );
    assert_eq!(
        mock.resolved_urls(),
        vec!["https://x.test/v2", "https://x.test/v3"],
        "only the selected positions are ever resolved"
    );
    // `playlist_count` is the LISTED count (yt-dlp `n_entries`), not the
    // resolved count.
    assert!(out.iter().all(|i| i.playlist_count == Some(4)));
}

#[tokio::test]
async fn playlist_start_end_window() {
    let mock = MockPlaylist::new(scripted(3, 2));
    let out = run(
        &mock,
        cfg(|c| {
            c.playlist_start = 3;
            c.playlist_end = Some(3);
        }),
    )
    .await
    .expect("selected entry resolves");
    assert_eq!(indices(&out), vec![3]);
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 2);
    assert_eq!(mock.resolves.load(Ordering::SeqCst), 1);
}

/// The skip branch also emits a `warn!`; this crate has no log capture,
/// so the log line is not asserted — only the pruning it accompanies.
#[tokio::test]
async fn ignore_errors_true_skips_false_aborts() {
    let fail_v2: BehaviorFn = Box::new(|e| {
        if e.url.ends_with("/v2") {
            failing()
        } else {
            Behavior::default()
        }
    });
    // Default (unset) policy is skip.
    let mock = MockPlaylist::new(scripted(2, 2)).with_behavior(fail_v2);
    let out = run(&mock, cfg(|_| {}))
        .await
        .expect("skip policy returns the rest");
    assert_eq!(indices(&out), vec![1, 3, 4]);
    assert!(out.iter().all(|i| i.playlist_count == Some(4)));

    // Explicit `true` is the same.
    let out = run(&mock, cfg(|c| c.playlist_ignore_errors = Some(true)))
        .await
        .expect("skip policy returns the rest");
    assert_eq!(indices(&out), vec![1, 3, 4]);

    // `false` aborts with the failed entry's own error.
    let err = run(&mock, cfg(|c| c.playlist_ignore_errors = Some(false)))
        .await
        .expect_err("abort policy propagates the failure");
    assert!(
        err.to_string().contains("mock resolve failure"),
        "got {err}"
    );
    assert_eq!(error_url(&err), "https://x.test/v2");
}

#[tokio::test(start_paused = true)]
async fn abort_reports_the_first_failure_by_listing_position() {
    // Position 3 fails instantly; position 2 fails after a delay. Under
    // `buffer_unordered` #3 completes first, but the reported failure
    // must be #2 — the first by position, not by completion.
    let behavior: BehaviorFn = Box::new(|e| {
        if e.url.ends_with("/v2") {
            Behavior {
                delay: Duration::from_millis(10),
                fail: true,
                ..Behavior::default()
            }
        } else if e.url.ends_with("/v3") {
            failing()
        } else {
            Behavior::default()
        }
    });
    let mock = MockPlaylist::new(scripted(1, 4)).with_behavior(behavior);
    let err = run(
        &mock,
        cfg(|c| {
            c.playlist_concurrency = Some(4);
            c.playlist_ignore_errors = Some(false);
        }),
    )
    .await
    .expect_err("abort policy propagates a failure");
    assert_eq!(error_url(&err), "https://x.test/v2");
}

#[tokio::test(start_paused = true)]
async fn concurrency_one_is_strictly_sequential() {
    let mock = MockPlaylist::new(scripted(1, 4)).with_behavior(Box::new(|_| slow(20)));
    let out = run(&mock, cfg(|_| {})).await.expect("all resolve");
    assert_eq!(out.len(), 4);
    assert_eq!(mock.max_in_flight.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn concurrency_n_bounds_in_flight() {
    let mock = MockPlaylist::new(scripted(1, 6)).with_behavior(Box::new(|_| slow(20)));
    let out = run(&mock, cfg(|c| c.playlist_concurrency = Some(3)))
        .await
        .expect("all resolve");
    assert_eq!(out.len(), 6);
    // Structural under `buffer_unordered(3)`: the first three futures are
    // all polled (and parked on their timers) before any completes.
    assert_eq!(mock.max_in_flight.load(Ordering::SeqCst), 3);
}

/// The per-item budget reaches the implementor and is the ONE timer on
/// the entry: an implementor that honours it fails with its own timeout
/// error (for a plugin, the runner's `Timeout` — a strike), which the
/// loop prunes or propagates like any other failure. The loop's guard
/// never fires here.
#[tokio::test(start_paused = true)]
async fn per_item_budget_is_enforced_by_the_implementor_and_prunes_the_slow_entry() {
    let slow_v2: BehaviorFn = Box::new(|e| {
        if e.url.ends_with("/v2") {
            slow(1500)
        } else {
            Behavior::default()
        }
    });
    let mock = MockPlaylist::new(scripted(1, 3)).with_behavior(slow_v2);
    let started = tokio::time::Instant::now();
    let out = run(&mock, cfg(|c| c.playlist_item_timeout = Some(1)))
        .await
        .expect("skip policy prunes the entry that ran out of budget");
    assert_eq!(indices(&out), vec![1, 3]);
    assert!(
        started.elapsed() < Duration::from_secs(1) + PLAYLIST_ITEM_TIMEOUT_GRACE,
        "the implementor's own timer ended the entry, not the loop's guard"
    );

    // Under the abort policy the implementor's timeout error is the
    // playlist's error — its own message, not the guard's.
    let err = run(
        &mock,
        cfg(|c| {
            c.playlist_item_timeout = Some(1);
            c.playlist_ignore_errors = Some(false);
        }),
    )
    .await
    .expect_err("abort policy propagates the implementor's timeout");
    assert!(err.to_string().contains(MOCK_BUDGET_EXCEEDED), "got {err}");
    assert_eq!(error_url(&err), "https://x.test/v2");
}

/// The loop's guard fires only for an implementor that ignores its
/// budget, and only `PLAYLIST_ITEM_TIMEOUT_GRACE` after the budget —
/// so it can never race an implementor's own timer.
#[tokio::test(start_paused = true)]
async fn guard_fires_only_for_an_implementor_that_ignores_the_budget() {
    let budget = Duration::from_secs(1);
    let runaway_v2: BehaviorFn = Box::new(|e| {
        if e.url.ends_with("/v2") {
            Behavior {
                delay: Duration::from_secs(3600),
                ignores_budget: true,
                ..Behavior::default()
            }
        } else {
            Behavior::default()
        }
    });
    let mock = MockPlaylist::new(scripted(1, 3)).with_behavior(runaway_v2);
    let started = tokio::time::Instant::now();
    let out = run(&mock, cfg(|c| c.playlist_item_timeout = Some(1)))
        .await
        .expect("skip policy prunes the entry the guard gave up on");
    assert_eq!(indices(&out), vec![1, 3]);
    let elapsed = started.elapsed();
    assert!(
        elapsed >= budget + PLAYLIST_ITEM_TIMEOUT_GRACE,
        "the guard waits the grace past the budget: {elapsed:?}"
    );
    assert!(
        elapsed < budget + PLAYLIST_ITEM_TIMEOUT_GRACE + Duration::from_secs(1),
        "the guard did fire: {elapsed:?}"
    );

    // Under the abort policy the guard's verdict names the entry.
    let err = run(
        &mock,
        cfg(|c| {
            c.playlist_item_timeout = Some(1);
            c.playlist_ignore_errors = Some(false);
        }),
    )
    .await
    .expect_err("abort policy propagates the guard's timeout");
    assert!(err.to_string().contains("ignored its budget"), "got {err}");
    assert_eq!(error_url(&err), "https://x.test/v2");
}

#[tokio::test(start_paused = true)]
async fn timed_out_entry_releases_its_concurrency_slot() {
    // Entry #1 is dropped by the 1 s budget while #2..#4 finish; with
    // concurrency 2 the in-flight peak is 2 both before and after the
    // drop, so a leaked slot would show as a peak of 3 only if the
    // counter over-reported — the guard keeps it at exactly 2.
    let slow_v1: BehaviorFn = Box::new(|e| {
        if e.url.ends_with("/v1") {
            slow(5000)
        } else {
            slow(20)
        }
    });
    let mock = MockPlaylist::new(scripted(1, 4)).with_behavior(slow_v1);
    let out = run(
        &mock,
        cfg(|c| {
            c.playlist_concurrency = Some(2);
            c.playlist_item_timeout = Some(1);
        }),
    )
    .await
    .expect("skip policy prunes the timed-out entry");
    assert_eq!(indices(&out), vec![2, 3, 4]);
    assert_eq!(mock.max_in_flight.load(Ordering::SeqCst), 2);
    assert_eq!(
        mock.in_flight.load(Ordering::SeqCst),
        0,
        "the dropped resolve released its slot"
    );
}

#[tokio::test]
async fn playlist_count_prefers_the_site_total_estimate() {
    // Estimate present on page one → count = estimate, even though the
    // range stops the listing at page two (4 of 6 listed).
    let with_estimate: PageFn = Box::new(|p| {
        let mut pg = page(entries((p as usize - 1) * 2 + 1, 2), p < 3);
        pg.total_estimate = if p == 1 { Some(40) } else { None };
        Ok(pg)
    });
    let mock = MockPlaylist::new(with_estimate);
    let out = run(&mock, cfg(|c| c.playlist_items = Some("2-3".to_string())))
        .await
        .expect("selected entries resolve");
    assert_eq!(indices(&out), vec![2, 3]);
    assert!(out.iter().all(|i| i.playlist_count == Some(40)));

    // Estimate absent → the listed count (a lower bound under a range).
    let mock = MockPlaylist::new(scripted(3, 2));
    let out = run(&mock, cfg(|c| c.playlist_items = Some("2-3".to_string())))
        .await
        .expect("selected entries resolve");
    assert!(out.iter().all(|i| i.playlist_count == Some(4)));

    // An estimate below what was actually listed is not trusted.
    let low_estimate: PageFn = Box::new(|_| {
        let mut pg = page(entries(1, 3), false);
        pg.total_estimate = Some(1);
        Ok(pg)
    });
    let mock = MockPlaylist::new(low_estimate);
    let out = run(&mock, cfg(|_| {})).await.expect("all resolve");
    assert!(out.iter().all(|i| i.playlist_count == Some(3)));
}

#[tokio::test]
async fn unparsable_playlist_items_is_an_extraction_error() {
    let mock = MockPlaylist::new(scripted(2, 2));
    let err = run(&mock, cfg(|c| c.playlist_items = Some("3-1".to_string())))
        .await
        .expect_err("a reversed range is rejected");
    assert!(matches!(err, RdlpError::Extraction { .. }), "got {err:?}");
    assert_eq!(error_url(&err), "https://x.test/list");
    assert_eq!(
        mock.fetches.load(Ordering::SeqCst),
        0,
        "rejected before the first page is fetched"
    );
    assert_eq!(mock.resolves.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn range_beyond_the_listing_yields_ok_empty() {
    let mock = MockPlaylist::new(scripted(2, 2));
    let out = run(&mock, cfg(|c| c.playlist_start = 10))
        .await
        .expect("an out-of-range window is not an error");
    assert!(out.is_empty());
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 2);
    assert_eq!(mock.resolves.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn max_playlist_size_caps_listing() {
    let per_page = 600;
    let mock = MockPlaylist::new(Box::new(move |p| {
        let start = (p as usize - 1) * per_page + 1;
        Ok(page(entries(start, per_page), true))
    }));
    let out = run(&mock, cfg(|_| {})).await.expect("all resolve");
    assert_eq!(out.len(), MAX_PLAYLIST_SIZE);
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 2);
    assert!(
        out.iter()
            .all(|i| i.playlist_count == Some(MAX_PLAYLIST_SIZE))
    );
}

#[tokio::test]
async fn first_page_error_propagates_later_page_error_returns_partial() {
    let first_fails = MockPlaylist::new(Box::new(|_| {
        Err(RdlpError::extraction("page 1 down", "https://x.test"))
    }));
    let err = run(&first_fails, cfg(|_| {}))
        .await
        .expect_err("first page failure propagates");
    assert!(err.to_string().contains("page 1 down"), "got {err}");
    assert_eq!(first_fails.resolves.load(Ordering::SeqCst), 0);

    let second_fails = MockPlaylist::new(Box::new(|p| {
        if p == 1 {
            Ok(page(entries(1, 2), true))
        } else {
            Err(RdlpError::extraction("page 2 down", "https://x.test"))
        }
    }));
    let out = run(&second_fails, cfg(|_| {}))
        .await
        .expect("later page failure returns the partial listing");
    assert_eq!(indices(&out), vec![1, 2]);
    assert!(out.iter().all(|i| i.playlist_count == Some(2)));
    // The same partial result under an explicit skip policy.
    let out = run(
        &second_fails,
        cfg(|c| c.playlist_ignore_errors = Some(true)),
    )
    .await
    .expect("skip policy resolves the entries listed so far");
    assert_eq!(indices(&out), vec![1, 2]);
}

/// Under `AbortOnFirstFailure` a later page's listing failure is the
/// playlist's failure: the page error comes back and nothing listed
/// so far is resolved — a partial playlist is exactly what the
/// operator opted out of.
#[tokio::test]
async fn later_page_error_aborts_under_abort_policy_without_resolving() {
    let second_fails = MockPlaylist::new(Box::new(|p| {
        if p == 1 {
            Ok(page(entries(1, 2), true))
        } else {
            Err(RdlpError::extraction("page 2 down", "https://x.test"))
        }
    }));
    let err = run(
        &second_fails,
        cfg(|c| c.playlist_ignore_errors = Some(false)),
    )
    .await
    .expect_err("abort policy propagates a later page's failure");
    assert!(err.to_string().contains("page 2 down"), "got {err}");
    assert_eq!(
        second_fails.resolves.load(Ordering::SeqCst),
        0,
        "nothing is resolved once the listing has failed under abort"
    );
}

#[tokio::test]
async fn empty_first_page_is_ok_and_empty() {
    let mock = MockPlaylist::new(Box::new(|_| Ok(page(Vec::new(), true))));
    let out = run(&mock, cfg(|_| {}))
        .await
        .expect("empty page is not an error");
    assert!(out.is_empty());
    assert_eq!(mock.fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn pages_are_paced() {
    let pace = Duration::from_millis(20);
    let mock = MockPlaylist::new(scripted(3, 1)).with_rate_limit(pace);
    let started = tokio::time::Instant::now();
    let out = run(&mock, cfg(|_| {})).await.expect("all resolve");
    assert_eq!(out.len(), 3);
    let elapsed = started.elapsed();
    // Two gaps between three pages, and none after the last one.
    assert!(elapsed >= 2 * pace, "elapsed {elapsed:?}");
    assert!(
        elapsed < 3 * pace,
        "no pacing sleep after the last page: {elapsed:?}"
    );
}

// ---- Log lines carry their payload in the message text (#768 C1) ----
//
// The CLI bridge and `tauri-plugin-log` render `record.args()` only
// (CODING_RULES "Boundary Logging"): a value passed as a `log` kv pair
// compiles and is dropped before any sink sees it. Each line below is
// asserted on its rendered TEXT, on a URL no other test in this binary
// logs, so the page number / position / count are provably in the message.

/// Entries under a per-test URL prefix, so an item line (keyed on the
/// entry URL, not the playlist's) is distinguishable from every other
/// test's in the process-global capture buffer.
fn entries_under(prefix: &str, n: usize) -> Vec<PlaylistEntry> {
    (1..=n)
        .map(|i| PlaylistEntry {
            url: format!("https://x.test/{prefix}/v{i}"),
            id: Some(format!("v{i}")),
            title: None,
        })
        .collect()
}

/// A page failure names the page and the URL; a skipped item names its
/// position out of the total; a range selecting nothing names the listed
/// count — all in the text.
#[tokio::test]
async fn skip_policy_log_lines_carry_page_position_and_count_in_the_text() {
    use crate::log_capture::{captured_entry_containing, captured_logs};

    let logs = captured_logs();
    let url = "https://x.test/list-c1-skip";
    let mock = MockPlaylist::new(Box::new(|p| {
        if p == 1 {
            Ok(page(entries_under("c1-skip", 3), true))
        } else {
            Err(RdlpError::extraction("page 2 down", "https://x.test"))
        }
    }))
    .with_behavior(Box::new(|e| {
        if e.url.ends_with("/v2") {
            failing()
        } else {
            Behavior::default()
        }
    }));
    let out = mock
        .extract_all_entries(url, &test_ctx_with(cfg(|_| {})))
        .await
        .expect("skip policy resolves the rest");
    assert_eq!(indices(&out), vec![1, 3]);

    let (_, page_line) =
        captured_entry_containing(&logs, &format!("3 entries listed so far ({url})"));
    assert!(
        page_line.contains("Playlist page 2 failed") && page_line.contains("page 2 down"),
        "page number and cause must be in the text: {page_line:?}"
    );
    let (_, item_line) = captured_entry_containing(&logs, "https://x.test/c1-skip/v2)");
    assert!(
        item_line.contains("Playlist item 2/3 failed, skipping"),
        "position and total must be in the text: {item_line:?}"
    );
    captured_entry_containing(
        &logs,
        &format!("Playlist listed 3 entries through page 2, 3 selected ({url})"),
    );
}

/// Under the abort policy the page line names the page too, and a range
/// that selects nothing names how many were listed.
#[tokio::test]
async fn abort_and_empty_range_log_lines_carry_their_values_in_the_text() {
    use crate::log_capture::{captured_entry_containing, captured_logs};

    let logs = captured_logs();
    let url = "https://x.test/list-c1-abort";
    let mock = MockPlaylist::new(Box::new(|p| {
        if p == 1 {
            Ok(page(entries(1, 2), true))
        } else {
            Err(RdlpError::extraction("page 2 down", "https://x.test"))
        }
    }));
    mock.extract_all_entries(
        url,
        &test_ctx_with(cfg(|c| c.playlist_ignore_errors = Some(false))),
    )
    .await
    .expect_err("abort policy returns the page error");
    let (_, line) = captured_entry_containing(&logs, &format!("aborting ({url})"));
    assert!(
        line.contains("Playlist page 2 failed, aborting"),
        "page number must be in the text: {line:?}"
    );

    let url = "https://x.test/list-c1-empty";
    let mock = MockPlaylist::new(scripted(1, 2));
    let out = mock
        .extract_all_entries(url, &test_ctx_with(cfg(|c| c.playlist_start = 5)))
        .await
        .expect("an empty selection is Ok(empty)");
    assert!(out.is_empty());
    let (_, line) = captured_entry_containing(&logs, &format!("listed entries ({url})"));
    assert!(
        line.contains("selects nothing from the 2 listed entries"),
        "listed count must be in the text: {line:?}"
    );
}

/// The guard line for an implementor that ignored its budget names the
/// position, total, and guard seconds in the text.
#[tokio::test(start_paused = true)]
async fn guard_timeout_log_line_carries_position_and_seconds_in_the_text() {
    use crate::log_capture::{captured_entry_containing, captured_logs};

    let logs = captured_logs();
    let url = "https://x.test/list-c1-guard";
    let mock = MockPlaylist::new(Box::new(|_| Ok(page(entries_under("c1-guard", 2), false))))
        .with_behavior(Box::new(|e| {
            if e.url.ends_with("/v1") {
                Behavior {
                    delay: Duration::from_secs(60),
                    ignores_budget: true,
                    ..Behavior::default()
                }
            } else {
                Behavior::default()
            }
        }));
    let out = mock
        .extract_all_entries(
            url,
            &test_ctx_with(cfg(|c| c.playlist_item_timeout = Some(1))),
        )
        .await
        .expect("the guard prunes the runaway entry");
    assert_eq!(indices(&out), vec![2]);
    let (_, line) = captured_entry_containing(&logs, "https://x.test/c1-guard/v1)");
    assert!(
        line.contains("Playlist item 1/2 ignored its budget and timed out after 6s"),
        "position, total and guard seconds must be in the text: {line:?}"
    );
}
