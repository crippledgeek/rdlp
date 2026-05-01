//! Plugin install / update confirmation prompts.
//!
//! Pluggable via the [`Prompter`] trait. The loader (Task 23) calls
//! `prompter.confirm(request)` whenever a plugin needs first-install approval
//! or a capability-creep re-confirmation.
//!
//! Concrete prompters are provided by:
//! - **CLI** — `inquire`-backed text prompt
//! - **Tauri (desktop)** — Tauri-event-bridge prompter
//! - **CI / non-interactive** — [`AlwaysApprove`], [`AlwaysDeny`], or
//!   [`PreTrustedIdentities`] when the user passes `--trust-publisher <id>`

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::too_long_first_doc_paragraph)]

/// Describes the scenario requiring user confirmation.
#[derive(Debug, Clone)]
pub enum ConfirmRequest {
    /// First time a plugin name is being installed.
    FirstInstall {
        /// Human-readable plugin name.
        plugin_name: String,
        /// Version string from the manifest.
        version: String,
        /// Signing identity (e.g. `sigstore:github:user/foo`).
        identity: String,
        /// Capabilities the plugin declares.
        capabilities: Vec<String>,
        /// Any claims the user has pre-overridden on the command line.
        claims_override: Vec<String>,
    },
    /// Subsequent install of a known plugin requesting MORE capabilities than
    /// previously approved.
    CapabilityCreep {
        /// Plugin name.
        plugin_name: String,
        /// New version being installed.
        new_version: String,
        /// Capabilities approved in the prior install.
        previously_approved: Vec<String>,
        /// Net-new capabilities requested by the new version.
        new_capabilities: Vec<String>,
    },
}

/// The prompter's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmResponse {
    /// Permit the install / update for this session only.
    ///
    /// The capability set (or first-install) is allowed to proceed but is
    /// **not** persisted to the trust store. On the next startup the user
    /// will be prompted again.
    ApproveOnce,
    /// Permit the install / update and persist the approval to the trust store.
    ///
    /// Subsequent loads of the same plugin version + capability set will not
    /// prompt again.
    ApprovePersist,
    /// Reject the install / update.
    Deny,
}

/// User-facing confirmation interface. Implementors handle UI, CI flag mapping,
/// or test recording.
pub trait Prompter: Send + Sync {
    /// Present `request` to the user (or policy engine) and return their decision.
    fn confirm(&self, request: ConfirmRequest) -> ConfirmResponse;
}

/// Always approves and persists. CI use only — should require a
/// `--trust-everything`-style flag in the CLI to opt in (NOT exposed by
/// default per design spec §9).
pub struct AlwaysApprove;

impl Prompter for AlwaysApprove {
    fn confirm(&self, _: ConfirmRequest) -> ConfirmResponse {
        ConfirmResponse::ApprovePersist
    }
}

/// Always denies. Useful as a safe default in non-interactive contexts that
/// haven't pre-approved any publishers.
pub struct AlwaysDeny;

impl Prompter for AlwaysDeny {
    fn confirm(&self, _: ConfirmRequest) -> ConfirmResponse {
        ConfirmResponse::Deny
    }
}

/// Approves a [`ConfirmRequest::FirstInstall`] only if the request's identity
/// is in the pre-trusted list. Always denies [`ConfirmRequest::CapabilityCreep`]
/// because new capabilities should require explicit interactive re-trust, even
/// for previously-approved publishers.
pub struct PreTrustedIdentities {
    /// List of pre-trusted signing identities.
    pub trusted: Vec<String>,
}

impl Prompter for PreTrustedIdentities {
    fn confirm(&self, request: ConfirmRequest) -> ConfirmResponse {
        match request {
            ConfirmRequest::FirstInstall { identity, .. } => {
                if self.trusted.iter().any(|t| t == &identity) {
                    ConfirmResponse::ApprovePersist
                } else {
                    ConfirmResponse::Deny
                }
            }
            ConfirmRequest::CapabilityCreep { .. } => ConfirmResponse::Deny,
        }
    }
}
