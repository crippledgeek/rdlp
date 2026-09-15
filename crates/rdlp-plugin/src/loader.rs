//! Plugin loader. Integrates manifest parsing, signature verification, trust
//! store identity-pinning, capability-creep detection, prompt-based user
//! confirmation, and final component compilation.
//!
//! Errors are non-fatal: a plugin that fails to load is reported via the
//! return value but does not block sibling plugins from loading.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::unnecessary_debug_formatting)]

use crate::PluginError;
use crate::engine::Engine;
use crate::manifest::{self, Manifest};
use crate::prompt::{ConfirmRequest, ConfirmResponse, Prompter};
use crate::trust_store::{CapabilityCheck, IdentityCheck, TrustEntry, TrustStore};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// WIT contract version this host accepts.
///
/// Derived at compile time from `wit/types.wit`'s
/// `package rdlp:plugin@X.Y.Z;` directive (#327), so the constant cannot lag
/// the contract. `host_constant_matches_current_contract` proves all three
/// `.wit` files agree.
pub const HOST_WIT_VERSION: &str =
    crate::wit_version::package_version(include_str!("../wit/types.wit"));

/// Compare a plugin's declared WIT version against the host's `HOST_WIT_VERSION`.
/// Thin 2-arg wrapper around [`check_wit_version_against`] that bakes the host
/// constant in at the call site so callers cannot drift.
fn check_wit_version(plugin_name: &str, plugin_version: &str) -> Result<(), PluginError> {
    check_wit_version_against(plugin_name, plugin_version, HOST_WIT_VERSION)
}

/// Compare a plugin's declared WIT version against an explicit host version.
///
/// Accepts when `major.minor` match and `plugin.patch <= host.patch`.
/// Component Model canonical names fold `0.x.y` to `0.x`, so any 0.5.y links;
/// the patch bound exists because a plugin declaring a newer patch may
/// import a host function this host does not yet define — refuse it here
/// with a readable error instead of a linker failure at instantiate.
///
/// Unparseable plugin or host versions are mapped to `WitVersionMismatch`
/// (raw string preserved in `got` / `host`). This is intentional: malformed
/// inputs cannot match any valid host, so mismatch is the correct outcome.
///
/// Visibility is `pub(crate)` for tests only; production code calls the
/// 2-arg [`check_wit_version`] wrapper.
pub(crate) fn check_wit_version_against(
    plugin_name: &str,
    plugin_version: &str,
    host_version: &str,
) -> Result<(), PluginError> {
    let mismatch = || PluginError::WitVersionMismatch {
        plugin: plugin_name.to_string(),
        got: plugin_version.to_string(),
        host: host_version.to_string(),
    };
    let plugin = semver::Version::parse(plugin_version).map_err(|_| mismatch())?;
    let host = semver::Version::parse(host_version).map_err(|_| mismatch())?;
    if plugin.major != host.major || plugin.minor != host.minor || plugin.patch > host.patch {
        return Err(mismatch());
    }
    Ok(())
}

/// A successfully loaded plugin. Contains everything Task 24's `PluginExtractor`
/// needs to wire itself into the rdlp orchestrator.
pub struct LoadedPlugin {
    /// Parsed and validated manifest.
    pub manifest: Manifest,
    /// Compiled wasmtime component, ready for instantiation.
    pub component: wasmtime::component::Component,
    /// Stable identity string (e.g. `ed25519:<hex>` or `sigstore:<oidc>`).
    pub identity: String,
    /// Filesystem directory the plugin was loaded from.
    pub origin_dir: PathBuf,
}

/// One outcome per discovered plugin directory.
pub type DiscoverOutcome = Result<LoadedPlugin, (PathBuf, PluginError)>;

