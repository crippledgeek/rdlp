//! URL builders and constants for PornHub search API.

use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchFilterValue};
use url::form_urlencoded;

/// PornHub Webmaster API search base URL.
const API_BASE: &str = "https://www.pornhub.com/webmasters/search";

/// HTML search fallback base URL.
const HTML_SEARCH_BASE: &str = "https://www.pornhub.com/video/search";

/// Build the API search URL for the first page.
///
/// # Arguments
/// * `query` - Search keyword string.
/// * `filters` - Slice of active search filters.
///
/// # Returns
/// Full API URL string with query parameters.
pub(crate) fn build_api_search_url(query: &str, filters: &[SearchFilter]) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let mut url = format!("{API_BASE}?search={encoded_query}&output=json&thumbsize=large");

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

    url
}

/// Append a page parameter to an existing API search URL.
///
/// # Arguments
/// * `base_url` - The base URL returned by `build_api_search_url`.
/// * `page` - 1-based page number.
///
/// # Returns
/// URL string with `&page={page}` appended.
pub(crate) fn build_api_search_url_page(base_url: &str, page: u32) -> String {
    format!("{base_url}&page={page}")
}

/// Build the HTML search fallback URL.
///
/// # Arguments
/// * `query` - Search keyword string.
/// * `page` - 1-based page number.
///
/// # Returns
/// HTML search URL string.
pub(crate) fn build_html_search_url(query: &str, page: u32) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    format!("{HTML_SEARCH_BASE}?search={encoded_query}&page={page}")
}

/// Build the list of PornHub category filter values.
///
/// Sourced from the `pornhub.com/webmasters/categories` API endpoint (180 categories).
/// Only the most common/useful categories are included in the dropdown.
/// Free-text is also accepted — the API validates server-side.
fn category_values() -> Vec<SearchFilterValue> {
    [
        ("amateur", "Amateur"),
        ("anal", "Anal"),
        ("arab", "Arab"),
        ("asian", "Asian"),
        ("babe", "Babe"),
        ("bbw", "BBW"),
        ("big-ass", "Big Ass"),
        ("big-dick", "Big Dick"),
        ("big-tits", "Big Tits"),
        ("blonde", "Blonde"),
        ("blowjob", "Blowjob"),
        ("bondage", "Bondage"),
        ("brunette", "Brunette"),
        ("bukkake", "Bukkake"),
        ("cartoon", "Cartoon"),
        ("casting", "Casting"),
        ("celebrity", "Celebrity"),
        ("compilation", "Compilation"),
        ("cosplay", "Cosplay"),
        ("creampie", "Creampie"),
        ("cuckold", "Cuckold"),
        ("cumshot", "Cumshot"),
        ("double-penetration", "Double Penetration"),
        ("ebony", "Ebony"),
        ("euro", "Euro"),
        ("feet", "Feet"),
        ("fetish", "Fetish"),
        ("fingering", "Fingering"),
        ("fisting", "Fisting"),
        ("gangbang", "Gangbang"),
        ("handjob", "Handjob"),
        ("hardcore", "Hardcore"),
        ("hd-porn", "HD Porn"),
        ("hentai", "Hentai"),
        ("indian", "Indian"),
        ("interracial", "Interracial"),
        ("japanese", "Japanese"),
        ("latina", "Latina"),
        ("lesbian", "Lesbian"),
        ("massage", "Massage"),
        ("masturbation", "Masturbation"),
        ("mature", "Mature"),
        ("milf", "MILF"),
        ("orgy", "Orgy"),
        ("parody", "Parody"),
        ("pornstar", "Pornstar"),
        ("pov", "POV"),
        ("public", "Public"),
        ("pussy-licking", "Pussy Licking"),
        ("reality", "Reality"),
        ("red-head", "Red Head"),
        ("role-play", "Role Play"),
        ("romantic", "Romantic"),
        ("rough-sex", "Rough Sex"),
        ("small-tits", "Small Tits"),
        ("solo-female", "Solo Female"),
        ("squirt", "Squirt"),
        ("step-fantasy", "Step Fantasy"),
        ("striptease", "Striptease"),
        ("threesome", "Threesome"),
        ("toys", "Toys"),
        ("transgender", "Transgender"),
        ("vintage", "Vintage"),
        ("webcam", "Webcam"),
    ]
    .into_iter()
    .map(|(value, label)| SearchFilterValue {
        value: value.to_string(),
        label: label.to_string(),
    })
    .collect()
}

