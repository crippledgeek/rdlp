//! Bridges a [`LoadedPlugin`] into the `rdlp_core::InfoExtractor` trait so
//! plugins can be registered into the existing extractor registry alongside
//! built-in extractors.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::needless_raw_string_hashes,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::option_if_let_else,
    clippy::single_match_else,
    clippy::needless_pass_by_value
)]

use rdlp_redact::text::sanitize_for_terminal;
use rdlp_redact::{RedactedUrl, RedactedUrlBuf, redact_str};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;

use crate::PluginError;
use crate::bindings::ExtractorPluginHost;
use crate::engine::Engine;
use crate::host::cookie_jar::CookieJarCtx;
use crate::host::fetch::FetchCtx;
use crate::host::html_select::HtmlSelectCtx;
use crate::host::js_eval::JsEvalCtx;
use crate::host::store_kv::StoreKvCtx;
use crate::instance::{PluginStoreData, build_store, deadline_ticks};
use crate::loader::LoadedPlugin;
use crate::manifest::Manifest;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError};
use rdlp_http::wreq;
use rdlp_types::InfoDict;

/// Number of traps before a plugin is automatically disabled for the session.
pub(crate) const TRAP_DISABLE_THRESHOLD: u32 = 3;

/// Wall-clock cap on one `extract` call — the "30 s extract" default the
/// crate doc (`lib.rs`, "Per-call execution") promises. Bounds the
/// per-call epoch deadline AND the host-side `tokio::time::timeout`, so a
/// plugin looping in `host:fetch` (host time, which the epoch never sees)
/// is still stopped.
pub(crate) const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock cap on one `search` / `search-filters` call — the "60 s
/// search" default the crate doc promises. Twice the extract cap because a
/// search page typically fans out to several upstream requests where an
/// extract makes one.
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Wall-clock cap on one `extract-playlist` page call. The search cap,
/// under its own name: a listing page is the same shape of work as a
/// search page (one upstream listing request, possibly a couple more for
/// pagination tokens), and naming it separately lets the two be tuned
/// apart without a hunt for which `SEARCH_TIMEOUT` use was really a
/// playlist's.
pub(crate) const LISTING_TIMEOUT: Duration = SEARCH_TIMEOUT;

/// Longest plugin-authored error `detail` kept, in bytes. The detail lands
/// in every `RdlpError::Extraction` message, warn line, and desktop
/// failure record built from the error; 512 bytes holds a URL plus a
/// sentence of reason (the longest detail a well-behaved plugin has cause
/// to send) while stopping a plugin from pushing a page of text into each
/// of those sinks on every failed call.
pub(crate) const MAX_PLUGIN_ERROR_DETAIL_BYTES: usize = 512;

/// Make a plugin-authored error `detail` safe for every sink it reaches:
/// control characters stripped ([`sanitize_for_terminal`] — CWE-117 log
/// injection, CWE-150 terminal escapes), then credentials redacted
/// ([`redact_str`]), then cut to [`MAX_PLUGIN_ERROR_DETAIL_BYTES`] on a
/// char boundary. Strip BEFORE redacting: a CR or TAB inside `user:pw@`
/// breaks the userinfo pattern's match, so the other order leaks the
/// credential in the clear (measured, `sanitize-before-redact-order`).
/// Cut LAST so the cap can never split a `*:*@` replacement.
///
/// The one site every `detail` crosses — [`common_plugin_error`] and the
/// early-return arms of the per-export mappers all call it — so a new WIT
/// error case gets the same treatment by construction.
pub(crate) fn plugin_detail(detail: &str) -> String {
    let mut out = redact_str(&sanitize_for_terminal(detail));
    if out.len() > MAX_PLUGIN_ERROR_DETAIL_BYTES {
        let cut = out
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= MAX_PLUGIN_ERROR_DETAIL_BYTES)
            .last()
            .unwrap_or(0);
        out.truncate(cut);
    }
    out
}

