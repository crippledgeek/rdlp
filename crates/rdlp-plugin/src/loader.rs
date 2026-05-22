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
/// Must match the `package rdlp:plugin@X.Y.Z` directive in
/// `crates/rdlp-plugin/wit/*.wit` (extractor.wit / host.wit / types.wit).
/// Plugin manifests advertise their target via `Manifest.wit_version`;
/// loading rejects any plugin whose `major.minor` differs from this constant
/// (patch differences are considered backward-compatible within the same
/// minor).
// TODO(#327): derive from WIT file at build time
pub const HOST_WIT_VERSION: &str = "0.4.0";

/// Compare a plugin's declared WIT version against the host's `HOST_WIT_VERSION`.
/// Thin 2-arg wrapper around [`check_wit_version_against`] that bakes the host
/// constant in at the call site so callers cannot drift.
fn check_wit_version(plugin_name: &str, plugin_version: &str) -> Result<(), PluginError> {
    check_wit_version_against(plugin_name, plugin_version, HOST_WIT_VERSION)
}

/// Compare a plugin's declared WIT version against an explicit host version.
/// Returns `Err(PluginError::WitVersionMismatch)` when the major or minor
/// differ, or when either version fails to parse as semver. Patch differences
/// within a matching `major.minor` pair are accepted.
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
    if plugin.major != host.major || plugin.minor != host.minor {
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
    /// Returns one `DiscoverOutcome` per plugin directory found. Directories
    /// missing `plugin.toml` or `plugin.wasm` are silently skipped; only
    /// directories containing both files are processed.
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
        for entry in entries.flatten() {
            let dir = entry.path();
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
        let requested: BTreeSet<String> = manifest.capabilities.iter().cloned().collect();
        self.check_trust(&manifest, &identity, &requested)?;

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
    fn check_trust(
        &mut self,
        manifest: &Manifest,
        identity: &str,
        requested: &BTreeSet<String>,
    ) -> Result<(), PluginError> {
        match self
            .trust_store
            .check_identity_match(&manifest.name, identity)
        {
            IdentityCheck::Match => {
                // Known publisher — check for capability creep.
                if let CapabilityCheck::NewCapabilitiesRequested(new_caps) = self
                    .trust_store
                    .check_capabilities(&manifest.name, requested)
                {
                    let previously_approved: Vec<String> = self
                        .trust_store
                        .lookup(&manifest.name)
                        .map(|e| e.approved_capabilities.iter().cloned().collect())
                        .unwrap_or_default();

                    let resp = self.prompter.confirm(ConfirmRequest::CapabilityCreep {
                        plugin_name: manifest.name.clone(),
                        new_version: manifest.version.clone(),
                        previously_approved,
                        new_capabilities: new_caps.clone(),
                    });

                    match resp {
                        ConfirmResponse::Deny => {
                            return Err(PluginError::CapabilityCreep {
                                plugin: manifest.name.clone(),
                                cap: new_caps.join(", "),
                            });
                        }
                        ConfirmResponse::ApprovePersist => {
                            // Persist the expanded capability set so subsequent
                            // loads of the same version don't prompt again.
                            self.trust_store.record(TrustEntry {
                                name: manifest.name.clone(),
                                identity: identity.to_string(),
                                approved_capabilities: requested.clone(),
                            })?;
                        }
                        ConfirmResponse::ApproveOnce => {
                            // Allow the current load but do NOT update the
                            // trust store — user will be prompted again on the
                            // next startup.
                        }
                    }
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
                // First install — require explicit approval.
                let resp = self.prompter.confirm(ConfirmRequest::FirstInstall {
                    plugin_name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    identity: identity.to_string(),
                    capabilities: manifest.capabilities.clone(),
                    claims_override: manifest.claims_override.clone(),
                });

                match resp {
                    ConfirmResponse::Deny => {
                        return Err(PluginError::Internal(format!(
                            "user declined trust for plugin {}",
                            manifest.name
                        )));
                    }
                    ConfirmResponse::ApprovePersist => {
                        self.trust_store.record(TrustEntry {
                            name: manifest.name.clone(),
                            identity: identity.to_string(),
                            approved_capabilities: requested.clone(),
                        })?;
                    }
                    ConfirmResponse::ApproveOnce => {
                        // Allow this session load but do NOT record in trust
                        // store — user will be prompted again next startup.
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{HOST_WIT_VERSION, check_wit_version, check_wit_version_against};
    use crate::PluginError;

    #[test]
    fn host_constant_matches_current_contract() {
        // Sanity: the host constant is itself a valid semver string.
        let parsed = semver::Version::parse(HOST_WIT_VERSION)
            .expect("HOST_WIT_VERSION must parse as semver");
        // The host constant must match the WIT package directive in the .wit
        // sources; if a future bump moves the WIT contract, this assertion
        // surfaces the drift loudly.
        assert_eq!(
            (parsed.major, parsed.minor, parsed.patch),
            (0, 4, 0),
            "HOST_WIT_VERSION must track `package rdlp:plugin@X.Y.Z` in crates/rdlp-plugin/wit/*.wit"
        );
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

    #[test]
    fn patch_compatible_accepts() {
        check_wit_version_against("p", "0.1.5", "0.1.0")
            .expect("higher patch within same minor must accept");
        check_wit_version_against("p", "0.1.0", "0.1.5")
            .expect("lower patch within same minor must accept");
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
