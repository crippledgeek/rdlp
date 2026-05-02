//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.

mod audio_sampling_rate;
mod baseurl;
mod errors;
mod expand;
mod frame_rate;
mod segments;

pub(crate) use audio_sampling_rate::parse_audio_sampling_rate;
pub(crate) use errors::DashExpandError;
pub(crate) use expand::expand_dash_representations;
pub(crate) use frame_rate::parse_frame_rate;
