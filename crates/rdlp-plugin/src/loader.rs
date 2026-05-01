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

        // Step 2: read WASM bytes
        #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
        let wasm = std::fs::read(&wasm_path)?;

        // Step 3: verify signature
        crate::signature::verify(&manifest, &wasm)?;

        // Step 4: compute identity
        let identity = manifest.signature.identity_string();

        // Step 5: trust-store checks
        let requested: BTreeSet<String> = manifest.capabilities.iter().cloned().collect();
        self.check_trust(&manifest, &identity, &requested)?;

        // Step 6: compile component
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
