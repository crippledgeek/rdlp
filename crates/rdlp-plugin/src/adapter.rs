//! Bridges a [`LoadedPlugin`] into the `rdlp_core::InfoExtractor` trait so
//! plugins can be registered into the existing extractor registry alongside
//! built-in extractors.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;

use crate::PluginError;
use crate::engine::Engine;
use crate::instance::{PluginStoreData, build_store, deadline_ticks};
use crate::loader::LoadedPlugin;
use crate::manifest::Manifest;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError};
use rdlp_types::{DownloadProtocol, InfoDict};

/// Number of traps before a plugin is automatically disabled for the session.
const TRAP_DISABLE_THRESHOLD: u32 = 3;

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
    pub fn new(loaded: LoadedPlugin, engine: Arc<Engine>) -> Result<Self, PluginError> {
        let valid_url_regex = match &loaded.manifest.url_regex {
            Some(src) => crate::dispatch::compile_url_regex(&loaded.manifest.name, src)?,
            // Static literal — safe to unwrap.
            None => Regex::new(r"^https?://").expect("static regex is always valid"),
        };
        Ok(Self {
            engine,
            manifest: loaded.manifest,
            component: loaded.component,
            valid_url_regex,
            trap_count: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
            extract_timeout: Duration::from_secs(30),
        })
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

    fn priority(&self) -> i32 {
        self.plugin_priority()
    }

    async fn extract(&self, url: &str, _ctx: &ExtractionContext) -> rdlp_core::Result<InfoDict> {
        if self.disabled.load(Ordering::Relaxed) {
            return Err(RdlpError::Extraction {
                message: format!(
                    "plugin {} is disabled (3-strike trap rule)",
                    self.manifest.name
                ),
                url: Some(url.to_string()),
            });
        }

        let cancel = tokio_util::sync::CancellationToken::new();
        let ticks = deadline_ticks(self.extract_timeout, Duration::from_millis(100));
        let mut store = build_store(&self.engine, &self.manifest.name, cancel.clone(), ticks);
        let mut linker = wasmtime::component::Linker::<PluginStoreData>::new(self.engine.raw());
        crate::host::add_capability_imports(&mut linker, &self.manifest).map_err(|e| {
            RdlpError::Extraction {
                message: format!(
                    "plugin {} capability wiring failed: {e}",
                    self.manifest.name
                ),
                url: Some(url.to_string()),
            }
        })?;

        let result = call_plugin_extract(
            &mut store,
            &linker,
            &self.component,
            url,
            &self.manifest.name,
        )
        .await;

        match result {
            Ok(info) => Ok(info),
            Err(e) => {
                // Count traps / timeouts / internal errors against the 3-strike rule.
                // Domain-level extraction errors (UnsupportedUrl, NotFound, …) are
                // normal and should not penalise the plugin.
                if matches!(
                    e,
                    PluginError::Trapped { .. }
                        | PluginError::Timeout { .. }
                        | PluginError::Internal(_)
                ) {
                    self.record_trap();
                }
                Err(RdlpError::Extraction {
                    message: format!("{e:#}"),
                    url: Some(url.to_string()),
                })
            }
        }
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
    let inst = crate::bindings::ExtractorPlugin::instantiate_async(&mut *store, component, linker)
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
        Ok(info) => Ok(convert_info_dict(info, url, plugin_name)),
        Err(extract_err) => Err(extract_error_to_plugin_error(plugin_name, extract_err)),
    }
}

/// Map a WIT `ExtractError` variant to a `PluginError`.
fn extract_error_to_plugin_error(
    plugin: &str,
    err: crate::bindings::rdlp::plugin::types::ExtractError,
) -> PluginError {
    use crate::bindings::rdlp::plugin::types::ExtractError as W;
    let reason = match err {
        W::UnsupportedUrl(s) => format!("unsupported url: {s}"),
        W::NotFound(s) => format!("not found: {s}"),
        W::RateLimited(retry) => match retry {
            Some(secs) => format!("rate limited (retry after {secs}s)"),
            None => "rate limited".to_string(),
        },
        W::AuthRequired(s) => format!("auth required: {s}"),
        W::Network(s) => format!("network error: {s}"),
        W::Parse(s) => format!("parse error: {s}"),
        W::Cancelled => "cancelled".to_string(),
        W::Internal(s) => format!("internal: {s}"),
    };
    PluginError::Internal(format!("plugin {plugin} extract error: {reason}"))
}

/// Convert a bindgen-generated `InfoDict` to the rdlp-types `InfoDict`.
///
/// The WIT `InfoDict` does not carry `extractor` or `webpage_url` — those are
/// filled in from the call context (`plugin_name` and `url`).
fn convert_info_dict(
    w: crate::bindings::rdlp::plugin::types::InfoDict,
    url: &str,
    plugin_name: &str,
) -> InfoDict {
    let mut out = InfoDict::new(
        w.id,
        w.title,
        plugin_name,
        // Prefer the URL the plugin returned; fall back to the request URL.
        w.url.as_deref().unwrap_or(url),
    );
    out.thumbnail = w.thumbnail;
    out.description = w.description;
    out.uploader = w.uploader;
    out.uploader_id = w.uploader_id;
    out.upload_date = w.upload_date;
    // WIT duration is Option<u32> (whole seconds); rdlp-types uses Option<f64>.
    out.duration = w.duration.map(f64::from);
    out.view_count = w.view_count;
    out.like_count = w.like_count;
    out.tags = if w.tags.is_empty() {
        None
    } else {
        Some(w.tags)
    };
    out.categories = if w.categories.is_empty() {
        None
    } else {
        Some(w.categories)
    };
    out.formats = w.formats.into_iter().map(convert_format).collect();
    // Convert subtitle list → InfoDict's `HashMap<lang, Vec<Subtitle>>` format.
    if !w.subtitles.is_empty() {
        use rdlp_types::info_dict::Subtitle;
        use std::collections::HashMap;
        let mut map: HashMap<String, Vec<Subtitle>> = HashMap::new();
        for s in w.subtitles {
            map.entry(s.language).or_default().push(Subtitle {
                url: s.url,
                ext: s.ext,
                name: None,
            });
        }
        out.subtitles = Some(map);
    }
    out
}

/// Convert a WIT `Format` to `rdlp_types::Format`.
///
/// Numeric widening: WIT uses `f32` for `fps`/`tbr`/`vbr`/`abr`; rdlp-types
/// uses `f64`. All other field shapes match directly.
fn convert_format(w: crate::bindings::rdlp::plugin::types::Format) -> rdlp_types::Format {
    let protocol = w
        .protocol
        .parse::<DownloadProtocol>()
        .unwrap_or(DownloadProtocol::Https);
    let mut f = rdlp_types::Format::new(w.format_id, w.url, w.ext, protocol);
    f.width = w.width;
    f.height = w.height;
    f.fps = w.fps.map(f64::from);
    f.tbr = w.tbr.map(f64::from);
    f.vbr = w.vbr.map(f64::from);
    f.abr = w.abr.map(f64::from);
    f.vcodec = w.vcodec;
    f.acodec = w.acodec;
    f.container = w.container;
    f.filesize = w.filesize;
    f.format_note = w.format_note;
    f
}
