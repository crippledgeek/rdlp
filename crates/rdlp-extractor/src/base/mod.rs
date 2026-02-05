//! Base extractor functionality for site families
//!
//! This module provides shared extraction logic at multiple levels:
//!
//! ## Architecture
//!
//! The base extractor system uses a three-tier hierarchy:
//!
//! 1. **Common Base** (`common.rs`) - Generic utilities for ALL extractors
//!    - Webpage fetching with error handling
//!    - URL validation and security checks
//!    - Metadata extraction (title, description, thumbnail)
//!    - Size detection via HEAD/Range requests
//!    - Format building utilities
//!    - Logging helpers
//!
//! 2. **Network Bases** (`tnaflix_network.rs`, etc.) - Protocol/network-specific logic
//!    - Site family patterns (TNAFlix network uses same HTML structure)
//!    - XML/JSON parsing specific to site families
//!    - Config URL extraction patterns
//!
//! 3. **Site Extractors** (in `extractors/`) - Individual site implementations
//!    - Site-specific URL patterns
//!    - Custom extraction logic
//!    - Playlist handling
//!
//! ## Usage
//!
//! ```rust,ignore
//! use rdlp_extractor::base::common::BaseExtractor;
//! use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
//!
//! // Generic utilities (all extractors)
//! let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
//! let title = BaseExtractor::extract_title_multi_strategy(&html);
//! let size = BaseExtractor::detect_file_size(&video_url, ctx).await;
//!
//! // Network-specific utilities (TNAFlix family only)
//! let base = TnaFlixNetworkBase::new();
//! let metadata = base.extract_metadata(&html)?;
//! let config_url = base.extract_config_url(&webpage);
//! ```
//!
//! ## Common Patterns
//!
//! All extractors should follow these patterns from the base modules:
//!
//! - Use `BaseExtractor::fetch_webpage()` for HTTP requests
//! - Use `BaseExtractor::validate_url_security()` for SSRF protection
//! - Use `BaseExtractor::log_if_verbose()` for debug output
//! - Use `BaseExtractor::detect_file_size()` for filesize detection
//! - Use static `Lazy<Regex>` for URL patterns (see `common.rs` examples)

pub mod common;
pub mod kvs;
pub mod tnaflix_network;
