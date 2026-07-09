//! Shared search-URL construction helpers for API-based search extractors.
//!
//! The *base* search URL (host, fixed query params, `search=` position) is
//! per-site knowledge and stays in each extractor. What is genuinely shared —
//! the same knowledge, changing together across sites — is how the standard
//! filter set is appended to an API search URL. Sites whose API accepts the
//! `ordering` / `period` / `category` / `tags[]` filter vocabulary (PornHub,
//! RedTube) delegate that appending here.

use rdlp_types::SearchFilter;
use url::form_urlencoded;

/// Append the standard API search filters to an in-progress search URL.
///
/// Recognizes four filter keys; unknown keys are silently ignored (value
/// validation happens at the CLI/descriptor boundary, not here):
/// - `ordering`, `period` — appended verbatim as `&{key}={value}`.
/// - `category` — appended as `&category={encoded}` (form-urlencoded).
/// - `tags` — comma-separated; each non-empty, trimmed tag is appended as a
///   separate `&tags[]={encoded}` pair.
///
/// The URL must already contain a `?` and its base query params; this only
/// appends `&`-prefixed pairs.
pub(crate) fn append_search_filters(url: &mut String, filters: &[SearchFilter]) {
    for filter in filters {
        match filter.key.as_str() {
            "ordering" => {
                url.push_str("&ordering=");
                url.push_str(&filter.value);
            }
            "period" => {
                url.push_str("&period=");
                url.push_str(&filter.value);
            }
            "category" => {
                let encoded: String =
                    form_urlencoded::byte_serialize(filter.value.as_bytes()).collect();
                url.push_str("&category=");
                url.push_str(&encoded);
            }
            "tags" => {
                for tag in filter.value.split(',') {
                    let trimmed = tag.trim();
                    if !trimmed.is_empty() {
                        let encoded: String =
                            form_urlencoded::byte_serialize(trimmed.as_bytes()).collect();
                        url.push_str("&tags[]=");
                        url.push_str(&encoded);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(key: &str, value: &str) -> SearchFilter {
        SearchFilter {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn append(filters: &[SearchFilter]) -> String {
        let mut url = String::from("https://x.test/?output=json");
        append_search_filters(&mut url, filters);
        url
    }

    #[test]
    fn no_filters_is_noop() {
        assert_eq!(append(&[]), "https://x.test/?output=json");
    }

    #[test]
    fn ordering_and_period_appended_verbatim() {
        let out = append(&[filter("ordering", "newest"), filter("period", "weekly")]);
        assert!(out.ends_with("&ordering=newest&period=weekly"), "got {out}");
    }

    #[test]
    fn category_is_url_encoded() {
        let out = append(&[filter("category", "big ass")]);
        assert!(out.contains("&category=big+ass"), "got {out}");
    }

    #[test]
    fn empty_category_value_appends_empty() {
        let out = append(&[filter("category", "")]);
        assert!(out.ends_with("&category="), "got {out}");
    }

    #[test]
    fn ordering_and_period_are_not_encoded() {
        // Asymmetry lock: ordering/period append VERBATIM (values are
        // enum-constrained upstream), unlike category/tags which are
        // form-urlencoded. A future "helpful" encode of these would be a
        // behavior change — this pins it.
        let out = append(&[filter("ordering", "a b&c"), filter("period", "x y")]);
        assert!(out.contains("&ordering=a b&c"), "got {out}");
        assert!(out.contains("&period=x y"), "got {out}");
    }

    #[test]
    fn tags_split_trim_encode_each() {
        let out = append(&[filter("tags", " red head , milf ")]);
        assert!(out.contains("&tags[]=red+head"), "got {out}");
        assert!(out.contains("&tags[]=milf"), "got {out}");
    }

    #[test]
    fn empty_tag_entries_are_skipped() {
        // Leading/trailing/double commas must not emit empty tags[] pairs.
        let out = append(&[filter("tags", ",a,,b,")]);
        assert_eq!(out.matches("&tags[]=").count(), 2, "got {out}");
        assert!(
            out.contains("&tags[]=a") && out.contains("&tags[]=b"),
            "got {out}"
        );
    }

    #[test]
    fn unknown_filter_key_is_ignored() {
        let out = append(&[filter("bogus", "value"), filter("ordering", "top")]);
        assert!(!out.contains("bogus"), "got {out}");
        assert!(out.ends_with("&ordering=top"), "got {out}");
    }
}