/// Whether a [`PluginError::Timeout`] from a call counts against the
/// 3-strike rule. Every other strike kind (`Trapped`, `Internal`,
/// `LinkerWire`) always counts; only the timeout is gated, because it is
/// the one fault a slow upstream — not the plugin — can produce many times
/// at once.
pub(crate) enum TimeoutStrikes<'a> {
    /// Every timeout strikes: a single `extract`, a search call, a
    /// playlist page fetch.
    Always,
    /// At most one timeout strikes per gate: the first timed-out call
    /// claims it, later ones under the same gate return their `Timeout`
    /// unchanged but are not counted. A playlist batch resolves its
    /// entries under one gate (`PluginPlaylistSource`), so a dead
    /// upstream during a 16-wide batch is one strike, not sixteen — and a
    /// fresh batch has a fresh gate, so a plugin that keeps timing out
    /// across batches still strikes out.
    OncePer(&'a AtomicBool),
}

impl TimeoutStrikes<'_> {
    /// Whether THIS timeout counts. Claims the gate atomically, so two
    /// entries timing out in the same instant cannot both count.
    fn claim(&self) -> bool {
        match self {
            Self::Always => true,
            Self::OncePer(gate) => !gate.swap(true, Ordering::AcqRel),
        }
    }
}

/// Per-call parameters for [`PluginExtractor::run_in_fresh_store`]: the
/// wall-clock cap, what the call was about (for the strike log line), and
/// how a timeout is counted.
pub(crate) struct CallSpec<'a> {
    /// What the call was handling, named in the strike log line: the URL
    /// for `extract`, the search-site name for search calls. Rendered
    /// through [`RedactedUrl`] regardless, since only `extract` can prove
    /// the value carries no credentials.
    pub subject_for_errors: &'a str,
    /// Wall-clock cap for the whole call, instantiation included.
    pub timeout: Duration,
    /// Whether a timeout of this call strikes (see [`TimeoutStrikes`]).
    pub timeout_strikes: TimeoutStrikes<'a>,
}

impl CallSpec<'_> {
    /// Whether `e` counts against the 3-strike rule for this call:
    /// [`counts_as_strike`]'s verdict, with a `Timeout` additionally
    /// subject to `timeout_strikes`.
    fn strikes(&self, e: &PluginError) -> bool {
        match e {
            PluginError::Timeout { .. } => self.timeout_strikes.claim(),
            other => counts_as_strike(other),
        }
    }
}

/// A freshly instantiated component, handed to the per-call closure.
///
/// `host` is the generated `extractor-plugin-host` bindings (the frozen
/// 0.5.0 exports); `raw` is the underlying instance for exports the host
/// world deliberately omits and resolves by name instead — see
/// `search_adapter::call_search_filters` and `wit/COMPATIBILITY.md` §3.
pub(crate) struct FreshInstance {
    /// The wasmtime instance, for by-name export lookup.
    pub raw: wasmtime::component::Instance,
    /// Typed bindings over the same instance.
    pub host: ExtractorPluginHost,
}

/// The future a per-call closure returns to the runner: borrows the store
/// and instance for exactly the call's duration.
pub(crate) type CallFuture<'s, T> =
    Pin<Box<dyn Future<Output = Result<T, PluginError>> + Send + 's>>;

/// Shared host resources cloned into each plugin invocation's
/// capability contexts. Built once at bootstrap; populated only for the
/// capabilities the host supplies.
#[derive(Clone, Default)]
pub struct HostResources {
    /// Shared HTTPS client used by the `host:fetch` capability when granted.
    pub fetch_client: Option<wreq::Client>,
    /// Shared cookie jar scoped to each plugin's match patterns.
    pub cookie_jar: Option<Arc<rdlp_cookies::SimpleCookieJar>>,
    /// Sled DB used to namespace the `host:store-kv` capability per plugin.
    pub kv_db: Option<Arc<sled::Db>>,
    /// Test-only fixture map for `host:fetch`. When set, requests
    /// matching a fixture URL bypass the network and return the canned
    /// response. Production hosts leave this `None`. See
    /// [`crate::host::fetch_fixtures`].
    pub fetch_fixtures: crate::host::fetch_fixtures::SharedFixtures,
}

