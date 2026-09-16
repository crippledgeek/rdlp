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
/// entries sequentially; its `-N` is fragment-scoped); the xhamster
/// extractor ran 4 (`CONCURRENT_EXTRACTIONS`, before slice C0-b moved it
/// out of tree onto this loop), which is why this is a tunable and not a
/// constant.
pub const DEFAULT_PLAYLIST_CONCURRENCY: usize = 1;

/// Per-entry budget when `Config::playlist_item_timeout` is unset — the
/// value the xhamster playlist path ran under (`VIDEO_EXTRACTION_TIMEOUT`,
/// before slice C0-b moved it out of tree onto this loop).
pub const DEFAULT_PLAYLIST_ITEM_TIMEOUT_SECS: u64 = 30;

/// How long past the per-entry budget the loop's own guard waits before
/// giving up on an entry whose implementor has not returned.
///
/// The budget is enforced by the implementor (`ResolveRequest::budget` is
/// the plugin call's tokio timeout and epoch deadline), so its own error
/// is the one that should come back — for a plugin that error is a
/// strike, which the loop's guard is not. Two timers set to the same
/// instant race on the millisecond; the grace puts the guard far enough
/// behind that it only ever fires for an implementor with no timer at all.
/// 5 s covers the implementor's timer firing plus its cleanup (a plugin
/// runner trips its cancel token and drops the store — milliseconds) by
/// three orders of magnitude while still bounding a runaway implementor.
pub const PLAYLIST_ITEM_TIMEOUT_GRACE: Duration = Duration::from_secs(5);

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
    /// `playlist_id` (and `playlist`, when there is no title).
    pub playlist_id: Option<String>,
    /// The playlist's title, stamped into every resolved entry's
    /// `playlist_title` and `playlist`.
    pub playlist_title: Option<String>,
    /// The site's own total, when page one carries one. Stamped as
    /// `playlist_count` when present (never below the number actually
    /// listed); without it `playlist_count` is the listed count.
    pub total_estimate: Option<u64>,
}

/// How the loop treats a failed (or timed-out) entry, and a later page
/// that fails to list — from `Config::playlist_ignore_errors`.
///
/// An entry that runs past its `ResolveRequest::budget` fails with the
/// implementor's own timeout error (for a plugin, `PluginError::Timeout`,
/// which is a strike exactly as it is for a single `extract`); the loop
/// treats that like any other failed entry.
///
/// Outcomes are collected from the concurrent resolver first and the policy
/// is applied afterwards in listing order, so under `AbortOnFirstFailure`
/// the failure reported is the first BY POSITION, not the first to complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistResolution {
    /// Log a warning and leave the entry out of the result; a later page
    /// that fails to list is warned about and the entries listed so far
    /// are resolved (the default).
    SkipFailed,
    /// Return the failed entry's own error, or the failed page's, and
    /// nothing is returned.
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

/// One entry the loop asks an implementor to resolve, with the wall-clock
/// budget the implementor must enforce on its own call — for a plugin,
/// the `extract` call's tokio timeout and epoch deadline. One timer, owned
/// by the implementor: the loop only guards `budget +
/// [`PLAYLIST_ITEM_TIMEOUT_GRACE`]` behind it.
#[derive(Debug, Clone, Copy)]
pub struct ResolveRequest<'a> {
    /// The listed entry to resolve.
    pub entry: &'a PlaylistEntry,
    /// `Config::playlist_item_timeout` (or its default), the cap the
    /// implementor applies to its own work.
    pub budget: Duration,
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
/// variant (`clippy::large_enum_variant`). `TimedOut` is the loop's guard
/// firing — an implementor that ignored its budget; one that honoured it
/// comes back as `Failed` with its own timeout error.
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
/// long each may take (the budget handed to the implementor; the loop's
/// own guard sits `PLAYLIST_ITEM_TIMEOUT_GRACE` behind it).
struct ResolvePlan {
    selected: Vec<(usize, PlaylistEntry)>,
    concurrency: usize,
    budget: Duration,
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
            budget: Duration::from_secs(
                cfg.playlist_item_timeout
                    .unwrap_or(DEFAULT_PLAYLIST_ITEM_TIMEOUT_SECS),
            ),
        }
    }

    /// When the loop stops waiting for an implementor that never enforced
    /// its budget.
    fn guard(&self) -> Duration {
        self.budget.saturating_add(PLAYLIST_ITEM_TIMEOUT_GRACE)
    }
}

/// The playlist-level fields stamped onto every resolved entry.
struct Stamp {
    /// `InfoDict::playlist`: yt-dlp's `playlist = playlist_title or
    /// playlist_id` — the title when the site gave one, else the id.
    playlist: Option<String>,
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
        info.playlist = self.playlist.clone();
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
    /// Returns an error on any fetch/parse failure for `page`. An `Err` on
    /// the first page propagates to the caller under either policy
    /// (nothing listed yet). An `Err` on a later page depends on
    /// [`PlaylistResolution`]: under `SkipFailed` (the default) it ends
    /// the listing and the entries listed so far are resolved; under
    /// `AbortOnFirstFailure` it is returned as the playlist's own error
    /// before anything is resolved.
    fn fetch_playlist_page(
        &self,
        url: &str,
        page: u32,
        ctx: &ExtractionContext,
    ) -> impl Future<Output = Result<PlaylistPage>> + Send;

