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
///   D-Bus AUTH response, which under `EXTERNAL` is the client UID.
///   (`DBUS_COOKIE_SHA1` would carry a `~/.dbus-keyrings`-derived response,
///   but zbus dropped that mechanism in 5.0 — `AuthMechanism` has only
///   `External` and `Anonymous`, so the UID is the whole exposure here.)
///
/// On the desktop that material was being persisted to a rotating log file, so
/// this filter is an information-disclosure fix as much as a noise fix. All
/// six of zbus's `warn!` sites — including the handshake's own rejection paths
/// — sit above the threshold and still reach the operator.
///
/// The two sinks match this name differently, and only one of them is
/// hierarchy-aware. fern walks `::` segments (`fern-0.7.1` `log_impl.rs`), so
/// on the desktop it covers `zbus::…` and nothing else. `EnvFilter` compares
/// with a plain `str::starts_with` (`tracing-subscriber-0.3.22`
/// `filter/directive.rs`, `filter/env/directive.rs`), so on the CLI it also
/// silences `zbus_names…` and any other `zbus`-prefixed target. That is
/// harmless — `zbus_names 4.3.1` contains no `log` or `tracing` call sites at
/// all — but it is a real difference, so do not assume the desktop's
/// narrowness holds here. Writing `zbus::` to force a boundary would be worse:
/// it would stop matching the bare `zbus` target.
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
/// one: `tracing` stamps a span lifecycle record with this constant, instead
/// of the span's own target, exactly when the span has no fields.
///
/// So the reach is bounded but not zbus-specific: a future *fieldless*
/// `tracing` span in an rdlp crate would be silenced here too. Today none is
/// at risk — all seven rdlp `#[instrument]` sites (`rdlp-api`'s orchestrator)
/// carry `fields(...)`, so their creation records keep the `rdlp_api::…`
/// target and pass this filter untouched. Check that still holds before
/// adding a fieldless span you expect to read at INFO.
///
/// The field-conditional rule covers span creation and `record_all`. A span's
/// *close* record takes this target unconditionally (`Drop for Span`), fields
/// or not — but it is emitted at TRACE and discarded either way. It does not
/// even reach this filter: fern sets `log::max_level` to the maximum of the
/// default level and every override, so the `log!` macro gates a TRACE record
/// against that ceiling first. Had it got through, the `Warn` here would
/// discard it too — fern's per-target lookup replaces the global level rather
/// than combining with it (`find_module(target).unwrap_or(default_level)`).
pub const TRACING_SPAN_LIFECYCLE: &str = "tracing::span";
