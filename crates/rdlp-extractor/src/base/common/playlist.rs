//! Host-owned playlist loop (rdlp#762 slice C, spec D1).
//!
//! The extractor — in-tree or plugin — lists entries page by page; THIS
//! loop selects the requested range before resolving anything (yt-dlp
//! `__process_playlist`, gallery-dl's job runner), resolves entries under
//! bounded concurrency and a per-item timeout, prunes or aborts on failure
//! per configuration, and stamps the playlist fields. Nothing about
//! concurrency or failure policy crosses the extractor boundary.
//!
//! Sibling of [`PagedSearch`](super::PagedSearch): same shape (one required
//! per-page hook, a provided loop that must not be overridden, the same
//! first-page-fails-vs-later-page-fails asymmetry).

use futures::StreamExt;
use log::{debug, warn};
use rdlp_core::{ExtractionContext, RdlpError, Result};
use rdlp_redact::RedactedUrl;
use rdlp_types::{Config, InfoDict, PlaylistItems};
use std::future::Future;
use std::time::Duration;

use super::MAX_PLAYLIST_SIZE;
use super::search::{PAGE_RATE_LIMIT_MS, log_tag};

/// Entries resolved at once when `Config::playlist_concurrency` is unset.
/// 1 is what every surveyed downloader does (yt-dlp resolves playlist
/// entries sequentially; its `-N` is fragment-scoped); the in-tree xhamster
/// extractor ran 4 (`CONCURRENT_EXTRACTIONS`), which is why this is a
/// tunable and not a constant.
pub const DEFAULT_PLAYLIST_CONCURRENCY: usize = 1;

/// Per-entry budget when `Config::playlist_item_timeout` is unset — the
/// value the in-tree xhamster playlist path used (`VIDEO_EXTRACTION_TIMEOUT`).
pub const DEFAULT_PLAYLIST_ITEM_TIMEOUT_SECS: u64 = 30;

/// One listed playlist entry: the URL to resolve plus whatever cheap
/// identity the listing page already carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    /// The entry's page URL — what `resolve_entry` is handed.
    pub url: String,
    /// The site's id for the entry, when the listing exposes it.
    pub id: Option<String>,
    /// The entry's title, when the listing exposes it.
    pub title: Option<String>,
}

/// One listed playlist page. The playlist-level fields are read from the
/// FIRST page only; later pages may leave them `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistPage {
    /// The entries on this page, in listing order.
    pub entries: Vec<PlaylistEntry>,
    /// Whether the site reports (or implies) a further page exists.
    pub has_more: bool,
    /// The playlist's id, stamped into every resolved entry's
    /// `playlist_id` (and `playlist`).
    pub playlist_id: Option<String>,
    /// The playlist's title, stamped into every resolved entry's
    /// `playlist_title`.
    pub playlist_title: Option<String>,
    /// The site's own total, when page one carries one. Stamped as
    /// `playlist_count` when present (never below the number actually
    /// listed); without it `playlist_count` is the listed count.
    pub total_estimate: Option<u64>,
}

/// How the loop treats a failed (or timed-out) entry — from
/// `Config::playlist_ignore_errors`.
///
/// Outcomes are collected from the concurrent resolver first and the policy
/// is applied afterwards in listing order, so under `AbortOnFirstFailure`
/// the failure reported is the first BY POSITION, not the first to complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistResolution {
    /// Log a warning and leave the entry out of the result (the default).
    SkipFailed,
    /// Return the failed entry's own error; nothing is returned.
    AbortOnFirstFailure,
}

impl PlaylistResolution {
    /// `None` keeps the skip-and-continue default `Config` documents.
    fn from_config(ignore_errors: Option<bool>) -> Self {
        if ignore_errors.unwrap_or(true) {
            Self::SkipFailed
        } else {
            Self::AbortOnFirstFailure
        }
    }
}

/// The playlist URL and its already-fetched first page — what
/// [`PagedPlaylist::extract_all_entries_from`] starts from. Grouped so a
/// caller that has fetched page one for its own reasons (the plugin host
/// reads the playlist metadata off it) hands it over instead of the loop
/// fetching it twice.
#[derive(Debug)]
pub struct PlaylistStart<'a> {
    /// The playlist URL, passed through to every page fetch and used in
    /// error and log output.
    pub url: &'a str,
    /// Page `first_page_index()`, already fetched.
    pub first_page: PlaylistPage,
}

