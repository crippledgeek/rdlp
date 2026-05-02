//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.
//!
mod audio_sampling_rate;
mod baseurl;
pub mod errors;
mod expand;
mod frame_rate;
mod segments;

pub use errors::DashExpandError;
pub use expand::expand_dash_representations;
