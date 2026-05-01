//! Common static CSS selectors and regex patterns
//!
//! Pre-compiled selectors and patterns used across multiple extractors.
//! Initialized once at first use via `std::sync::LazyLock`.

#![allow(dead_code)]

use lazy_regex::{Lazy, Regex, lazy_regex};
use scraper::Selector;
use std::sync::LazyLock;

// ============================================================================
// Security Constants
// ============================================================================

/// Maximum title length for extracted metadata
pub const MAX_TITLE_LENGTH: usize = 500;

/// Maximum description length for extracted metadata
pub const MAX_DESCRIPTION_LENGTH: usize = 10_000;

/// Maximum number of videos in a playlist
pub const MAX_PLAYLIST_SIZE: usize = 1000;

/// Default sample size for debug output
pub const DEFAULT_DEBUG_SAMPLE_SIZE: usize = 5000;

// ============================================================================
// Common Static Selectors
// ============================================================================

/// Selector for Open Graph title: `<meta property="og:title" content="...">`
pub static OG_TITLE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[property="og:title"]"#);

/// Selector for Open Graph description: `<meta property="og:description" content="...">`
pub static OG_DESCRIPTION_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[property="og:description"]"#);

/// Selector for Open Graph image: `<meta property="og:image" content="...">`
pub static OG_IMAGE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[property="og:image"]"#);

/// Selector for meta description: `<meta name="description" content="...">`
pub static META_DESCRIPTION_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[name="description"]"#);

/// Selector for Twitter title: `<meta name="twitter:title" content="...">`
pub static TWITTER_TITLE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[name="twitter:title"]"#);

/// Selector for Twitter image: `<meta name="twitter:image" content="...">`
pub static TWITTER_IMAGE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[name="twitter:image"]"#);

/// Selector for HTML title tag: `<title>...</title>`
pub static TITLE_TAG_SELECTOR: LazyLock<Selector> = crate::static_selector!("title");

/// Selector for H1 heading: `<h1>...</h1>`
pub static H1_SELECTOR: LazyLock<Selector> = crate::static_selector!("h1");

/// Selector for JSON-LD scripts: `<script type="application/ld+json">`
pub static JSONLD_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"script[type="application/ld+json"]"#);

/// Selector for canonical link: `<link rel="canonical" href="...">`
pub static CANONICAL_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"link[rel="canonical"]"#);

// ============================================================================
// Common Static Patterns
// ============================================================================

/// Pattern to extract quality from URL (e.g., "720p", "1080P")
pub static QUALITY_FROM_URL_PATTERN: Lazy<Regex> = lazy_regex!(r"(\d+)[pP]");

/// Pattern to extract bitrate from URL (e.g., "720P_4000K")
pub static BITRATE_FROM_URL_PATTERN: Lazy<Regex> = lazy_regex!(r"(\d+)[pP]_(\d+)[kK]");

/// Pattern for ISO 8601 duration. Accepts both `PT…` and the day-prefixed
/// `P{N}DT…` form some sites emit (e.g. KoreanPornMovie's `P0DT1H1M14S`).
/// The day count is consumed but discarded — observed values are always 0
/// in practice and a per-day conversion would surface a misleading total
/// when the day slot was actually used as a placeholder.
pub static ISO8601_DURATION_PATTERN: Lazy<Regex> =
    lazy_regex!(r"^P(?:\d+D)?T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?$");

/// Pattern for ISO 8601 date (e.g., "2024-01-15" or "2024-01-15T10:30:00Z")
pub static ISO8601_DATE_PATTERN: Lazy<Regex> = lazy_regex!(r"^(\d{4})-(\d{2})-(\d{2})");

/// Pattern to strip HTML tags from text
pub static HTML_TAG_PATTERN: Lazy<Regex> = lazy_regex!(r"<[^>]+>");