/// Adapter wrapping a loaded WASM plugin to look like a built-in extractor.
///
/// Thread-safe — `Arc<Engine>` is `Send + Sync`, atomics are lock-free, and
/// `wasmtime::component::Component` is `Send + Sync`. A fresh `Store` is
/// constructed per call so no mutable state leaks between invocations.
pub struct PluginExtractor {
    /// Shared wasmtime engine.
    pub engine: Arc<Engine>,
    /// Plugin manifest (name, capabilities, priority, …).
    pub manifest: Manifest,
    /// Pre-compiled component, ready for instantiation.
    pub component: wasmtime::component::Component,
    /// Pre-compiled URL-match regex (from manifest or permissive fallback).
    pub valid_url_regex: Regex,
    /// Pre-built linker with the declared capability imports wired once.
    /// Cloned per invocation rather than rebuilt — `Linker` is cheap to clone.
    linker: wasmtime::component::Linker<PluginStoreData>,
    /// Shared host resources used to populate capability contexts per call.
    host_resources: HostResources,
    /// Running count of trap / timeout / internal errors for the 3-strike rule.
    trap_count: AtomicU32,
    /// Set to `true` after `TRAP_DISABLE_THRESHOLD` traps.
    disabled: AtomicBool,
    /// Whether the component declares an `extract-playlist` export, read
    /// off its type once at load. A component cannot gain or lose an
    /// export between calls, so `extract_playlist` consults this instead
    /// of instantiating a store to find out — otherwise every
    /// non-playlist URL through a pre-0.5.2 plugin would pay one extra
    /// instantiation for the probe.
    pub(crate) has_extract_playlist: bool,
}

impl PluginExtractor {
    /// Build an adapter from a [`LoadedPlugin`].
    ///
    /// The URL-match regex is compiled from `manifest.url_regex` with
    /// hardened size/DFA/wall-clock limits (see [`crate::dispatch::compile_url_regex`]).
    /// When `url_regex` is absent the adapter falls back to a permissive
    /// `^https?://` pattern so the registry can still route URLs via
    /// `manifest.matches` patterns elsewhere.
    ///
    /// `host_resources` carries the shared HTTP client, cookie jar, and sled
    /// DB the host has chosen to expose. Capabilities the plugin requested
    /// but for which no resource is supplied are silently denied at runtime
    /// — host policy decides what to share.
    pub fn new(
        loaded: LoadedPlugin,
        engine: Arc<Engine>,
        host_resources: HostResources,
    ) -> Result<Self, PluginError> {
        let valid_url_regex = match &loaded.manifest.url_regex {
            Some(src) => crate::dispatch::compile_url_regex(&loaded.manifest.name, src)?,
            // Static literal — safe to unwrap.
            None => Regex::new(r"^https?://").expect("static regex is always valid"),
        };
        // Build the linker once with this plugin's declared capability set.
        // Cloning a linker per call is cheap; rebuilding it (and re-running
        // each capability's bindgen-generated `add_to_linker`) is not.
        let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw());
        crate::host::add_capability_imports(&mut linker, &loaded.manifest).map_err(|e| {
            PluginError::LinkerWire {
                plugin: loaded.manifest.name.clone(),
                reason: format!("{e}"),
            }
        })?;
        let has_extract_playlist = loaded
            .component
            .component_type()
            .get_export(
                engine.raw(),
                crate::playlist_adapter::EXTRACT_PLAYLIST_EXPORT,
            )
            .is_some();
        Ok(Self {
            engine,
            manifest: loaded.manifest,
            component: loaded.component,
            valid_url_regex,
            linker,
            host_resources,
            trap_count: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
            has_extract_playlist,
        })
    }

    /// Test-only accessors for the trap counter / disabled flag. Integration
    /// tests in this crate's `tests/` dir exercise `record_trap` directly and
    /// read the counter after real calls; the unit tests in this file drive
    /// the regular path through the committed 0.5.0 fixture component.
    /// Hidden from rustdoc so consumers don't accidentally rely on them.
    #[doc(hidden)]
    pub fn test_record_trap(&self) {
        self.record_trap();
    }

    #[doc(hidden)]
    #[must_use]
    pub fn test_is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn test_trap_count(&self) -> u32 {
        self.trap_count.load(Ordering::Relaxed)
    }

    /// Record a runtime fault. Disables the plugin after `TRAP_DISABLE_THRESHOLD`
    /// cumulative traps so a misbehaving plugin cannot spin forever.
    fn record_trap(&self) {
        let count = self.trap_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= TRAP_DISABLE_THRESHOLD {
            log::error!(
                "plugin {} hit {TRAP_DISABLE_THRESHOLD}-strike trap rule; \
                 disabled for this session. Run `rdlp plugin disable {}` \
                 to make the ban permanent.",
                self.manifest.name,
                self.manifest.name,
            );
            self.disabled.store(true, Ordering::Relaxed);
        }
    }

    /// Returns the plugin's priority as declared in the manifest.
    ///
    /// Plugin priorities are constrained to `100..=199` at load time, placing
    /// them above built-in extractors (priority 0) but below any hypothetical
    /// site-specific override (200+).
    #[must_use]
    pub fn plugin_priority(&self) -> i32 {
        self.manifest.priority as i32
    }
}