/// Which 1-based listing positions the operator asked for, from
/// `Config::{playlist_start, playlist_end, playlist_items}`.
struct PlaylistSelection {
    start: usize,
    end: Option<usize>,
    items: Option<PlaylistItems>,
}

impl PlaylistSelection {
    fn from_config(cfg: &Config, url: &str) -> Result<Self> {
        let items = cfg
            .playlist_items
            .as_deref()
            .map(PlaylistItems::parse)
            .transpose()
            .map_err(|e| RdlpError::extraction(e.to_string(), url))?;
        Ok(Self {
            // `Config::validate` rejects 0 post-load, but a `Config` built by
            // hand never passes through it.
            start: cfg.playlist_start.max(1),
            end: cfg.playlist_end,
            items,
        })
    }

    fn wants(&self, position: usize) -> bool {
        position >= self.start
            && self.end.is_none_or(|e| position <= e)
            && self.items.as_ref().is_none_or(|p| p.contains(position))
    }

    /// The highest position that can possibly be wanted, or `None` when the
    /// selection is open-ended — listing stops once this many are listed.
    fn last_needed(&self) -> Option<usize> {
        match (
            self.end,
            self.items.as_ref().and_then(PlaylistItems::max_index),
        ) {
            (Some(e), Some(m)) => Some(e.min(m)),
            (Some(e), None) => Some(e),
            (None, m) => m,
        }
    }
}

/// Whether the listing loop fetches another page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    Continue,
    Stop,
}

/// Everything the listing phase accumulates: the entries in listing order
/// plus the playlist-level fields from the first page.
struct Listed {
    entries: Vec<PlaylistEntry>,
    playlist_id: Option<String>,
    playlist_title: Option<String>,
    total_estimate: Option<u64>,
}

impl Listed {
    fn start(first_page: PlaylistPage, last_needed: Option<usize>) -> (Self, Listing) {
        let PlaylistPage {
            entries,
            has_more,
            playlist_id,
            playlist_title,
            total_estimate,
        } = first_page;
        let mut listed = Self {
            entries: Vec::new(),
            playlist_id,
            playlist_title,
            total_estimate,
        };
        let next = listed.extend(entries, has_more, last_needed);
        (listed, next)
    }

    /// The `playlist_count` to stamp: the site's own total when page one
    /// reported one, else the number of entries listed. Never below the
    /// listed count — a site total smaller than what was actually listed is
    /// wrong by construction.
    fn count(&self) -> usize {
        let listed = self.entries.len();
        self.total_estimate
            .and_then(|t| usize::try_from(t).ok())
            .map_or(listed, |t| t.max(listed))
    }

    /// Append a page's entries and decide whether another page is needed:
    /// an empty page, `!has_more`, reaching `last_needed`, or reaching
    /// `MAX_PLAYLIST_SIZE` all end the listing.
    fn absorb(&mut self, page: PlaylistPage, last_needed: Option<usize>) -> Listing {
        self.extend(page.entries, page.has_more, last_needed)
    }

    fn extend(
        &mut self,
        entries: Vec<PlaylistEntry>,
        has_more: bool,
        last_needed: Option<usize>,
    ) -> Listing {
        if entries.is_empty() {
            return Listing::Stop;
        }
        self.entries.extend(entries);
        if self.entries.len() >= MAX_PLAYLIST_SIZE {
            self.entries.truncate(MAX_PLAYLIST_SIZE);
            return Listing::Stop;
        }
        if last_needed.is_some_and(|n| self.entries.len() >= n) || !has_more {
            return Listing::Stop;
        }
        Listing::Continue
    }
}

/// What one entry's resolution produced, before the failure policy is
/// applied. `InfoDict` is boxed so the enum is not sized by its one large
/// variant (`clippy::large_enum_variant`).
enum ItemOutcome {
    Resolved(Box<InfoDict>),
    Failed(RdlpError),
    TimedOut,
}

/// One entry's listing position, the entry, and how resolving it went.
struct Resolved {
    position: usize,
    entry: PlaylistEntry,
    outcome: ItemOutcome,
}

/// The resolution phase's inputs: which entries, how many at once, and how
/// long each may take.
struct ResolvePlan {
    selected: Vec<(usize, PlaylistEntry)>,
    concurrency: usize,
    timeout: Duration,
}