/// Loader handle. Borrows the engine + trust store; owns a clone-able prompter
/// arc.
pub struct Loader<'a> {
    /// The wasmtime engine used for component compilation.
    pub engine: &'a Engine,
    /// Mutable reference to the persistent trust store.
    pub trust_store: &'a mut TrustStore,
    /// User-confirmation interface (interactive, CI, or pre-trusted).
    pub prompter: Arc<dyn Prompter>,
}

impl<'a> Loader<'a> {
    /// Create a new loader with the given engine, trust store, and prompter.
    pub fn new(
        engine: &'a Engine,
        trust_store: &'a mut TrustStore,
        prompter: Arc<dyn Prompter>,
    ) -> Self {
        Self {
            engine,
            trust_store,
            prompter,
        }
    }

    /// Scan `root` for plugin subdirectories and load each. Errors are
    /// per-plugin and do not block siblings.
    ///
    /// Returns one `DiscoverOutcome` per plugin directory found, in path
    /// order — `read_dir` yields entries in filesystem order, which differs
    /// between filesystems, and the registry's "first registered wins"
    /// tie-break downstream must not depend on it. Directories missing
    /// `plugin.toml` or `plugin.wasm` are silently skipped; only directories
    /// containing both files are processed.
    pub fn discover(&mut self, root: &Path) -> Vec<DiscoverOutcome> {
        let mut out = Vec::new();
        #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("plugin dir {root:?}: {e}");
                return out;
            }
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        dirs.sort();
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            // Only process directories that have both required files.
            if !dir.join("plugin.toml").exists() || !dir.join("plugin.wasm").exists() {
                continue;
            }
            match self.load_one(&dir) {
                Ok(plugin) => out.push(Ok(plugin)),
                Err(e) => {
                    log::warn!("plugin {dir:?} failed to load: {e}");
                    out.push(Err((dir, e)));
                }
            }
        }
        out
    }

    fn load_one(&mut self, dir: &Path) -> Result<LoadedPlugin, PluginError> {
        let manifest_path = dir.join("plugin.toml");
        let wasm_path = dir.join("plugin.wasm");

        // Step 1: parse manifest
        let manifest = manifest::parse_manifest_file(&manifest_path)?;

        // Step 2: enforce WIT contract version before any crypto work
        check_wit_version(&manifest.name, &manifest.wit_version)?;

        // Step 3: read WASM bytes
        #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
        let wasm = std::fs::read(&wasm_path)?;

        // Step 4: verify signature
        crate::signature::verify(&manifest, &wasm)?;

        // Step 5: compute identity
        let identity = manifest.signature.identity_string();

        // Step 6: trust-store checks
        self.check_trust(&TrustSubject {
            manifest: &manifest,
            identity: &identity,
        })?;

        // Step 7: compile component
        let component = wasmtime::component::Component::new(self.engine.raw(), &wasm)
            .map_err(|e| PluginError::Internal(format!("component compile: {e}")))?;

        Ok(LoadedPlugin {
            manifest,
            component,
            identity,
            origin_dir: dir.to_path_buf(),
        })
    }

    /// Run the full trust-store / prompt workflow for one plugin. Mutates the
    /// trust store on `ApprovePersist`; session-only on `ApproveOnce`.
    ///
    /// A known publisher is re-confirmed for two kinds of change, each
    /// through its own prompt: capabilities not previously approved
    /// (`CapabilityCreep`), and a search-site claim that differs from the
    /// approved one (`SearchClaimsChange`) — a plugin must not be able to
    /// start shadowing a built-in's search on an update the user never saw.
    ///
    /// The store is written ONCE, after every prompt the load needed has
    /// approved: an entry always describes the whole manifest, so writing
    /// it on the first approval would also record whatever a later prompt
    /// then denied, and the next startup would load that denied claim
    /// without asking. It is persisted only when every prompt answered
    /// `ApprovePersist`; one `ApproveOnce` keeps the whole load session-only.
    fn check_trust(&mut self, subject: &TrustSubject<'_>) -> Result<(), PluginError> {
        let TrustSubject { manifest, identity } = *subject;
        let mut decisions = Decisions::default();
        match self
            .trust_store
            .check_identity_match(&manifest.name, identity)
        {
            IdentityCheck::Match => {
                // Snapshot the approved entry ONCE, before either prompt,
                // so both comparisons judge the update against what was
                // actually approved last time.
                let prior = self.trust_store.lookup(&manifest.name).cloned();
                if let CapabilityCheck::NewCapabilitiesRequested(new_caps) = self
                    .trust_store
                    .check_capabilities(&manifest.name, &subject.requested_capabilities())
                {
                    let previously_approved: Vec<String> = prior
                        .as_ref()
                        .map(|e| e.approved_capabilities.iter().cloned().collect())
                        .unwrap_or_default();
                    let resp = self.prompter.confirm(ConfirmRequest::CapabilityCreep {
                        plugin_name: manifest.name.clone(),
                        new_version: manifest.version.clone(),
                        previously_approved,
                        new_capabilities: new_caps.clone(),
                    });
                    decisions.record(resp, || PluginError::CapabilityCreep {
                        plugin: manifest.name.clone(),
                        cap: new_caps.join(", "),
                    })?;
                }

                let approved_claims = prior.map(|e| e.search).unwrap_or_default();
                let requested_claims = manifest.search_claims();
                if approved_claims != requested_claims {
                    let resp = self.prompter.confirm(ConfirmRequest::SearchClaimsChange {
                        plugin_name: manifest.name.clone(),
                        new_version: manifest.version.clone(),
                        previously_approved: approved_claims,
                        requested: requested_claims.clone(),
                    });
                    decisions.record(resp, || PluginError::SearchClaimsChange {
                        plugin: manifest.name.clone(),
                        detail: format!(
                            "search_site = {:?}, search_claims_override = {:?}",
                            manifest.search_site_name(),
                            requested_claims.search_claims_override
                        ),
                    })?;
                }
            }
            IdentityCheck::Mismatch {
                recorded,
                presented,
            } => {
                return Err(PluginError::IdentityMismatch {
                    plugin: manifest.name.clone(),
                    old: recorded,
                    new: presented,
                });
            }
            IdentityCheck::NewName => {
                let resp = self.prompter.confirm(ConfirmRequest::FirstInstall {
                    plugin_name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    identity: identity.to_string(),
                    capabilities: manifest.capabilities.clone(),
                    claims_override: manifest.claims_override.clone(),
                    search: manifest.search_claims(),
                });
                decisions.record(resp, || {
                    PluginError::Internal(format!(
                        "user declined trust for plugin {}",
                        manifest.name
                    ))
                })?;
            }
        }

        if decisions.persist() {
            self.trust_store.record(subject.entry())?;
        }
        Ok(())
    }
}

