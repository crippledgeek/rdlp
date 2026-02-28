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
