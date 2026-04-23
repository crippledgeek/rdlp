//! EPorner extractor (XHR-authenticated primary path + /dload/ DOM-scrape fallback).
//!
//! # Note on stubs
//! `hash` and `search` are currently empty stubs; they are filled in Tasks 4.2 and 4.4.
//! They are declared here so the module compiles cleanly from Task 4.1 onward.

pub mod hash;
pub mod patterns;
pub mod search;

/// EPorner video extractor.
#[derive(Default)]
pub struct EPornerExtractor;

impl EPornerExtractor {
    /// Create a new EPorner extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}