/// The prompt answers one load collected, folded into whether the trust
/// store gets written at the end. A `Deny` short-circuits the load at the
/// prompt that produced it; the store is never touched before the fold.
#[derive(Debug, Default)]
struct Decisions {
    prompted: bool,
    every_answer_persists: bool,
}

impl Decisions {
    /// Fold one answer in: `Deny` is `denied()`; `ApproveOnce` makes the
    /// whole load session-only; `ApprovePersist` keeps persistence on the
    /// table.
    fn record(
        &mut self,
        resp: ConfirmResponse,
        denied: impl FnOnce() -> PluginError,
    ) -> Result<(), PluginError> {
        let persists = match resp {
            ConfirmResponse::Deny => return Err(denied()),
            ConfirmResponse::ApprovePersist => true,
            ConfirmResponse::ApproveOnce => false,
        };
        self.every_answer_persists = if self.prompted {
            self.every_answer_persists && persists
        } else {
            persists
        };
        self.prompted = true;
        Ok(())
    }

    /// Whether the entry is written: at least one prompt fired and every
    /// one of them asked to persist.
    const fn persist(&self) -> bool {
        self.prompted && self.every_answer_persists
    }
}

/// What the trust store judges one load by: the manifest being loaded and
/// the identity its signature proved.
struct TrustSubject<'a> {
    manifest: &'a Manifest,
    identity: &'a str,
}

