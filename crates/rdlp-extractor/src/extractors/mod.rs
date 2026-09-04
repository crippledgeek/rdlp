//! Site-specific extractor implementations
//!
//! Each submodule implements a single extractor (or extractor family) for a
//! particular website. Re-exports provide convenient access to the top-level
//! extractor types.

pub mod abxxx;
pub mod eporner;
pub mod generic;
pub mod hqporner;
pub mod koreanpornmovie;
pub mod nine_anime;
pub mod pornhub;
pub mod pornoxo;
pub mod redtube;
pub mod spankbang;
pub mod tnaflix;
pub mod xhamster;
pub mod xnxx;
pub mod xtits;
pub mod xvideos;

pub use abxxx::AbxxxExtractor;
pub use eporner::EPornerExtractor;
pub use generic::GenericExtractor;
pub use hqporner::HQPornerExtractor;
pub use koreanpornmovie::KoreanPornMovieExtractor;
pub use nine_anime::NineAnimeExtractor;
pub use pornhub::PornHubExtractor;
pub use pornoxo::PornoxoExtractor;
pub use redtube::RedTubeExtractor;
pub use spankbang::SpankBangExtractor;
pub use tnaflix::{
    EMPFlixSearchExtractor, MovieFapSearchExtractor, TNAFlixExtractor, TNAFlixSearchExtractor,
};
pub use xhamster::XHamsterExtractor;
pub use xnxx::XNXXExtractor;
pub use xtits::XTitsExtractor;
pub use xvideos::XVideosExtractor;