#[async_trait]
impl InfoExtractor for PluginExtractor {
    /// Display-only: `%(extractor)s`, log tags, list/UI rendering.
    /// `self.manifest.name` (identity, dispatch, the trust store, the
    /// archive token) is deliberately untouched by `display_name`.
    fn name(&self) -> &str {
        self.manifest.display_name()
    }

    fn valid_url(&self) -> &Regex {
        &self.valid_url_regex
    }

    /// Honour the manifest's `matches` patterns at dispatch time.
    ///
    /// The default trait implementation calls `valid_url().is_match(url)`,
    /// but for plugins without an explicit `url_regex` the adapter's
    /// fallback regex is the permissive `^https?://` — which would make
    /// every plugin claim every URL and shadow the Generic extractor.
    /// Delegating to [`crate::dispatch::claims_url`] uses the manifest's
    /// declared Chrome-style match patterns as the authoritative source
    /// of truth, so plugins only claim URLs they were configured for.
    /// See `claims_url` for the godresource regression that motivated
    /// this override.
    fn suitable(&self, url: &str) -> bool {
        crate::dispatch::claims_url(&self.manifest, url)
    }

    fn priority(&self) -> i32 {
        self.plugin_priority()
    }

    fn is_plugin(&self) -> bool {
        true
    }

    /// Plugin-aware priority that clamps to BUILT_IN_MAX (99) when a
    /// built-in extractor also matches this URL — unless the plugin's
    /// signed manifest explicitly lists this URL's host in
    /// `claims_override`.
    fn effective_priority(&self, url: &str, builtin_competitor: bool) -> i32 {
        let parsed = url::Url::parse(url).ok();
        // No competing built-in: no clamp.
        if !builtin_competitor {
            return self.plugin_priority();
        }
        let p = match parsed {
            Some(u) => crate::priority::effective_priority(&self.manifest, &u, true, None),
            None => self.manifest.priority.min(crate::priority::BUILT_IN_MAX),
        };
        p as i32
    }

    /// One `extract` call under [`PluginExtractor::extract_spec`] — the
    /// crate's default extract budget (`EXTRACT_TIMEOUT`, 30 s), every
    /// timeout a strike. See `PluginExtractor::extract_within` for the
    /// call itself; the playlist loop uses that entry point with its own
    /// per-item spec.
    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> rdlp_core::Result<InfoDict> {
        self.extract_within(ctx, Self::extract_spec(url)).await
    }

    /// Drives `extract-playlist` through [`crate::playlist_adapter`]'s
    /// `PagedPlaylist` scaffold, falling back to the trait default (one
    /// `extract` call) when the export is absent, the operator turned
    /// playlist extraction off, or the plugin declines the URL. See
    /// `PluginExtractor::extract_playlist_via_plugin` for the
    /// probe/fallback design.
    async fn extract_playlist(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> rdlp_core::Result<Vec<InfoDict>> {
        self.extract_playlist_via_plugin(url, ctx).await
    }
}

impl PluginExtractor {
    /// The spec a standalone `extract` of `url` runs under: the crate's
    /// default budget and every timeout a strike.
    pub(crate) const fn extract_spec(url: &str) -> CallSpec<'_> {
        CallSpec {
            subject_for_errors: url,
            timeout: EXTRACT_TIMEOUT,
            timeout_strikes: TimeoutStrikes::Always,
        }
    }

