//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.

mod errors;
mod expand;

pub use errors::DashExpandError;
pub use expand::expand_dash_representations;
