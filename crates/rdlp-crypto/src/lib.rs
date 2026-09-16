//! # rdlp-crypto
//!
//! Site-agnostic obfuscation toolkit: the small reversible primitives that
//! recur across video-hosting sites' client-side URL/source obfuscation.
//! Every primitive is public as "a number plus its parameters" — a site
//! wiring a new cipher composes these directly rather than re-deriving them.
//!
//! - [`hash`] — [`hash::java_string_hash32`] (Java-style `String.hashCode`
//!   folding hash); [`hash::fmix32`] re-exported from [`prng`]
//! - [`shuffle`] — [`shuffle::seeded_shuffle`] (seeded Fisher-Yates
//!   permutation, caller supplies the PRNG draw)
//! - [`transposition`] — [`transposition::columnar_transpose`] /
//!   [`transposition::columnar_untranspose`] (columnar transposition cipher)
//! - [`radix`] — [`radix::to_radix`] (arbitrary-base integer encoding,
//!   `2..=36`) and [`radix::Radix`] (the validated base); [`radix::to_base36`]
//!   is `to_radix(n, Radix::BASE36)`
//! - [`homoglyph`] — [`homoglyph::HomoglyphTable`] and the
//!   [`CYRILLIC_UPPERCASE_TO_LATIN`] table (visually-identical character
//!   substitution)
//! - [`js_int`] — [`js_int::to_signed_32`] (the JS `|0` coercion the
//!   i64-arithmetic PRNG steps use)
//! - [`prng`] — the PRNG algorithm variants: [`prng::lcg_step`],
//!   [`prng::weyl_step`], [`prng::xorshift`] (+ [`prng::XorshiftShifts`]),
//!   [`prng::rotate_scramble`] (+ [`prng::Rotation`]), [`prng::fmix32`],
//!   [`prng::pcg_xsh_rs`], [`prng::mxs_mix`], and the composed 7-algorithm
//!   [`ByteGenerator`] façade
//!
//! Site wiring lives in the caller — a plugin links this crate as a library,
//! never through a host import.

#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

pub mod hash;
pub mod homoglyph;
pub mod js_int;
pub mod prng;
pub mod radix;
pub mod shuffle;
pub mod transposition;

pub use homoglyph::{CYRILLIC_UPPERCASE_TO_LATIN, HomoglyphTable};
pub use js_int::to_signed_32;
pub use prng::ByteGenerator;