    /// One `extract` call of `spec.subject_for_errors` (the URL) under
    /// `spec.timeout` — the runner's tokio timeout and epoch deadline both.
    /// [`InfoExtractor::extract`] passes [`PluginExtractor::extract_spec`];
    /// the playlist loop (`playlist_adapter::PluginPlaylistSource::resolve_entry`)
    /// passes `Config::playlist_item_timeout` under the batch's shared
    /// timeout gate, so an entry has ONE timer and a slow one is a
    /// `PluginError::Timeout` — a strike at most once per batch.
    ///
    /// # Errors
    ///
    /// The runner's errors and the plugin's own, mapped through
    /// [`plugin_error_to_rdlp`] with the URL as the subject.
    pub(crate) async fn extract_within(
        &self,
        ctx: &ExtractionContext,
        spec: CallSpec<'_>,
    ) -> rdlp_core::Result<InfoDict> {
        let url = spec.subject_for_errors;
        // An owned copy moves into the future: the runner's closure is
        // higher-ranked over the store borrow, so it cannot return a future
        // that also borrows `url` from this frame.
        let owned_url = url.to_string();
        // The caps ride on the store data rather than as a fourth
        // `call_plugin_extract` parameter: only `extract` has a `Config`
        // to derive them from, so the runner (shared with search and
        // playlist calls) leaves the store at `Default` and this closure
        // sets them before the export runs.
        let metadata_caps = crate::metadata_adapter::MetadataCaps::from(&*ctx.config);
        self.run_in_fresh_store(spec, move |store, inst| {
            store.data_mut().metadata_caps = metadata_caps;
            Box::pin(async move { call_plugin_extract(store, inst, &owned_url).await })
        })
        .await
        .map_err(|e| plugin_error_to_rdlp(e, Some(url)))
    }

