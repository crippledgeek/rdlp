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

use rdlp_redact::RedactedUrlBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;

use crate::PluginError;
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
    /// Wall-clock cap on a single `extract` call.
    extract_timeout: Duration,
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
            extract_timeout: Duration::from_secs(30),
        })
    }

    /// Test-only accessors for the trap counter / disabled flag. Integration
    /// tests in this crate's `tests/` dir exercise `record_trap` directly
    /// (the regular path requires a real component + wasmtime store, which
    /// is too heavy for an invariant test). Hidden from rustdoc so consumers
    /// don't accidentally rely on them.
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
        if self.disabled.load(Ordering::Relaxed) {
            return Err(RdlpError::Extraction {
                message: format!(
                    "plugin {} is disabled (3-strike trap rule)",
                    self.manifest.name
                ),
                url: Some(RedactedUrlBuf::from(url)),
            });
        }

        // A fresh cancel token per call; the tokio timeout below trips it
        // when the wall-clock deadline elapses, so host-side futures that
        // are racing it via `run_with_cancel` (e.g., host:fetch) abort
        // promptly even if the wasmtime epoch hasn't fired yet.
        let cancel = tokio_util::sync::CancellationToken::new();
        let ticks = deadline_ticks(self.extract_timeout, Duration::from_millis(100));
        let mut store = build_store(&self.engine, &self.manifest.name, cancel.clone(), ticks);
        self.populate_capability_contexts(store.data_mut())
            .map_err(|e| RdlpError::Extraction {
                message: format!("{e:#}"),
                url: Some(RedactedUrlBuf::from(url)),
            })?;

        // Wrap the call in a wall-clock timeout. Without it, a CPU-bound
        // plugin with no host calls only stops on the wasmtime epoch trap,
        // and a plugin doing host:fetch in a long retry loop never trips
        // the epoch (host time isn't WASM time).
        let plugin_name = self.manifest.name.clone();
        let component = &self.component;
        let linker = &self.linker;
        let timeout = self.extract_timeout;
        let cancel_for_timeout = cancel.clone();
        let result = match tokio::time::timeout(timeout, async move {
            call_plugin_extract(&mut store, linker, component, url, &plugin_name).await
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => {
                cancel_for_timeout.cancel();
                Err(PluginError::Timeout {
                    plugin: self.manifest.name.clone(),
                })
            }
        };

        match result {
            Ok(info) => Ok(info),
            Err(e) => {
                // Count traps / timeouts / internal errors against the 3-strike
                // rule. Domain-level extraction errors (UnsupportedUrl,
                // NotFound, RateLimited, AuthRequired, …) are surfaced as
                // dedicated typed variants and do NOT penalise the plugin.
                if matches!(
                    e,
                    PluginError::Trapped { .. }
                        | PluginError::Timeout { .. }
                        | PluginError::Internal(_)
                        | PluginError::LinkerWire { .. }
                ) {
                    self.record_trap();
                }
                Err(plugin_error_to_rdlp(e, url))
            }
        }
    }
}

impl PluginExtractor {
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
fn plugin_error_to_rdlp(e: PluginError, url: &str) -> RdlpError {
    RdlpError::Extraction {
        message: format!("{e:#}"),
        url: Some(RedactedUrlBuf::from(url)),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Instantiate the WASM component, call `extract`, and convert the result.
async fn call_plugin_extract(
    store: &mut wasmtime::Store<PluginStoreData>,
    linker: &wasmtime::component::Linker<PluginStoreData>,
    component: &wasmtime::component::Component,
    url: &str,
    plugin_name: &str,
) -> Result<InfoDict, PluginError> {
    let inst =
        crate::bindings::ExtractorPluginHost::instantiate_async(&mut *store, component, linker)
            .await
            .map_err(|e| PluginError::Trapped {
                plugin: plugin_name.to_string(),
                reason: format!("instantiate: {e}"),
            })?;

    let wit_result =
        inst.call_extract(&mut *store, url)
            .await
            .map_err(|e| PluginError::Trapped {
                plugin: plugin_name.to_string(),
                reason: format!("call_extract: {e}"),
            })?;

    match wit_result {
        Ok(info) => Ok(crate::convert::info_dict_from_wit(info, url, plugin_name)),
        Err(extract_err) => Err(extract_error_to_plugin_error(plugin_name, extract_err)),
    }
}

/// Map a WIT `ExtractError` variant to a `PluginError`.
///
/// Domain-level errors (UnsupportedUrl, NotFound, RateLimited, AuthRequired)
/// map to dedicated `PluginError` variants — they are NOT `Internal` — so the
/// 3-strike trap rule in `extract` does not penalise plugins that legitimately
/// reject a URL. Only `W::Internal(_)` (genuine plugin-internal failures) maps
/// to `PluginError::Internal`.
fn extract_error_to_plugin_error(
    plugin: &str,
    err: crate::bindings::rdlp::plugin::types::ExtractError,
) -> PluginError {
    use crate::bindings::rdlp::plugin::types::ExtractError as W;
    let plugin = plugin.to_string();
    match err {
        W::UnsupportedUrl(detail) => PluginError::UnsupportedUrl { plugin, detail },
        W::NotFound(detail) => PluginError::NotFound { plugin, detail },
        W::RateLimited(retry_after) => PluginError::RateLimited {
            plugin,
            retry_after,
        },
        W::AuthRequired(detail) => PluginError::AuthRequired { plugin, detail },
        W::Network(detail) => PluginError::ExtractNetwork { plugin, detail },
        W::Parse(detail) => PluginError::ExtractParse { plugin, detail },
        W::Cancelled => PluginError::Cancelled { plugin },
        W::Internal(detail) => PluginError::Internal(format!("plugin {plugin}: {detail}")),
    }
}