    /// Resolve ONE listed entry to its full `InfoDict` within
    /// `request.budget`. REQUIRED. The loop stamps the playlist fields
    /// afterwards; implementors leave them alone.
    ///
    /// The budget is the implementor's to enforce — it is the one timer on
    /// the entry (a plugin runs its `extract` under it), so the error that
    /// comes back from a slow entry is the implementor's own. The loop
    /// only guards [`PLAYLIST_ITEM_TIMEOUT_GRACE`] behind it, for an
    /// implementor that ignores the budget altogether.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be extracted, or when it
    /// runs past `request.budget`. Whether that error skips the entry or
    /// aborts the playlist is the loop's decision
    /// (`Config::playlist_ignore_errors`), not the implementor's.
    fn resolve_entry(
        &self,
        request: ResolveRequest<'_>,
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
    /// one). A later page's listing failure is warned about and, under
    /// `AbortOnFirstFailure`, returned as the page's own error before
    /// anything is resolved; under `SkipFailed` the entries listed so far
    /// are resolved. Under `AbortOnFirstFailure`, the first failed entry
    /// BY LISTING POSITION returns its own error (or an `Extraction` error
    /// naming the entry, when the loop's guard fired); the cost of that
    /// contract is that every selected entry is resolved before the error
    /// comes back, since a by-position verdict needs every outcome.
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

            let policy = PlaylistResolution::from_config(cfg.playlist_ignore_errors);
            let mut last_page = self.first_page_index();
            let (mut listed, mut next) = Listed::start(first_page, last_needed);
            while next == Listing::Continue {
                tokio::time::sleep(self.page_rate_limit()).await;
                last_page += 1;
                next = match self.fetch_playlist_page(url, last_page, ctx).await {
                    Ok(p) => listed.absorb(p, last_needed),
                    Err(e) => {
                        let redacted_url = RedactedUrl::new(url);
                        match policy {
                            PlaylistResolution::SkipFailed => {
                                let n = listed.entries.len();
                                warn!(
                                    "{tag} Playlist page {last_page} failed, resolving the \
                                     {n} entries listed so far ({redacted_url}): {e}"
                                );
                                Listing::Stop
                            }
                            PlaylistResolution::AbortOnFirstFailure => {
                                warn!(
                                    "{tag} Playlist page {last_page} failed, aborting \
                                     ({redacted_url}): {e}"
                                );
                                return Err(e);
                            }
                        }
                    }
                };
            }
            let total = listed.count();
            let n_listed = listed.entries.len();
            let stamp = Stamp {
                playlist: listed
                    .playlist_title
                    .clone()
                    .or_else(|| listed.playlist_id.clone()),
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
            let redacted_url = RedactedUrl::new(url);
            debug!(
                "{tag} Playlist listed {n_listed} entries through page {last_page}, {} selected \
                 ({redacted_url})",
                selected.len()
            );
            if selected.is_empty() && n_listed > 0 {
                warn!(
                    "{tag} Requested playlist range selects nothing from the {n_listed} listed \
                     entries ({redacted_url})"
                );
            }

            let plan = ResolvePlan::new(selected, cfg);
            let guard = plan.guard();
            let outcomes = resolve_all(self, plan, ctx).await;

            let mut out = Vec::with_capacity(outcomes.len());
            for Resolved {
                position,
                entry,
                outcome,
            } in outcomes
            {
                let guard_secs = guard.as_secs();
                match (outcome, policy) {
                    (ItemOutcome::Resolved(mut info), _) => {
                        stamp.apply(&mut info, position);
                        out.push(*info);
                    }
                    (ItemOutcome::Failed(e), PlaylistResolution::SkipFailed) => {
                        let redacted_url = RedactedUrl::new(&entry.url);
                        warn!(
                            "{tag} Playlist item {position}/{total} failed, skipping \
                             ({redacted_url}): {e}"
                        );
                    }
                    (ItemOutcome::Failed(e), PlaylistResolution::AbortOnFirstFailure) => {
                        return Err(e);
                    }
                    (ItemOutcome::TimedOut, PlaylistResolution::SkipFailed) => {
                        let redacted_url = RedactedUrl::new(&entry.url);
                        warn!(
                            "{tag} Playlist item {position}/{total} ignored its budget and timed \
                             out after {guard_secs}s, skipping ({redacted_url})"
                        );
                    }
                    (ItemOutcome::TimedOut, PlaylistResolution::AbortOnFirstFailure) => {
                        return Err(RdlpError::extraction(
                            format!(
                                "playlist item {position} ignored its budget and timed out \
                                 after {guard_secs}s"
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

/// Resolve every planned entry under the plan's concurrency, handing each
/// implementor call the per-item budget and guarding
/// `PLAYLIST_ITEM_TIMEOUT_GRACE` behind it, returning the outcomes sorted
/// by listing position.
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
    let budget = plan.budget;
    let guard = plan.guard();
    let mut outcomes: Vec<Resolved> = futures::stream::iter(plan.selected)
        .map(|(position, entry)| async move {
            let request = ResolveRequest {
                entry: &entry,
                budget,
            };
            let outcome = match tokio::time::timeout(guard, site.resolve_entry(request, ctx)).await
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
mod tests;
