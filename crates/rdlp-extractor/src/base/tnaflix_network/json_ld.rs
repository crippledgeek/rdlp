//! JSON-LD parsing for TNAFlix network sites.
//!
//! Re-exports shared JSON-LD types and functions from `base::common::json_ld`.
//! All core parsing logic lives in the shared module; this file exists only
//! for backward compatibility with existing imports.

pub(crate) use crate::base::common::json_ld::{
    extract_categories, extract_json_ld, extract_like_count, extract_tags, extract_thumbnails,
    extract_view_count, get_thumbnail_url,
};
