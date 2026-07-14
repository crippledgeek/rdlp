// Lint-tightening for LIBRARY code only. `pedantic` / `nursery` are
// stylistic; `indexing_slicing` is enforced here because production code must
// not panic on out-of-bounds. Integration tests under `tests/` deliberately
// use `vec[0]` after a length assertion as the assertion form — see
// `Cargo.toml` `[lints.clippy]` for the rationale.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]
//! rdlp-cli library
//!
//! CLI-specific modules for the rdlp download tool.
//! The core orchestration logic lives in `rdlp-api`; this crate
//! provides the terminal UI layer (inquire callbacks, indicatif
//! progress bars).

#![warn(missing_docs)]

/// CLI event handler mapping API events to indicatif progress bars.
pub mod event_handler;
/// CLI interactive callback using inquire.
pub mod interactive;
/// `rdlp plugin <subcommand>` handlers, exported so integration tests can
/// drive them without going through `clap`'s argument parsing.
#[path = "plugin_cmd.rs"]
pub mod plugin_cmd;
/// Neutralize terminal control sequences in extractor-sourced text before it
/// is written to a TTY or log (#482).
pub mod sanitize;
/// Cross-platform shutdown-signal handling and escalation state machine (#413).
pub mod signal;
