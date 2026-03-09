//! Site-specific extractor implementations
//!
//! Each submodule implements a single extractor (or extractor family) for a
//! particular website. Re-exports provide convenient access to the top-level
//! extractor types.

pub mod hqporner;
pub mod nine_anime;
pub mod pornhub;
pub mod redtube;
pub mod tnaflix;
pub mod xhamster;
pub mod xtits;

pub use hqporner::HQPornerExtractor;
pub use nine_anime::NineAnimeExtractor;
pub use pornhub::PornHubExtractor;
pub use redtube::RedTubeExtractor;
pub use tnaflix::{TNAFlixExtractor, TNAFlixSearchExtractor};
pub use xhamster::XHamsterExtractor;
pub use xtits::XTitsExtractor;