impl TrustSubject<'_> {
    /// The capability set the manifest asks for.
    fn requested_capabilities(&self) -> BTreeSet<String> {
        self.manifest.capabilities.iter().cloned().collect()
    }

    /// The entry an `ApprovePersist` writes: everything this version asks
    /// for, so the same version never prompts again.
    fn entry(&self) -> TrustEntry {
        TrustEntry {
            name: self.manifest.name.clone(),
            identity: self.identity.to_string(),
            approved_capabilities: self.requested_capabilities(),
            search: self.manifest.search_claims(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HOST_WIT_VERSION, check_wit_version, check_wit_version_against};
    use crate::PluginError;

    /// Pull the version out of a `package rdlp:plugin@X.Y.Z;` directive by
    /// scanning lines — an oracle independent of the `const fn` byte walk
    /// in `crate::wit_version::package_version` that produces
    /// `HOST_WIT_VERSION`, so the two can disagree and be caught.
    fn package_version_by_lines(wit_source: &str, file: &str) -> String {
        wit_source
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("package rdlp:plugin@")?
                    .strip_suffix(';')
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| panic!("{file} must declare `package rdlp:plugin@X.Y.Z;`"))
    }

    #[test]
    fn host_constant_matches_current_contract() {
        // Sanity: the host constant is itself a valid semver string.
        semver::Version::parse(HOST_WIT_VERSION).expect("HOST_WIT_VERSION must parse as semver");

        // Read the contract rather than restating it. This assertion used to
        // compare against a hardcoded (0, 4, 0) tuple, which meant it passed
        // whenever someone updated the tuple and the constant together while
        // leaving the WIT package directive behind — the one drift it names
        // itself for catching. Every `.wit` in the contract is checked,
        // because `wit-bindgen` resolves them as one package and a single
        // stale directive breaks the build in a way that reads as unrelated.
        for (file, source) in [
            ("types.wit", include_str!("../wit/types.wit")),
            ("host.wit", include_str!("../wit/host.wit")),
            ("extractor.wit", include_str!("../wit/extractor.wit")),
        ] {
            assert_eq!(
                package_version_by_lines(source, file),
                HOST_WIT_VERSION,
                "HOST_WIT_VERSION must track `package rdlp:plugin@X.Y.Z` in crates/rdlp-plugin/wit/{file}"
            );
        }
    }

    /// The checked-in example manifests declare the version the loader gates
    /// on, so a stale one is a plugin that cannot load.
    ///
    /// This is not hypothetical: all three sat at `0.1.0` from the 0.2.0 bump
    /// until the 0.5.0 one, because nothing compared them against the host and
    /// no test loads a plugin through its template. They are documentation
    /// that had silently stopped being true.
    #[test]
    fn example_manifest_templates_declare_the_host_version() {
        for (file, source) in [
            (
                "examples/plugins/example-extractor/plugin.toml.template",
                include_str!("../../../examples/plugins/example-extractor/plugin.toml.template"),
            ),
            (
                "examples/plugins/example-extractor/plugin.sigstore.toml.template",
                include_str!(
                    "../../../examples/plugins/example-extractor/plugin.sigstore.toml.template"
                ),
            ),
            (
                "examples/plugins/ytdlp-hello-world/plugin.toml.template",
                include_str!("../../../examples/plugins/ytdlp-hello-world/plugin.toml.template"),
            ),
        ] {
            let declared = source
                .lines()
                .find_map(|line| line.trim().strip_prefix("wit_version = "))
                .map_or_else(
                    || panic!("{file} must declare wit_version"),
                    |v| v.trim_matches('"'),
                );
            // A template may lag the host by patch (D1's accepted range) and
            // still load — checked via the production 2-arg wrapper, not a
            // hand-rolled comparison.
            check_wit_version("template", declared).unwrap_or_else(|e| {
                panic!("{file} declares a WIT version the loader would reject: {e}")
            });
            // Templates ship the CURRENT contract, though: this is a stronger
            // claim than "loadable" and catches a template left one release
            // behind even though patch-below would still pass the loader.
            assert_eq!(
                declared, HOST_WIT_VERSION,
                "{file} should declare the current WIT contract version"
            );
        }
    }

    #[test]
    fn wrapper_passes_host_constant_through() {
        // The 2-arg wrapper must call through with HOST_WIT_VERSION, so a
        // plugin declaring exactly that version is accepted.
        check_wit_version("p", HOST_WIT_VERSION)
            .expect("plugin declaring HOST_WIT_VERSION must be accepted by 2-arg wrapper");
    }

    #[test]
    fn matching_version_accepts() {
        check_wit_version_against("p", "0.1.0", "0.1.0").expect("identical version must accept");
    }

    /// D1: same major.minor and `plugin.patch <= host.patch`. The canonical
    /// Component Model name of `0.x.y` is `0.x`, so 0.5.0 and 0.5.1 link;
    /// a plugin that declares a NEWER patch may call an import this host
    /// does not define, so it is refused here with a clear error rather
    /// than at instantiate.
    #[test]
    fn patch_at_or_below_host_accepts() {
        check_wit_version_against("p", "0.5.0", "0.5.1").expect("0.5.0 plugin on 0.5.1 host");
        check_wit_version_against("p", "0.5.1", "0.5.1").expect("0.5.1 plugin on 0.5.1 host");
    }

    #[test]
    fn patch_above_host_rejects() {
        let err = check_wit_version_against("p", "0.5.2", "0.5.1")
            .expect_err("0.5.2 plugin must be refused by a 0.5.1 host");
        assert!(
            matches!(err, PluginError::WitVersionMismatch { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn minor_above_host_rejects() {
        assert!(check_wit_version_against("p", "0.6.0", "0.5.1").is_err());
    }

    #[test]
    fn minor_below_host_rejects() {
        assert!(check_wit_version_against("p", "0.4.9", "0.5.1").is_err());
    }

    #[test]
    fn major_differs_rejects_even_with_same_minor_patch() {
        assert!(check_wit_version_against("p", "1.5.1", "0.5.1").is_err());
    }

    #[test]
    fn host_constant_is_derived_from_types_wit() {
        assert_eq!(HOST_WIT_VERSION, "0.5.1");
        assert_eq!(
            crate::wit_version::package_version(include_str!("../wit/types.wit")),
            "0.5.1"
        );
    }

    #[test]
    fn minor_mismatch_rejects() {
        let err =
            check_wit_version_against("p", "0.2.0", "0.1.0").expect_err("minor bump must reject");
        match err {
            PluginError::WitVersionMismatch { plugin, got, host } => {
                assert_eq!(plugin, "p");
                assert_eq!(got, "0.2.0");
                assert_eq!(host, "0.1.0");
            }
            other => panic!("expected WitVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn major_mismatch_rejects() {
        let err =
            check_wit_version_against("p", "1.0.0", "0.1.0").expect_err("major bump must reject");
        assert!(
            matches!(err, PluginError::WitVersionMismatch { .. }),
            "expected WitVersionMismatch, got {err:?}"
        );
    }

    #[test]
    fn malformed_plugin_version_rejects() {
        let err = check_wit_version_against("p", "not-a-semver", "0.1.0")
            .expect_err("malformed plugin version must reject");
        match err {
            PluginError::WitVersionMismatch { plugin, got, host } => {
                assert_eq!(plugin, "p");
                assert_eq!(got, "not-a-semver");
                assert_eq!(host, "0.1.0");
            }
            other => panic!("expected WitVersionMismatch, got {other:?}"),
        }
    }
}
