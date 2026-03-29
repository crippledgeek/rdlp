//! # rdlp-types
//!
//! Pure domain types for rdlp (Rust Download Program).
//!
//! This crate contains the core data structures with zero I/O dependencies,
//! making it ideal for compile-time optimization and clear separation of concerns.
//!
//! ## Types
//!
//! - [`InfoDict`] - Central metadata structure for videos
//! - [`Format`] - Video/audio format information
//! - [`Config`] - Application configuration
//! - [`FormatSelector`] - Format selection DSL
//!
//! ## Design Philosophy
//!
//! This crate intentionally has minimal dependencies:
//! - `serde` for serialization
//! - `url` for URL parsing
//! - `regex` for pattern matching
//!
//! No async runtime, HTTP client, or I/O operations are included.

#![warn(missing_docs)]

pub mod audio_format;
pub mod browser_type;
pub mod config;
pub mod container;
pub mod fixup_policy;
pub mod format;
pub mod info_dict;
pub mod parse_error;
pub mod postprocess;
pub mod protocol;
pub mod recode_audio_mode;
pub mod search;
pub mod subtitle_format;
pub mod subtitle_kind;
pub mod subtitle_selection;
pub mod subtitle_track;

// Re-export main types
pub use audio_format::AudioFormat;
pub use browser_type::BrowserType;
pub use config::{Config, ConfigValidationError};
pub use container::ContainerFormat;
pub use fixup_policy::FixupPolicy;
pub use format::{
    Format, FormatSelectError, FormatSelector, FormatSorter, Fragment, format_select,
};
pub use info_dict::{Chapter, InfoDict, Subtitle, Thumbnail};
pub use parse_error::ParseEnumError;
pub use postprocess::PostProcess;
pub use protocol::DownloadProtocol;
pub use recode_audio_mode::RecodeAudioMode;
pub use search::{
    SearchFilter, SearchFilterDescriptor, SearchFilterValue, SearchPageResponse, SearchQuery,
    SearchResultPreview, SearchSiteInfo,
};
pub use subtitle_format::SubtitleFormat;
pub use subtitle_kind::SubtitleKind;
pub use subtitle_selection::{select_subtitles, subtitle_filename};
pub use subtitle_track::{
    SubtitleDiagnostic, SubtitleReason, SubtitleResult, SubtitleStatus, SubtitleTrack,
    normalize_from_info_dict,
};
