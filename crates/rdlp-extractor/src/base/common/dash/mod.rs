//! DASH MPD parsing → per-Representation Format emission.
//!
//! See `docs/superpowers/specs/2026-05-02-dash-per-representation-formats-design.md`.
//!
//! # Dead-code allowance
//!
//! The expansion API (`expand_dash_representations`, segment builders, etc.) is
//! fully implemented but not yet wired into any extractor.  Wiring happens in
//! Task 13 (Generic extractor) and Task 14 (end-to-end test).  The allow
//! below is scoped to this module subtree; remove it once Task 13 lands.
#![allow(dead_code, unused_imports)]

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
