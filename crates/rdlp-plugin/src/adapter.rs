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

use rdlp_redact::{RedactedUrl, RedactedUrlBuf};
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
const TRAP_DISABLE_THRESHOLD: u32 = 3;

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

/// Per-call parameters for [`PluginExtractor::run_in_fresh_store`]: the
/// wall-clock cap and what the call was about, for the strike log line.
pub(crate) struct CallSpec<'a> {
    /// What the call was handling, named in the strike log line: the URL
    /// for `extract`, the search-site name for search calls. Rendered
    /// through [`RedactedUrl`] regardless, since only `extract` can prove
    /// the value carries no credentials.
    pub subject_for_errors: &'a str,
    /// Wall-clock cap for the whole call, instantiation included.
    pub timeout: Duration,
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
        Ok(Self {
            engine,
            manifest: loaded.manifest,
            component: loaded.component,
            valid_url_regex,
            linker,
            host_resources,
            trap_count: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
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
    fn name(&self) -> &str {
        &self.manifest.name
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

    async fn extract(&self, url: &str, _ctx: &ExtractionContext) -> rdlp_core::Result<InfoDict> {
        let spec = CallSpec {
            subject_for_errors: url,
            timeout: EXTRACT_TIMEOUT,
        };
        // An owned copy moves into the future: the runner's closure is
        // higher-ranked over the store borrow, so it cannot return a future
        // that also borrows `url` from this frame.
        let owned_url = url.to_string();
        self.run_in_fresh_store(spec, move |store, inst| {
            Box::pin(async move { call_plugin_extract(store, inst, &owned_url).await })
        })
        .await
        .map_err(|e| plugin_error_to_rdlp(e, Some(url)))
    }
}

impl PluginExtractor {
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
    /// whatever `f` returns. Traps, timeouts, internal and linker errors
    /// count as strikes — domain outcomes do not (see `counts_as_strike`).
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

        if let Err(e) = &result
            && counts_as_strike(e)
        {
            log::warn!(
                "plugin {plugin} strike ({e}) while handling {}",
                RedactedUrl::new(spec.subject_for_errors)
            );
            self.record_trap();
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
/// NotFound, RateLimited, SearchUnsupported, …) do not.
pub(crate) const fn counts_as_strike(e: &PluginError) -> bool {
    matches!(
        e,
        PluginError::Trapped { .. }
            | PluginError::Timeout { .. }
            | PluginError::Internal(_)
            | PluginError::LinkerWire { .. }
    )
}

/// Call `extract` on an already-instantiated component and convert the
/// result. The plugin name comes from the store data the runner built.
pub(crate) async fn call_plugin_extract(
    store: &mut wasmtime::Store<PluginStoreData>,
    inst: &FreshInstance,
    url: &str,
) -> Result<InfoDict, PluginError> {
    let plugin_name = store.data().plugin_name.clone();
    let wit_result = inst
        .host
        .call_extract(&mut *store, url)
        .await
        .map_err(|e| PluginError::Trapped {
            plugin: plugin_name.clone(),
            reason: format!("call_extract: {e}"),
        })?;

    match wit_result {
        Ok(info) => Ok(crate::convert::info_dict_from_wit(info, url, &plugin_name)),
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

/// Map a shared WIT error case to its `PluginError` variant.
pub(crate) fn common_plugin_error(plugin: String, kind: CommonPluginErr) -> PluginError {
    match kind {
        CommonPluginErr::RateLimited(retry_after) => PluginError::RateLimited {
            plugin,
            retry_after,
        },
        CommonPluginErr::Network(detail) => PluginError::ExtractNetwork { plugin, detail },
        CommonPluginErr::Parse(detail) => PluginError::ExtractParse { plugin, detail },
        CommonPluginErr::Cancelled => PluginError::Cancelled { plugin },
        CommonPluginErr::Internal(detail) => {
            PluginError::Internal(format!("plugin {plugin}: {detail}"))
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
        W::UnsupportedUrl(detail) => return PluginError::UnsupportedUrl { plugin, detail },
        W::NotFound(detail) => return PluginError::NotFound { plugin, detail },
        W::AuthRequired(detail) => return PluginError::AuthRequired { plugin, detail },
        W::RateLimited(retry_after) => CommonPluginErr::RateLimited(retry_after),
        W::Network(detail) => CommonPluginErr::Network(detail),
        W::Parse(detail) => CommonPluginErr::Parse(detail),
        W::Cancelled => CommonPluginErr::Cancelled,
        W::Internal(detail) => CommonPluginErr::Internal(detail),
    };
    common_plugin_error(plugin, common)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::engine::EngineConfig;
    use crate::manifest::parse_manifest_str;
    use rdlp_http::HttpClientFactory;
    use std::sync::atomic::AtomicBool;

    /// The committed 0.5.0 example component: WASI-free, no capabilities,
    /// instantiates on the host world (see its README).
    const FIXTURE_0_5_0: &[u8] =
        include_bytes!("../tests/fixtures/example-extractor-0.5.0/plugin.wasm");

    /// The 0.5.0 fixture wrapped in an adapter, on a manifest with no
    /// `search_site` and no `claims_override`.
    pub fn fixture_extractor() -> PluginExtractor {
        fixture_extractor_with_manifest(FIXTURE_MANIFEST)
    }

    /// The 0.5.0 fixture on `toml` — for tests that need a manifest field
    /// (a `search_site`, a `claims_override`) the default fixture lacks.
    pub fn fixture_extractor_with_manifest(toml: &str) -> PluginExtractor {
        let engine = Arc::new(Engine::new(EngineConfig::default()).expect("engine"));
        let component = wasmtime::component::Component::from_binary(engine.raw(), FIXTURE_0_5_0)
            .expect("component");
        let manifest = parse_manifest_str(toml).expect("manifest");
        let identity = manifest.signature.identity_string();
        let loaded = LoadedPlugin {
            manifest,
            identity,
            component,
            origin_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        };
        PluginExtractor::new(loaded, engine, HostResources::default()).expect("adapter")
    }

    /// Placeholder signature: `parse_manifest_str` checks shape only;
    /// verification happens in the loader, which this fixture bypasses.
    pub const FIXTURE_MANIFEST: &str = r#"
name = "example"
version = "0.0.1"
wit_version = "0.5.0"
matches = ["https://example.com/*"]
priority = 150
capabilities = []

[signature]
type = "ed25519"
pubkey = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
signature = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
"#;

    fn spec(timeout: Duration) -> CallSpec<'static> {
        CallSpec {
            subject_for_errors: "https://example.com/video/42",
            timeout,
        }
    }

    fn extraction_ctx() -> ExtractionContext {
        ExtractionContext::new(
            Arc::new(HttpClientFactory::default().build()),
            Arc::new(rdlp_jsinterp::BoaJsEngine::new()),
            Arc::new(rdlp_cookies::SimpleCookieJar::new()),
            Arc::new(rdlp_types::Config::default()),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_counts_an_internal_error_as_a_strike() {
        let ext = fixture_extractor();
        let err = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
                Box::pin(async { Err::<(), _>(PluginError::Internal("boom".into())) })
            })
            .await
            .expect_err("closure error propagates");
        assert!(matches!(err, PluginError::Internal(_)), "got {err:?}");
        assert_eq!(ext.test_trap_count(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_counts_a_trap_as_a_strike() {
        let ext = fixture_extractor();
        let err = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
                Box::pin(async {
                    Err::<(), _>(PluginError::Trapped {
                        plugin: "example".into(),
                        reason: "unreachable".into(),
                    })
                })
            })
            .await
            .expect_err("closure error propagates");
        assert!(matches!(err, PluginError::Trapped { .. }), "got {err:?}");
        assert_eq!(ext.test_trap_count(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_counts_a_linker_wire_error_as_a_strike() {
        let ext = fixture_extractor();
        let err = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
                Box::pin(async {
                    Err::<(), _>(PluginError::LinkerWire {
                        plugin: "example".into(),
                        reason: "missing import".into(),
                    })
                })
            })
            .await
            .expect_err("closure error propagates");
        assert!(matches!(err, PluginError::LinkerWire { .. }), "got {err:?}");
        assert_eq!(ext.test_trap_count(), 1);
    }

    /// Moved here from the former `tests/adapter_trap_disable.rs`, which
    /// returned early whenever its gitignored wasip1 artefact was absent
    /// and so proved nothing on a fresh checkout.
    #[test]
    fn third_trap_disables_plugin() {
        let ext = fixture_extractor();
        assert!(!ext.test_is_disabled());
        assert_eq!(ext.test_trap_count(), 0);
        ext.test_record_trap();
        assert!(!ext.test_is_disabled(), "1 strike does not disable");
        ext.test_record_trap();
        assert!(!ext.test_is_disabled(), "2 strikes does not disable");
        ext.test_record_trap();
        assert!(ext.test_is_disabled(), "3 strikes MUST disable the adapter");
        assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD);
    }

    #[test]
    fn additional_traps_after_disable_are_idempotent() {
        let ext = fixture_extractor();
        for _ in 0..TRAP_DISABLE_THRESHOLD + 2 {
            ext.test_record_trap();
        }
        // The counter keeps climbing; the disabled flag stays latched.
        assert!(ext.test_is_disabled());
        assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD + 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_does_not_count_a_domain_error_as_a_strike() {
        let ext = fixture_extractor();
        let err = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
                Box::pin(async {
                    Err::<(), _>(PluginError::NotFound {
                        plugin: "example".into(),
                        detail: "gone".into(),
                    })
                })
            })
            .await
            .expect_err("closure error propagates");
        assert!(matches!(err, PluginError::NotFound { .. }), "got {err:?}");
        assert_eq!(ext.test_trap_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_hands_the_closure_a_live_instance() {
        let ext = fixture_extractor();
        let title = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |store, inst| {
                Box::pin(async move {
                    let info =
                        call_plugin_extract(store, inst, "https://example.com/video/42").await?;
                    Ok(info.title)
                })
            })
            .await
            .expect("extract via runner");
        assert_eq!(title, "Example Video 42");
        assert_eq!(ext.test_trap_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_timeout_is_a_strike_and_cancels_the_call() {
        let ext = fixture_extractor();
        let seen_cancel = Arc::new(std::sync::Mutex::new(None));
        let sink = seen_cancel.clone();
        let err = ext
            .run_in_fresh_store(spec(Duration::from_millis(50)), move |store, _inst| {
                *sink.lock().expect("mutex") = Some(store.data().cancel.clone());
                Box::pin(std::future::pending::<Result<(), PluginError>>())
            })
            .await
            .expect_err("deadline elapses");
        assert!(matches!(err, PluginError::Timeout { .. }), "got {err:?}");
        assert_eq!(ext.test_trap_count(), 1);
        let token = seen_cancel
            .lock()
            .expect("mutex")
            .clone()
            .expect("closure ran");
        assert!(token.is_cancelled(), "timeout must trip the per-call token");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runner_refuses_a_disabled_plugin_without_running_the_closure() {
        let ext = fixture_extractor();
        for _ in 0..TRAP_DISABLE_THRESHOLD {
            ext.test_record_trap();
        }
        assert!(ext.test_is_disabled());
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let err = ext
            .run_in_fresh_store(spec(EXTRACT_TIMEOUT), move |_store, _inst| {
                flag.store(true, Ordering::Relaxed);
                Box::pin(async { Ok::<(), _>(()) })
            })
            .await
            .expect_err("disabled plugin refuses calls");
        assert!(matches!(err, PluginError::Disabled { .. }), "got {err:?}");
        assert!(!ran.load(Ordering::Relaxed), "closure must not run");
        // Refusal is not itself a strike.
        assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_goes_through_the_runner() {
        let ext = fixture_extractor();
        let info = ext
            .extract("https://example.com/video/42", &extraction_ctx())
            .await
            .expect("extract");
        assert_eq!(info.title, "Example Video 42");

        // A domain error from the plugin is not a strike.
        let err = ext
            .extract("https://example.com/video/abc", &extraction_ctx())
            .await
            .expect_err("non-numeric id is a parse error");
        assert!(err.to_string().contains("parse error"), "got {err}");
        assert_eq!(ext.test_trap_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn extract_on_a_disabled_plugin_reports_disabled() {
        let ext = fixture_extractor();
        for _ in 0..TRAP_DISABLE_THRESHOLD {
            ext.test_record_trap();
        }
        let err = ext
            .extract("https://example.com/video/42", &extraction_ctx())
            .await
            .expect_err("disabled");
        assert!(err.to_string().contains("is disabled"), "got {err}");
    }

    #[test]
    fn extract_error_mapping_keeps_the_plugin_name_on_internal() {
        use crate::bindings::rdlp::plugin::types::ExtractError as W;
        match extract_error_to_plugin_error("example", W::Internal("bad".into())) {
            PluginError::Internal(msg) => assert_eq!(msg, "plugin example: bad"),
            other => panic!("got {other:?}"),
        }
        assert!(matches!(
            extract_error_to_plugin_error("example", W::RateLimited(Some(7))),
            PluginError::RateLimited {
                retry_after: Some(7),
                ..
            }
        ));
        assert!(matches!(
            extract_error_to_plugin_error("example", W::Cancelled),
            PluginError::Cancelled { .. }
        ));
    }
}
