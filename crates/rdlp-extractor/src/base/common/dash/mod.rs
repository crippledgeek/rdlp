//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.

mod baseurl;
mod errors;
mod expand;
mod segments;

pub(crate) use errors::DashExpandError;
pub(crate) use expand::expand_dash_representations;