impl ResolvePlan {
    fn new(selected: Vec<(usize, PlaylistEntry)>, cfg: &Config) -> Self {
        Self {
            selected,
            // `Config::validate` rejects 0 post-load, but a hand-built `Config`
            // never runs it, and `buffer_unordered(0)` polls an empty queue
            // and stays `Pending` forever.
            concurrency: cfg
                .playlist_concurrency
                .unwrap_or(DEFAULT_PLAYLIST_CONCURRENCY)
                .max(1),
            timeout: Duration::from_secs(
                cfg.playlist_item_timeout
                    .unwrap_or(DEFAULT_PLAYLIST_ITEM_TIMEOUT_SECS),
            ),
        }
    }
}

/// The playlist-level fields stamped onto every resolved entry.
struct Stamp {
    playlist_id: Option<String>,
    playlist_title: Option<String>,
    /// The site's reported total (`PlaylistPage::total_estimate` on page
    /// one) when it gave one, else the number of entries LISTED (yt-dlp
    /// `n_entries`) — never the number resolved or selected. Under a range
    /// the listing stops at the last needed page, so without a site total
    /// this is a lower bound on the playlist's true size, not the size.
    count: usize,
}

impl Stamp {
    fn apply(&self, info: &mut InfoDict, position: usize) {
        info.playlist = self.playlist_id.clone();
        info.playlist_id = self.playlist_id.clone();
        info.playlist_title = self.playlist_title.clone();
        info.playlist_index = Some(position);
        info.playlist_count = Some(self.count);
    }
}

/// A paginated playlist: one required per-page listing hook, one required
/// per-entry resolver, and the shared host-owned loop.
///
/// `name` feeds the log tag (see `log_tag`); the two `extract_all_entries*`
/// methods are the shared scaffold and should not be overridden.
pub trait PagedPlaylist: Send + Sync {
    /// Display name for log tags.
    fn name(&self) -> &str;

    /// List ONE page of entries. REQUIRED.
    ///
    /// # Errors
    ///
    /// Returns an error on any fetch/parse failure for `page`. As with
    /// `PagedSearch::search_all_pages`, the loop distinguishes only *when*
    /// it happens: an `Err` on the first page propagates to the caller
    /// (nothing listed yet); an `Err` on a later page ends the listing and
    /// the entries listed so far are resolved.
    fn fetch_playlist_page(
        &self,
        url: &str,
        page: u32,
        ctx: &ExtractionContext,
    ) -> impl Future<Output = Result<PlaylistPage>> + Send;

    /// Resolve ONE listed entry to its full `InfoDict`. REQUIRED. The loop
    /// stamps the playlist fields afterwards; implementors leave them alone.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be extracted. Whether that
    /// error skips the entry or aborts the playlist is the loop's decision
    /// (`Config::playlist_ignore_errors`), not the implementor's.
    fn resolve_entry(
        &self,
        entry: &PlaylistEntry,
        ctx: &ExtractionContext,
    ) -> impl Future<Output = Result<InfoDict>> + Send;

    /// The page number the listing starts at (0 or 1 per site).
    fn first_page_index(&self) -> u32 {
        1
    }

    /// Delay before each page fetch after the first. Defaults to
    /// [`PAGE_RATE_LIMIT_MS`]. Never applied after the last page.
    fn page_rate_limit(&self) -> Duration {
        Duration::from_millis(PAGE_RATE_LIMIT_MS)
    }

    /// Validate `Config::{playlist_start,playlist_end,playlist_items}` for
    /// `url` without fetching anything — the same fail-fast-before-any-page
    /// check [`extract_all_entries`](Self::extract_all_entries) makes
    /// before its own first-page fetch. A caller that must probe a page
    /// before it even knows whether `url` is this site's playlist at all
    /// (the plugin host: an absent `extract-playlist` export or
    /// `unsupported-url` on page one falls back to a single `extract`
    /// call) calls this first, so a malformed range is reported the same
    /// way on both paths — before that probe, not silently skipped by it.
    /// Provided; do not override.
    ///
    /// # Errors
    ///
    /// An unparsable `Config::playlist_items` (start > end, or any other
    /// shape [`rdlp_types::PlaylistItems::parse`] rejects) is an
    /// `Extraction` error naming `url`.
    fn validate_selection(&self, url: &str, ctx: &ExtractionContext) -> Result<()> {
        PlaylistSelection::from_config(&ctx.config, url)?;
        Ok(())
    }

