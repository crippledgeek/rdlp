//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.

mod errors;
mod expand;

pub(crate) use errors::DashExpandError;
pub(crate) use expand::expand_dash_representations;
