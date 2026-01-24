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

pub mod config;
pub mod format;
pub mod info_dict;

// Re-export main types
pub use config::Config;
pub use format::{Format, FormatSelector, Fragment};
pub use info_dict::{Chapter, InfoDict, Subtitle, Thumbnail};
