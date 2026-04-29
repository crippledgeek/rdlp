//! Plugin system error type.

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur within the rdlp plugin system.
#[derive(Debug, Error)]
pub enum PluginError {
    /// The plugin manifest file could not be parsed or is structurally invalid.
    #[error("manifest at {path} is invalid: {reason}")]
    InvalidManifest {
        /// Filesystem path of the manifest that failed to parse.
        path: PathBuf,
        /// Human-readable description of what is wrong with the manifest.
        reason: String,
    },

    /// Cryptographic signature verification failed.
    #[error("signature verification failed for plugin '{plugin}': {reason}")]
    SignatureInvalid {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Description of the verification failure.
        reason: String,
    },

    /// The plugin binary hash no longer matches the previously trusted identity.
    #[error(
        "plugin '{plugin}' identity mismatch: previously trusted '{old}', now '{new}'. Run `rdlp plugin retrust {plugin}` to accept."
    )]
    IdentityMismatch {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Previously pinned identity (e.g. a content hash or public key fingerprint).
        old: String,
        /// New identity observed at load time.
        new: String,
    },

    /// The plugin requests a capability not previously approved by the user.
    #[error(
        "plugin '{plugin}' requests new capability '{cap}' not previously approved. Re-confirm to update."
    )]
    CapabilityCreep {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Name of the newly requested capability.
        cap: String,
    },

    /// A different plugin is already trusted under this name.
    #[error(
        "plugin name '{plugin}' already trusted under different identity '{existing}'. Run `rdlp plugin uninstall {plugin}` first."
    )]
    NameSquatting {
        /// Plugin name that is being squatted.
        plugin: String,
        /// Identity of the plugin already registered under this name.
        existing: String,
    },

    /// The plugin declared a capability that the host did not grant.
    #[error("plugin '{plugin}' declares capability '{cap}' not granted by host")]
    UndeclaredCapability {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Name of the capability that was declared but not granted.
        cap: String,
    },

    /// The plugin's declared priority is outside the reserved range 100..=199.
    #[error("plugin '{plugin}' priority {got} outside allowed range 100..=199")]
    InvalidPriority {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// The out-of-range priority value that was declared.
        got: u32,
    },

    /// A URL-match regex in the manifest failed to compile.
    #[error("plugin '{plugin}' regex compile failed: {reason}")]
    RegexCompile {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Description of the regex compile error.
        reason: String,
    },

    /// The WIT interface version exported by the plugin is incompatible with the host.
    #[error("plugin '{plugin}' WIT version {got} incompatible with host {host}")]
    WitVersionMismatch {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// WIT version string declared by the plugin component.
        got: String,
        /// WIT version string the host requires.
        host: String,
    },

    /// The plugin call exceeded its allotted execution time.
    #[error("plugin '{plugin}' execution timed out")]
    Timeout {
        /// Plugin name as declared in its manifest.
        plugin: String,
    },

    /// The plugin call was cancelled by the caller.
    #[error("plugin '{plugin}' was cancelled")]
    Cancelled {
        /// Plugin name as declared in its manifest.
        plugin: String,
    },

    /// The WASM instance trapped (e.g. OOB memory, unreachable instruction).
    #[error("plugin '{plugin}' trapped: {reason}")]
    Trapped {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Trap message from the WASM runtime.
        reason: String,
    },

    /// The plugin returned a domain-level "URL not supported" error from `extract`.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' does not support URL: {detail}")]
    UnsupportedUrl {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Plugin-supplied detail (typically the URL it could not handle).
        detail: String,
    },

    /// The plugin returned a domain-level "not found" error from `extract`.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' reported resource not found: {detail}")]
    NotFound {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Plugin-supplied detail (e.g. the missing video id).
        detail: String,
    },

    /// The plugin reported it was rate-limited by the upstream service.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' was rate-limited{}",
        retry_after.map(|s| format!(" (retry after {s}s)")).unwrap_or_default())]
    RateLimited {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Optional plugin-suggested retry delay in seconds.
        retry_after: Option<u32>,
    },

    /// The plugin reported an authentication / login is required upstream.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' reports auth required: {detail}")]
    AuthRequired {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Plugin-supplied detail.
        detail: String,
    },

    /// The plugin reported a network failure during extract.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' network error during extract: {detail}")]
    ExtractNetwork {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Plugin-supplied detail.
        detail: String,
    },

    /// The plugin reported a parse failure on upstream content.
    /// Counted as a normal extraction outcome, NOT as a trap.
    #[error("plugin '{plugin}' parse error during extract: {detail}")]
    ExtractParse {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Plugin-supplied detail.
        detail: String,
    },

    /// Wiring host capability imports into the wasmtime linker failed
    /// (e.g. a bindgen-generated `add_to_linker` returned an error).
    #[error("plugin '{plugin}' linker wiring failed: {reason}")]
    LinkerWire {
        /// Plugin name as declared in its manifest.
        plugin: String,
        /// Underlying wasmtime / bindgen error message.
        reason: String,
    },

    /// The user explicitly declined to trust this plugin during the
    /// first-install confirmation prompt.
    #[error("plugin '{plugin}' load declined by user")]
    UserDeclined {
        /// Plugin name as declared in its manifest.
        plugin: String,
    },

    /// The plugin name failed validation: must be lowercase kebab-case,
    /// no path-separator characters, no leading hyphen.
    #[error("plugin name '{name}' is invalid: {reason}")]
    InvalidPluginName {
        /// The offending plugin name as supplied by caller / manifest.
        name: String,
        /// Why it was rejected.
        reason: String,
    },

    /// The plugin has been disabled (3-strike rule or explicit user action).
    #[error("plugin '{plugin}' is disabled (3-strike rule or user disabled)")]
    Disabled {
        /// Plugin name as declared in its manifest.
        plugin: String,
    },

    /// An I/O error occurred (wrapped from [`std::io::Error`]).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A TOML deserialization error (wrapped from [`toml::de::Error`]).
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),

    /// An internal invariant was violated.
    #[error("internal: {0}")]
    Internal(String),
}