    /// Fetch the first page, then run the shared loop
    /// ([`extract_all_entries_from`](Self::extract_all_entries_from)).
    /// Shared scaffold — do not override.
    ///
    /// # Errors
    ///
    /// An unparsable `Config::playlist_items` is an `Extraction` error
    /// before the first page is fetched ([`validate_selection`](Self::validate_selection)
    /// fails fast, and `extract_all_entries_from` — also an entry point in
    /// its own right — validates again). A first-page failure propagates
    /// as-is; see `extract_all_entries_from` for the loop's own errors.
    fn extract_all_entries(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> impl Future<Output = Result<Vec<InfoDict>>> + Send {
        async move {
            self.validate_selection(url, ctx)?;
            let first_page = self
                .fetch_playlist_page(url, self.first_page_index(), ctx)
                .await?;
            self.extract_all_entries_from(PlaylistStart { url, first_page }, ctx)
                .await
        }
    }

    /// The shared loop, starting from an already-fetched first page: list
    /// further pages only as far as the configured range needs, resolve the
    /// selected positions under `Config::playlist_concurrency` and
    /// `Config::playlist_item_timeout`, apply `Config::playlist_ignore_errors`
    /// in listing order, and stamp the playlist fields. Results come back in
    /// listing order. Shared scaffold — do not override.
    ///
    /// Cancel-safe: each entry's resolve future is independent, nothing is
    /// held across an `.await`, and dropping the returned future drops every
    /// in-flight resolve with it.
    ///
    /// # Errors
    ///
    /// An unparsable `Config::playlist_items` is an `Extraction` error before
    /// any further page is fetched (the caller has already fetched page
    /// one). Under `AbortOnFirstFailure`, the first failed entry BY LISTING
    /// POSITION returns its own error (or an `Extraction` error naming the
    /// entry, for a timeout); the cost of that contract is that every
    /// selected entry is resolved before the error comes back, since a
    /// by-position verdict needs every outcome. A later-page listing failure
    /// is not an error: the entries listed so far are resolved.
    fn extract_all_entries_from(
        &self,
        start: PlaylistStart<'_>,
        ctx: &ExtractionContext,
    ) -> impl Future<Output = Result<Vec<InfoDict>>> + Send {
        async move {
            let tag = log_tag(self.name());
            let PlaylistStart { url, first_page } = start;
            let cfg = &ctx.config;
            let selection = PlaylistSelection::from_config(cfg, url)?;
            let last_needed = selection.last_needed();

            let mut last_page = self.first_page_index();
            let (mut listed, mut next) = Listed::start(first_page, last_needed);
            while next == Listing::Continue {
                tokio::time::sleep(self.page_rate_limit()).await;
                last_page += 1;
                next = match self.fetch_playlist_page(url, last_page, ctx).await {
                    Ok(p) => listed.absorb(p, last_needed),
                    Err(e) => {
                        debug!(page = last_page; "{tag} Playlist page failed, resolving the entries listed so far: {e}");
                        Listing::Stop
                    }
                };
            }
            let total = listed.count();
            let n_listed = listed.entries.len();
            let stamp = Stamp {
                playlist_id: listed.playlist_id,
                playlist_title: listed.playlist_title,
                count: total,
            };
            let selected: Vec<(usize, PlaylistEntry)> = listed
                .entries
                .into_iter()
                .enumerate()
                .map(|(i, e)| (i + 1, e))
                .filter(|(position, _)| selection.wants(*position))
                .collect();
            debug!(listed = n_listed, selected = selected.len(), last_page; "{tag} Playlist listed");
            if selected.is_empty() && n_listed > 0 {
                debug!(listed = n_listed; "{tag} Requested playlist range selects nothing from the listed entries");
            }

            let plan = ResolvePlan::new(selected, cfg);
            let policy = PlaylistResolution::from_config(cfg.playlist_ignore_errors);
            let timeout = plan.timeout;
            let outcomes = resolve_all(self, plan, ctx).await;

            let mut out = Vec::with_capacity(outcomes.len());
            for Resolved {
                position,
                entry,
                outcome,
            } in outcomes
            {
                match (outcome, policy) {
                    (ItemOutcome::Resolved(mut info), _) => {
                        stamp.apply(&mut info, position);
                        out.push(*info);
                    }
                    (ItemOutcome::Failed(e), PlaylistResolution::SkipFailed) => {
                        warn!(position, total; "{tag} Playlist item failed, skipping ({}): {e}", RedactedUrl::new(&entry.url));
                    }
                    (ItemOutcome::Failed(e), PlaylistResolution::AbortOnFirstFailure) => {
                        return Err(e);
                    }
                    (ItemOutcome::TimedOut, PlaylistResolution::SkipFailed) => {
                        warn!(position, total; "{tag} Playlist item timed out after {}s, skipping ({})", timeout.as_secs(), RedactedUrl::new(&entry.url));
                    }
                    (ItemOutcome::TimedOut, PlaylistResolution::AbortOnFirstFailure) => {
                        return Err(RdlpError::extraction(
                            format!(
                                "playlist item {position} timed out after {}s",
                                timeout.as_secs()
                            ),
                            &entry.url,
                        ));
                    }
                }
            }
            Ok(out)
        }
    }
}

/// Resolve every planned entry under the plan's concurrency and per-item
/// timeout, returning the outcomes sorted by listing position.
///
/// `buffer_unordered` (not `buffered`) so a slow entry does not hold back
/// completed ones behind it; listing order is restored by the sort. Each
/// item future owns its entry and touches no shared state, so dropping the
/// stream mid-way cancels cleanly.
async fn resolve_all<P: PagedPlaylist + ?Sized>(
    site: &P,
    plan: ResolvePlan,
    ctx: &ExtractionContext,
) -> Vec<Resolved> {
    let timeout = plan.timeout;
    let mut outcomes: Vec<Resolved> = futures::stream::iter(plan.selected)
        .map(|(position, entry)| async move {
            let outcome = match tokio::time::timeout(timeout, site.resolve_entry(&entry, ctx)).await
            {
                Ok(Ok(info)) => ItemOutcome::Resolved(Box::new(info)),
                Ok(Err(e)) => ItemOutcome::Failed(e),
                Err(_elapsed) => ItemOutcome::TimedOut,
            };
            Resolved {
                position,
                entry,
                outcome,
            }
        })
        .buffer_unordered(plan.concurrency)
        .collect()
        .await;
    outcomes.sort_by_key(|r| r.position);
    outcomes
}

#[cfg(test)]
mod tests {
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
    #[derive(Debug, Clone, Copy, Default)]
    struct Behavior {
        delay: Duration,
        fail: bool,
    }