    /// Run one plugin call in a fresh store: refuse if disabled, build the
    /// store with the capability contexts, instantiate, run `f` under
    /// `spec.timeout`, and apply the 3-strike accounting to the outcome.
    /// `extract` and every search call go through here so the timeout and
    /// strike policy exist exactly once.
    ///
    /// The tokio timeout is needed in addition to the epoch deadline:
    /// a plugin doing `host:fetch` in a long retry loop never advances
    /// WASM time, so only host time can stop it. On expiry the per-call
    /// cancel token is tripped so host futures racing it via
    /// `run_with_cancel` abort promptly.
    ///
    /// # Errors
    ///
    /// [`PluginError::Disabled`] when the plugin has struck out;
    /// [`PluginError::Timeout`] when `spec.timeout` elapses;
    /// [`PluginError::Trapped`] when instantiation fails; otherwise
    /// whatever `f` returns. Traps, internal and linker errors count as
    /// strikes, a timeout as `spec.timeout_strikes` says — domain outcomes
    /// do not (see `counts_as_strike`, [`TimeoutStrikes`]).
    pub(crate) async fn run_in_fresh_store<T, F>(
        &self,
        spec: CallSpec<'_>,
        f: F,
    ) -> Result<T, PluginError>
    where
        F: for<'s> FnOnce(
                &'s mut wasmtime::Store<PluginStoreData>,
                &'s FreshInstance,
            ) -> CallFuture<'s, T>
            + Send,
    {
        let plugin = self.manifest.name.clone();
        if self.disabled.load(Ordering::Relaxed) {
            return Err(PluginError::Disabled { plugin });
        }

        let cancel = tokio_util::sync::CancellationToken::new();
        let ticks = deadline_ticks(spec.timeout, self.engine.tick_period());
        let mut store = build_store(&self.engine, &plugin, cancel.clone(), ticks);
        // Which budget a call ran under is what a `Timeout` strike needs
        // read next to: the playlist loop hands `extract` a per-item
        // budget that differs from `EXTRACT_TIMEOUT`.
        log::debug!(
            target: &store.data().log_target,
            "calling plugin {plugin} for {} under a {}s budget",
            RedactedUrl::new(spec.subject_for_errors),
            spec.timeout.as_secs()
        );
        // Every call through this runner (extract, search, playlist pages)
        // shares one display identity for the call's lifetime, so it is set
        // once here rather than per-closure the way `metadata_caps` is
        // (that one genuinely varies per `extract` call; this one doesn't).
        store.data_mut().display_name = self.manifest.display_name().to_string();
        self.populate_capability_contexts(store.data_mut())?;

        let call = async {
            let raw = self
                .linker
                .instantiate_async(&mut store, &self.component)
                .await
                .map_err(|e| PluginError::Trapped {
                    plugin: plugin.clone(),
                    reason: format!("instantiate: {e}"),
                })?;
            let host =
                ExtractorPluginHost::new(&mut store, &raw).map_err(|e| PluginError::Trapped {
                    plugin: plugin.clone(),
                    reason: format!("bind host-world exports: {e}"),
                })?;
            let inst = FreshInstance { raw, host };
            f(&mut store, &inst).await
        };
        let result = match tokio::time::timeout(spec.timeout, call).await {
            Ok(inner) => inner,
            Err(_) => {
                cancel.cancel();
                Err(PluginError::Timeout {
                    plugin: plugin.clone(),
                })
            }
        };

        if let Err(e) = &result {
            if spec.strikes(e) {
                log::warn!(
                    "plugin {plugin} strike ({e}) while handling {}",
                    RedactedUrl::new(spec.subject_for_errors)
                );
                self.record_trap();
            } else if matches!(e, PluginError::Timeout { .. }) {
                log::debug!(
                    "plugin {plugin} timed out while handling {} (already \
                     counted once for this batch)",
                    RedactedUrl::new(spec.subject_for_errors)
                );
            }
        }
        result
    }

    /// Populate the per-call capability contexts on the store data based on
    /// the manifest's declared capabilities AND the host resources we have.
    /// A capability declared in the manifest with no matching host resource
    /// is silently denied at the host-impl layer; this matches the design
    /// principle that the host decides what's actually grantable.
    fn populate_capability_contexts(&self, data: &mut PluginStoreData) -> Result<(), PluginError> {
        let caps = &self.manifest.capabilities;

        if caps.iter().any(|c| c == "fetch")
            && let Some(client) = self.host_resources.fetch_client.clone()
        {
            data.fetch = Some(FetchCtx {
                client,
                fixtures: self.host_resources.fetch_fixtures.clone(),
            });
        }
        if caps.iter().any(|c| c == "cookie-jar")
            && let Some(jar) = self.host_resources.cookie_jar.clone()
        {
            data.cookie_jar = Some(CookieJarCtx::new(jar, &self.manifest.matches));
        }
        if caps.iter().any(|c| c == "js-eval") {
            data.js_eval = Some(JsEvalCtx::default());
        }
        if caps.iter().any(|c| c == "html-select") {
            data.html_select = Some(HtmlSelectCtx);
        }
        if caps.iter().any(|c| c == "store-kv")
            && let Some(db) = self.host_resources.kv_db.as_ref()
        {
            data.store_kv = Some(StoreKvCtx::open(db, &self.manifest.name)?);
        }
        // `log` and `claim-all-urls` need no per-call ctx.
        Ok(())
    }
}

