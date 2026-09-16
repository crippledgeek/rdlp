//! # rdlp-crypto
//!
//! Site-agnostic obfuscation toolkit: the small reversible primitives that
//! recur across video-hosting sites' client-side URL/source obfuscation.
//!
//! - [`hash`] — Java-style `String.hashCode` folding hash, and the
//!   `MurmurHash3` fmix32 finaliser re-exported from [`prng`]
//! - [`shuffle`] — seeded Fisher-Yates permutation
//! - [`transposition`] — columnar transposition cipher (forward + inverse)
//! - [`radix`] — base-36 integer encoding
//! - [`homoglyph`] — visually-identical character substitution tables
//! - [`prng`] — the PRNG algorithm variants the above primitives are seeded
//!   from (JS-integer-semantics LCG, xorshift, Weyl-sequence mixers)
//!
//! Site wiring lives in the caller — a plugin links this crate as a library,
//! never through a host import.

#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

pub mod hash;
pub mod homoglyph;
pub mod prng;
pub mod radix;
pub mod shuffle;
pub mod transposition;
pub mod xhamster;

pub use homoglyph::{CYRILLIC_UPPERCASE_TO_LATIN, HomoglyphTable};
pub use prng::ByteGenerator;
pub use xhamster::decipher_format_url;