    fn failing() -> Behavior {
        Behavior {
            delay: Duration::ZERO,
            fail: true,
        }
    }

    fn slow(ms: u64) -> Behavior {
        Behavior {
            delay: Duration::from_millis(ms),
            fail: false,
        }
    }

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
            entry: &PlaylistEntry,
            _ctx: &ExtractionContext,
        ) -> Result<InfoDict> {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            self.resolved_urls
                .lock()
                .expect("test mutex is never poisoned")
                .push(entry.url.clone());
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let _guard = InFlight(&self.in_flight);
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            let behavior = (self.behavior)(entry);
            tokio::time::sleep(behavior.delay).await;
            if behavior.fail {
                return Err(RdlpError::extraction("mock resolve failure", &entry.url));
            }
            let id = entry.id.clone().unwrap_or_default();
            Ok(InfoDict::new(id.clone(), id, "mp4", &entry.url))
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
            assert_eq!(info.playlist.as_deref(), Some("pl"));
            assert_eq!(info.playlist_title.as_deref(), Some("The List"));
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

    #[tokio::test(start_paused = true)]
    async fn per_item_timeout_prunes_the_slow_entry() {
        let slow_v2: BehaviorFn = Box::new(|e| {
            if e.url.ends_with("/v2") {
                slow(1500)
            } else {
                Behavior::default()
            }
        });
        let mock = MockPlaylist::new(scripted(1, 3)).with_behavior(slow_v2);
        let out = run(&mock, cfg(|c| c.playlist_item_timeout = Some(1)))
            .await
            .expect("skip policy prunes the timed-out entry");
        assert_eq!(indices(&out), vec![1, 3]);

        // Under the abort policy the timeout is a failure like any other.
        let err = run(
            &mock,
            cfg(|c| {
                c.playlist_item_timeout = Some(1);
                c.playlist_ignore_errors = Some(false);
            }),
        )
        .await
        .expect_err("abort policy propagates the timeout");
        assert!(err.to_string().contains("timed out"), "got {err}");
        assert_eq!(error_url(&err), "https://x.test/v2");
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_entry_releases_its_concurrency_slot() {
        // Entry #1 is dropped by the 1 s timeout while #2..#4 finish; with
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
}