/// Convert a `PluginError` into an `RdlpError` for the orchestrator.
/// Domain errors carry the same trapping/non-trapping flag at the call
/// site; this conversion only shapes the user-facing message.
/// `url` is the subject URL of an `extract`; a search call has none.
pub(crate) fn plugin_error_to_rdlp(e: PluginError, url: Option<&str>) -> RdlpError {
    RdlpError::Extraction {
        message: format!("{e:#}"),
        url: url.map(RedactedUrlBuf::from),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Whether an error counts against the 3-strike rule: runtime faults do,
/// domain outcomes the plugin reported on purpose (UnsupportedUrl,
/// NotFound, RateLimited, SearchUnsupported, …) do not. A `Timeout` is a
/// fault here too; the runner additionally gates it per call through
/// [`TimeoutStrikes`] (`CallSpec::strikes`), so a playlist batch counts
/// its timeouts at most once.
pub(crate) const fn counts_as_strike(e: &PluginError) -> bool {
    matches!(
        e,
        PluginError::Trapped { .. }
            | PluginError::Timeout { .. }
            | PluginError::Internal(_)
            | PluginError::LinkerWire { .. }
    )
}

/// The two things that vary between `call_export_by_name`'s callers: which
/// export to look up, and what to pass it. Grouped into one value so the
/// call keeps to three positional parameters alongside `store`/`inst`,
/// rather than growing a fourth.
pub(crate) struct ExportCall<'a, P> {
    /// Name of the world export to look up, as declared in `wit/extractor.wit`.
    pub name: &'a str,
    /// Parameters to pass once the export is found and typechecks.
    pub params: P,
}

/// Look up `call.name` by name on the live instance and call it with
/// `call.params`, returning `Ok(None)` when the component never declared
/// the export. Logs once, at debug, on that absent-export path — callers
/// do not log it again.
///
/// The mechanism `search_adapter::call_search_filters`,
/// `playlist_adapter::call_extract_playlist`, and
/// `metadata_adapter::call_extract_with_metadata` all share: post-0.5.0
/// exports are optional on the frozen `extractor-plugin-host` bindings
/// (wasmtime-wit-bindgen 30 requires every world export at instantiate
/// time, so binding them would break a component that predates the
/// export), so each is resolved on the live instance instead. Each call
/// site attaches its own domain meaning to "absent" — `search_adapter`
/// treats absence as no filters, `playlist_adapter`/`metadata_adapter`
/// hand `None` back to their caller to try a different export in turn —
/// so only the mechanism, not the meaning, lives here.
pub(crate) async fn call_export_by_name<P, R>(
    store: &mut wasmtime::Store<PluginStoreData>,
    inst: &wasmtime::component::Instance,
    call: ExportCall<'_, P>,
) -> Result<Option<R>, PluginError>
where
    P: wasmtime::component::ComponentNamedList + wasmtime::component::Lower + Send + Sync + 'static,
    R: wasmtime::component::ComponentNamedList + wasmtime::component::Lift + Send + Sync + 'static,
{
    let ExportCall {
        name: export,
        params,
    } = call;
    let plugin = store.data().plugin_name.clone();
    let Some(idx) = inst.get_export(&mut *store, None, export) else {
        log::debug!(target: &store.data().log_target, "plugin exports no `{export}`");
        return Ok(None);
    };
    let trapped = |stage: &str, e: wasmtime::Error| PluginError::Trapped {
        plugin: plugin.clone(),
        reason: format!("{stage} {export}: {e}"),
    };
    let func = inst
        .get_typed_func::<P, R>(&mut *store, idx)
        .map_err(|e| trapped("signature of", e))?;
    let out = func
        .call_async(&mut *store, params)
        .await
        .map_err(|e| trapped("call", e))?;
    func.post_return_async(&mut *store)
        .await
        .map_err(|e| trapped("post-return", e))?;
    Ok(Some(out))
}

/// What `call_plugin_extract` does with `call_extract_with_metadata`'s
/// already-obtained result: `None` means "fall through to the typed
/// `extract` path" (export absent); `Some(_)` short-circuits with the
/// converted extraction or the mapped domain error.
///
/// Isolated as its own pure function — no `Store`/`Instance` — so the
/// routing DECISION (short-circuit vs. fall through) is unit-testable
/// without a wasm component that satisfies the full
/// `extractor-plugin-host` world. `call_extract_with_metadata`'s own tests
/// (`metadata_adapter/tests.rs`) already cover the wasm-boundary mechanics
/// this function's input comes from: absent export, wrong-typed export.
fn route_metadata_extraction(
    r: Option<
        Result<
            crate::metadata_adapter::WitExtraction,
            crate::bindings::rdlp::plugin::types::ExtractError,
        >,
    >,
    plugin_name: &str,
    site: &crate::convert::ExtractionSite<'_>,
) -> Option<Result<InfoDict, PluginError>> {
    Some(match r? {
        Ok(extraction) => Ok(crate::convert::info_dict_from_extraction(extraction, site)),
        Err(extract_err) => Err(extract_error_to_plugin_error(plugin_name, extract_err)),
    })
}

/// Call `extract` on an already-instantiated component and convert the
/// result. The plugin name comes from the store data the runner built.
///
/// Tries the 0.5.2 `extract-with-metadata` export first
/// (`metadata_adapter::call_extract_with_metadata`); [`route_metadata_extraction`]
/// decides whether to short-circuit on it or fall back to the frozen 0.5.0
/// `extract` unchanged. The extras caps come from the store data, where
/// `PluginExtractor::extract` put the call's `Config`-derived values.
pub(crate) async fn call_plugin_extract(
    store: &mut wasmtime::Store<PluginStoreData>,
    inst: &FreshInstance,
    url: &str,
) -> Result<InfoDict, PluginError> {
    let plugin_name = store.data().plugin_name.clone();

    let metadata_caps = store.data().metadata_caps;
    let r = crate::metadata_adapter::call_extract_with_metadata(store, &inst.raw, url).await?;
    if let Some(routed) = route_metadata_extraction(
        r,
        &plugin_name,
        &crate::convert::ExtractionSite {
            url,
            origin: store.data().origin(),
            caps: &metadata_caps,
        },
    ) {
        return routed;
    }

    let wit_result = inst
        .host
        .call_extract(&mut *store, url)
        .await
        .map_err(|e| PluginError::Trapped {
            plugin: plugin_name.clone(),
            reason: format!("call_extract: {e}"),
        })?;

    match wit_result {
        Ok(info) => Ok(crate::convert::info_dict_from_wit(
            info,
            url,
            &store.data().origin(),
        )),
        Err(extract_err) => Err(extract_error_to_plugin_error(&plugin_name, extract_err)),
    }
}

/// The error cases `extract-error` and `search-error` share, so both WIT
/// mappers produce the same `PluginError` variants for them.
pub(crate) enum CommonPluginErr {
    /// Upstream rate limit, with the plugin-suggested retry delay in seconds.
    RateLimited(Option<u32>),
    /// Upstream network failure.
    Network(String),
    /// Upstream content did not parse.
    Parse(String),
    /// The plugin observed the host's cancellation.
    Cancelled,
    /// A genuine plugin-internal failure — the only shared case that strikes.
    Internal(String),
}

/// Map a shared WIT error case to its `PluginError` variant, with every
/// plugin-authored `detail` passed through [`plugin_detail`].
pub(crate) fn common_plugin_error(plugin: String, kind: CommonPluginErr) -> PluginError {
    match kind {
        CommonPluginErr::RateLimited(retry_after) => PluginError::RateLimited {
            plugin,
            retry_after,
        },
        CommonPluginErr::Network(detail) => PluginError::ExtractNetwork {
            plugin,
            detail: plugin_detail(&detail),
        },
        CommonPluginErr::Parse(detail) => PluginError::ExtractParse {
            plugin,
            detail: plugin_detail(&detail),
        },
        CommonPluginErr::Cancelled => PluginError::Cancelled { plugin },
        CommonPluginErr::Internal(detail) => {
            PluginError::Internal(format!("plugin {plugin}: {}", plugin_detail(&detail)))
        }
    }
}

/// Map a WIT `ExtractError` variant to a `PluginError`.
///
/// Domain-level errors (UnsupportedUrl, NotFound, RateLimited, AuthRequired)
/// map to dedicated `PluginError` variants — they are NOT `Internal` — so the
/// 3-strike rule in `run_in_fresh_store` does not penalise plugins that
/// legitimately reject a URL. Only `W::Internal(_)` maps to
/// `PluginError::Internal`.
fn extract_error_to_plugin_error(
    plugin: &str,
    err: crate::bindings::rdlp::plugin::types::ExtractError,
) -> PluginError {
    use crate::bindings::rdlp::plugin::types::ExtractError as W;
    let plugin = plugin.to_string();
    let common = match err {
        W::UnsupportedUrl(detail) => {
            return PluginError::UnsupportedUrl {
                plugin,
                detail: plugin_detail(&detail),
            };
        }
        W::NotFound(detail) => {
            return PluginError::NotFound {
                plugin,
                detail: plugin_detail(&detail),
            };
        }
        W::AuthRequired(detail) => {
            return PluginError::AuthRequired {
                plugin,
                detail: plugin_detail(&detail),
            };
        }
        W::RateLimited(retry_after) => CommonPluginErr::RateLimited(retry_after),
        W::Network(detail) => CommonPluginErr::Network(detail),
        W::Parse(detail) => CommonPluginErr::Parse(detail),
        W::Cancelled => CommonPluginErr::Cancelled,
        W::Internal(detail) => CommonPluginErr::Internal(detail),
    };
    common_plugin_error(plugin, common)
}

#[cfg(test)]
mod tests;
