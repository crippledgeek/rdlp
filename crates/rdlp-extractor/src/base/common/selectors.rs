//! Common static CSS selectors and regex patterns
//!
//! Pre-compiled selectors and patterns used across multiple extractors.
//! Initialized once at first use via `std::sync::LazyLock`.

use regex::Regex;
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
pub static OG_TITLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:title"]"#).expect("Valid OG title selector")
});

/// Selector for Open Graph description: `<meta property="og:description" content="...">`
pub static OG_DESCRIPTION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:description"]"#).expect("Valid OG description selector")
});

/// Selector for Open Graph image: `<meta property="og:image" content="...">`
pub static OG_IMAGE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[property="og:image"]"#).expect("Valid OG image selector")
});

/// Selector for meta description: `<meta name="description" content="...">`
pub static META_DESCRIPTION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[name="description"]"#).expect("Valid meta description selector")
});

/// Selector for Twitter title: `<meta name="twitter:title" content="...">`
pub static TWITTER_TITLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[name="twitter:title"]"#).expect("Valid Twitter title selector")
});

/// Selector for Twitter image: `<meta name="twitter:image" content="...">`
pub static TWITTER_IMAGE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[name="twitter:image"]"#).expect("Valid Twitter image selector")
});

/// Selector for HTML title tag: `<title>...</title>`
pub static TITLE_TAG_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("title").expect("Valid title selector"));

/// Selector for H1 heading: `<h1>...</h1>`
pub static H1_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("h1").expect("Valid h1 selector"));

/// Selector for JSON-LD scripts: `<script type="application/ld+json">`
pub static JSONLD_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"script[type="application/ld+json"]"#).expect("Valid JSON-LD selector")
});

/// Selector for canonical link: `<link rel="canonical" href="...">`
pub static CANONICAL_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"link[rel="canonical"]"#).expect("Valid canonical selector")
});

// ============================================================================
// Common Static Patterns
// ============================================================================

/// Pattern to extract quality from URL (e.g., "720p", "1080P")
pub static QUALITY_FROM_URL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)[pP]").expect("Valid quality pattern"));

/// Pattern to extract bitrate from URL (e.g., "720P_4000K")
pub static BITRATE_FROM_URL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)[pP]_(\d+)[kK]").expect("Valid bitrate pattern"));

/// Pattern for ISO 8601 duration (e.g., "PT1H2M3S")
pub static ISO8601_DURATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?$")
        .expect("Valid ISO8601 duration pattern")
});

/// Pattern for ISO 8601 date (e.g., "2024-01-15" or "2024-01-15T10:30:00Z")
pub static ISO8601_DATE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").expect("Valid ISO8601 date pattern"));

/// Pattern to strip HTML tags from text
pub static HTML_TAG_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("Valid HTML tag pattern"));
