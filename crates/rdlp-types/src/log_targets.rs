//! Third-party log/tracing targets the application filters at its sinks.
//!
//! rdlp installs two unrelated logging backends — `tracing-subscriber` in
//! `rdlp-cli` and `tauri-plugin-log` (fern) in `rdlp-desktop`. The backends
//! cannot converge; the *list of targets worth silencing* can, and does here,
//! so a target added for one sink is not forgotten at the other.
//!
//! Each sink formats these names in its own dialect: an `EnvFilter` directive
//! (`"zbus=warn"`) for `tracing-subscriber`, a `.level_for(..)` call for fern.

/// zbus, the D-Bus client reached through `secret-service` → `rdlp-cookies`.
///
/// Filtered to `Warn` because its connection handshake is instrumented at
/// INFO and carries authentication material, not merely volume:
///
/// - zbus 5.14.0 contains exactly one `info!` call site in the entire crate
///   (`connection/mod.rs`, "Connection lost name"). The 140 INFO lines
///   observed in a single 287-line session came from `#[instrument]`, whose
///   default level is INFO, on `write_commands`/`read_commands`
///   (`connection/handshake/common.rs`).
/// - `write_commands` records its arguments as span fields, and
///   `Command::Auth(Option<AuthMechanism>, Option<Vec<u8>>)` carries the
///   D-Bus AUTH response — the client UID under `EXTERNAL`, the
///   `~/.dbus-keyrings` cookie-derived response under `DBUS_COOKIE_SHA1`.
///
/// On the desktop that material was being persisted to a rotating log file, so
/// this filter is an information-disclosure fix as much as a noise fix. All
/// six of zbus's `warn!` sites — including the handshake's own rejection paths
/// — sit above the threshold and still reach the operator.
///
/// Both sinks match on the `::` module hierarchy rather than a raw string
/// prefix, so this name covers `zbus::…` without also catching `zbus_names`.
pub const ZBUS: &str = "zbus";

/// `tracing`'s fixed target for span *lifecycle* records crossing the `log`
/// bridge (`LIFECYCLE_LOG_TARGET`, tracing 0.1.44 `src/span.rs`).
///
/// Needed only by a sink that receives `tracing` events as `log` records —
/// i.e. the desktop, where no `tracing` subscriber is installed and the
/// workspace's `tracing/log` feature bridges them. `rdlp-cli` installs a
/// subscriber, so zbus's spans arrive there as real spans under their own
/// `zbus::…` target and [`ZBUS`] alone covers them.
///
/// This target is deliberately not crate-specific and there is no narrower
/// one: `tracing` stamps every *fieldless* span lifecycle record with this
/// constant regardless of the span's own target. A future `tracing` span in an
/// rdlp crate would therefore be silenced here too. That is acceptable only
/// while no rdlp crate emits spans worth reading at INFO — verify before
/// relying on it, rather than assuming the filter stays zbus-specific.
pub const TRACING_SPAN_LIFECYCLE: &str = "tracing::span";
