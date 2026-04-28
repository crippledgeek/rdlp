//! Plugin trust store.
//!
//! Persists publisher identity + approved capability set per plugin name in a
//! TOML file (default location `~/.config/rdlp/plugin-trust.toml`). The store
//! is consulted by the loader to:
//!
//! - Detect identity mismatch when a plugin update arrives signed by a
//!   different publisher (vector A2 mitigation).
//! - Detect capability creep when a plugin update requests capabilities that
//!   were not previously approved (vector A1 mitigation).
//! - Track first-install vs subsequent-install state so the prompt fires only
//!   when needed.

use crate::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One trust-store entry per plugin name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustEntry {
    /// Plugin name (key).
    pub name: String,
    /// Stable identity string (e.g. `sigstore:github:user/repo` or
    /// `ed25519:<8-byte-hex>` from `Signature::identity_string()`).
    pub identity: String,
    /// Capabilities the user approved at first install or last re-confirm.
    pub approved_capabilities: BTreeSet<String>,
}

/// On-disk shape of the trust file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    entries: BTreeMap<String, TrustEntry>,
}

/// In-memory wrapper around the on-disk file.
pub struct TrustStore {
    path: PathBuf,
    file: TrustFile,
}

/// Result of comparing a presented identity against the recorded one.
#[derive(Debug)]
pub enum IdentityCheck {
    /// Plugin name has not been seen before.
    NewName,
    /// Recorded identity matches the presented one.
    Match,
    /// Recorded identity differs — load must be refused (vector A2).
    Mismatch {
        /// Identity already on file.
        recorded: String,
        /// Identity presented by the new install attempt.
        presented: String,
    },
}

/// Result of comparing requested capabilities against the approved set.
#[derive(Debug)]
pub enum CapabilityCheck {
    /// All requested capabilities are within the previously approved set.
    AllApproved,
    /// New capabilities present that need re-confirmation (vector A1).
    NewCapabilitiesRequested(Vec<String>),
}

impl TrustStore {
    /// Open the trust store at `path`, creating an empty in-memory store if
    /// the file does not exist yet.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PluginError> {
        let path = path.into();
        let file: TrustFile = if path.exists() {
            #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
            let s = std::fs::read_to_string(&path)?;
            toml::from_str(&s).map_err(PluginError::Toml)?
        } else {
            TrustFile::default()
        };
        Ok(Self { path, file })
    }

    /// Look up an entry by plugin name.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&TrustEntry> {
        self.file.entries.get(name)
    }

    /// Record (or replace) a trust entry, persisting to disk.
    ///
    /// If `persist()` fails the in-memory state is still mutated. This is
    /// safe because the store is rebuilt from disk on the next `open()` —
    /// any partial in-memory drift is transient and never observed across
    /// process boundaries.
    pub fn record(&mut self, entry: TrustEntry) -> Result<(), PluginError> {
        self.file.entries.insert(entry.name.clone(), entry);
        self.persist()
    }

    /// Forget the entry for `name`, persisting to disk.
    ///
    /// Same in-memory/disk consistency contract as [`Self::record`].
    pub fn forget(&mut self, name: &str) -> Result<(), PluginError> {
        self.file.entries.remove(name);
        self.persist()
    }

    /// Compare a presented identity against the recorded one for `name`.
    #[must_use]
    pub fn check_identity_match(&self, name: &str, presented: &str) -> IdentityCheck {
        match self.file.entries.get(name) {
            None => IdentityCheck::NewName,
            Some(e) if e.identity == presented => IdentityCheck::Match,
            Some(e) => IdentityCheck::Mismatch {
                recorded: e.identity.clone(),
                presented: presented.to_string(),
            },
        }
    }

    /// Compare requested capabilities against the previously approved set for `name`.
    /// For unknown names, every requested capability is treated as new.
    #[must_use]
    pub fn check_capabilities(
        &self,
        name: &str,
        requested: &BTreeSet<String>,
    ) -> CapabilityCheck {
        let Some(entry) = self.file.entries.get(name) else {
            return CapabilityCheck::NewCapabilitiesRequested(
                requested.iter().cloned().collect(),
            );
        };
        let new: Vec<String> = requested
            .difference(&entry.approved_capabilities)
            .cloned()
            .collect();
        if new.is_empty() {
            CapabilityCheck::AllApproved
        } else {
            CapabilityCheck::NewCapabilitiesRequested(new)
        }
    }

    /// Atomically persist the trust file: write to a sibling temp path and
    /// rename over the destination. POSIX `rename` guarantees that readers
    /// either see the old content or the fully-written new content, never a
    /// truncated intermediate. Prevents trust-file corruption on crash or kill.
    fn persist(&self) -> Result<(), PluginError> {
        let s = toml::to_string_pretty(&self.file)
            .map_err(|e| PluginError::Internal(format!("trust store serialize: {e}")))?;
        if let Some(parent) = self.path.parent() {
            #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
            std::fs::create_dir_all(parent)?;
        }

        // Write to <path>.tmp in the same directory, then atomic rename. The
        // tempfile lives next to the destination so the rename stays on the
        // same filesystem (cross-fs rename is not atomic).
        let mut tmp_path = self.path.clone();
        let mut file_name = self.path.file_name().unwrap_or_default().to_os_string();
        file_name.push(".tmp");
        tmp_path.set_file_name(file_name);

        #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
        std::fs::write(&tmp_path, s)?;
        #[allow(clippy::disallowed_methods)] // startup/load-time sync I/O
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}
