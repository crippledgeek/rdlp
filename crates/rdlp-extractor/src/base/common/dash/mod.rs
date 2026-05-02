//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.

mod errors;
mod expand;
mod segments;

// Re-exported for callers outside this module (wired up incrementally across Tasks 3–13).
#[allow(unused_imports)]
pub(crate) use errors::DashExpandError;
#[allow(unused_imports)]
pub(crate) use expand::expand_dash_representations;