/// Return the static filter descriptors for PornHub search.
///
/// Defines four filters:
/// - `ordering`: sort order (enum — 4 values)
/// - `period`: time period (enum — 3 values)
/// - `category`: category (dropdown + free-text — 64 common values, API accepts any)
/// - `tags`: comma-separated tags (free-text only — 445k+ tags exist)
///
/// # Returns
/// Vector of filter descriptors.
pub(crate) fn search_filter_descriptors() -> Vec<SearchFilterDescriptor> {
    vec![
        SearchFilterDescriptor {
            key: "ordering".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "featured".to_string(),
                    label: "Featured".to_string(),
                },
                SearchFilterValue {
                    value: "newest".to_string(),
                    label: "Newest".to_string(),
                },
                SearchFilterValue {
                    value: "mostviewed".to_string(),
                    label: "Most viewed".to_string(),
                },
                SearchFilterValue {
                    value: "rating".to_string(),
                    label: "Top rated".to_string(),
                },
            ],
            default: Some("featured".to_string()),
        },
        SearchFilterDescriptor {
            key: "period".to_string(),
            display_name: "Time period".to_string(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "alltime".to_string(),
                    label: "All time".to_string(),
                },
                SearchFilterValue {
                    value: "weekly".to_string(),
                    label: "This week".to_string(),
                },
                SearchFilterValue {
                    value: "monthly".to_string(),
                    label: "This month".to_string(),
                },
            ],
            default: Some("alltime".to_string()),
        },
        SearchFilterDescriptor {
            key: "category".to_string(),
            display_name: "Category".to_string(),
            allowed_values: category_values(),
            default: None,
        },
        SearchFilterDescriptor {
            key: "tags".to_string(),
            display_name: "Tags".to_string(),
            allowed_values: vec![],
            default: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_api_search_url_basic() {
        let url = build_api_search_url("test query", &[]);
        assert!(url.starts_with("https://www.pornhub.com/webmasters/search?"));
        assert!(url.contains("search=test+query"));
        assert!(url.contains("output=json"));
        assert!(url.contains("thumbsize=large"));
    }

    #[test]
    fn test_build_api_search_url_encodes_special_chars() {
        let url = build_api_search_url("hello world", &[]);
        assert!(!url.contains(' '));
    }

    #[test]
    fn test_build_api_search_url_with_ordering() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "newest".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("&ordering=newest"));
    }

    #[test]
    fn test_build_api_search_url_with_period() {
        let filters = vec![SearchFilter {
            key: "period".to_string(),
            value: "weekly".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("&period=weekly"));
    }

    #[test]
    fn test_build_api_search_url_with_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "amateur".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("&category=amateur"));
    }

    #[test]
    fn test_build_api_search_url_with_tags() {
        let filters = vec![SearchFilter {
            key: "tags".to_string(),
            value: "tag1,tag2".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("tags[]=tag1"));
        assert!(url.contains("tags[]=tag2"));
    }

    #[test]
    fn test_build_api_search_url_page() {
        let base =
            "https://www.pornhub.com/webmasters/search?search=test&output=json&thumbsize=large";
        let paged = build_api_search_url_page(base, 3);
        assert!(paged.ends_with("&page=3"));
    }

    #[test]
    fn test_build_html_search_url() {
        let url = build_html_search_url("test query", 1);
        assert_eq!(
            url,
            "https://www.pornhub.com/video/search?search=test+query&page=1"
        );
    }

    #[test]
    fn test_search_filter_descriptors_count() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 4);
        let keys: Vec<&str> = filters.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"ordering"));
        assert!(keys.contains(&"period"));
        assert!(keys.contains(&"category"));
        assert!(keys.contains(&"tags"));
    }

    #[test]
    fn test_search_filter_descriptors_ordering() {
        let filters = search_filter_descriptors();
        let ordering = filters.iter().find(|f| f.key == "ordering").unwrap();
        assert_eq!(ordering.allowed_values.len(), 4);
        assert_eq!(ordering.default, Some("featured".to_string()));
        let values: Vec<&str> = ordering
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"featured"));
        assert!(values.contains(&"newest"));
        assert!(values.contains(&"mostviewed"));
        assert!(values.contains(&"rating"));
    }

    #[test]
    fn test_search_filter_descriptors_period() {
        let filters = search_filter_descriptors();
        let period = filters.iter().find(|f| f.key == "period").unwrap();
        assert_eq!(period.allowed_values.len(), 3);
        assert_eq!(period.default, Some("alltime".to_string()));
    }

    #[test]
    fn test_search_filter_descriptors_category() {
        let filters = search_filter_descriptors();
        let category = filters.iter().find(|f| f.key == "category").unwrap();
        assert!(!category.allowed_values.is_empty());
        assert_eq!(category.default, None);
        let values: Vec<&str> = category
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"amateur"));
        assert!(values.contains(&"anal"));
        assert!(values.contains(&"milf"));
        assert!(values.contains(&"hd-porn"));
        assert!(values.contains(&"lesbian"));
    }

    #[test]
    fn test_search_filter_descriptors_tags_empty() {
        let filters = search_filter_descriptors();
        let tags = filters.iter().find(|f| f.key == "tags").unwrap();
        assert!(
            tags.allowed_values.is_empty(),
            "Tags should be free-text only (445k+ tags exist)"
        );
        assert_eq!(tags.default, None);
    }
}
