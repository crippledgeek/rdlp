//! Search URL patterns and filter descriptors for TNAFlix.
//!
//! Contains the search URL builder, page URL builder, and filter descriptor
//! definitions used by `TNAFlixSearchExtractor` and `EMPFlixSearchExtractor`.

/// Built-in browse sections (non-category pages).
const BROWSE_SECTIONS: &[&str] = &["new", "toprated", "featured"];

/// Known TNAFlix category slugs (sourced from `/categories` page).
const BROWSE_CATEGORIES: &[&str] = &[
    "amateur-porn",
    "anal-porn",
    "arabian-porn",
    "asian-porn",
    "babe-videos",
    "bbw-porn",
    "bdsm-porn",
    "big-boobs",
    "big-cock",
    "bizarre-porn",
    "blonde-porn",
    "blowjob-videos",
    "brunette-porn",
    "bukkake-porn",
    "cartoon-porn",
    "celebrity-porn",
    "classic-porn",
    "creampie-videos",
    "cum-videos",
    "czech-porn",
    "double-penetration",
    "ebony-porn",
    "euro-porn",
    "facial-porn",
    "fat-porn",
    "feet-porn",
    "fetish-videos",
    "fisting-videos",
    "french-porn",
    "gang-bang",
    "gay-porn",
    "german-porn",
    "granny-porn",
    "group-sex",
    "hairy-porn",
    "handjobs-porn",
    "hardcore-porn",
    "hd-videos",
    "hentai-porn",
    "homemade-porn",
    "indian-porn",
    "interracial-porn",
    "japanese-porn",
    "latina-porn",
    "lesbian-porn",
    "massage-porn",
    "masturbation-videos",
    "mature-porn",
    "milf-porn",
    "oral-sex",
    "petite-porn",
    "piss-videos",
    "porn-compilations",
    "porn-stars",
    "pov-porn",
    "pregnant-porn",
    "public-porn",
    "reality-porn",
    "redhead-porn",
    "russian-porn",
    "shemale-porn",
    "softcore-videos",
    "solo-porn",
    "spanking-videos",
    "squirting-videos",
    "storyline-porn",
    "strapon-sex",
    "teen-porn",
    "thai-porn",
    "threesome-sex",
    "toy-videos",
    "vintage-sex",
    "vr-porn",
    "webcam-shows",
];

/// Build a search URL from a [`SearchQuery`](rdlp_types::SearchQuery) for a given base URL.
///
/// Format: `{base_url}/search?what={encoded_query}&tab=`
///
/// Filters are appended as additional query parameters.
///
/// # Arguments
/// * `base_url` - Site base URL, e.g. `"https://www.tnaflix.com"`.
/// * `query` - The search query with optional filters.
///
/// # Returns
/// A fully-formed search URL string.
pub fn build_search_url_for(base_url: &str, query: &rdlp_types::SearchQuery) -> String {
    let encoded_query = query.query.split_whitespace().collect::<Vec<_>>().join("+");

    let mut url = format!("{base_url}/search?what={encoded_query}&tab=");

    for filter in &query.filters {
        url.push('&');
        url.push_str(&filter.key);
        url.push('=');
        url.push_str(&filter.value);
    }

    url
}

/// Build a search URL for a specific page number for the given base URL.
///
/// Appends `&page={page}` to the base search URL.
///
/// # Arguments
/// * `base_url` - Site base URL, e.g. `"https://www.tnaflix.com"`.
/// * `query` - The search query with optional filters.
/// * `page` - 1-based page number.
///
/// # Returns
/// A fully-formed search URL string for the given page.
pub fn build_search_url_page_for(
    base_url: &str,
    query: &rdlp_types::SearchQuery,
    page: usize,
) -> String {
    let mut url = build_search_url_for(base_url, query);
    url.push_str(&format!("&page={page}"));
    url
}

/// Build a TNAFlix search URL from a [`SearchQuery`](rdlp_types::SearchQuery).
///
/// Format: `https://www.tnaflix.com/search?what={encoded_query}&tab=`
///
/// Filters are appended as additional query parameters.
///
/// # Arguments
/// * `query` - The search query with optional filters.
///
/// # Returns
/// A fully-formed search URL string.
pub fn build_search_url(query: &rdlp_types::SearchQuery) -> String {
    build_search_url_for("https://www.tnaflix.com", query)
}

/// Build a TNAFlix search URL for a specific page number.
///
/// Appends `&page={page}` to the base search URL.
///
/// # Arguments
/// * `query` - The search query with optional filters.
/// * `page` - 1-based page number.
///
/// # Returns
/// A fully-formed search URL string for the given page.
pub fn build_search_url_page(query: &rdlp_types::SearchQuery, page: usize) -> String {
    let mut url = build_search_url(query);
    url.push_str(&format!("&page={page}"));
    url
}

/// Check whether a value is a valid browse section or category slug.
///
/// # Arguments
/// * `value` - The slug to validate.
///
/// # Returns
/// `true` if the value matches a known browse section or category slug.
pub fn is_valid_category(value: &str) -> bool {
    BROWSE_SECTIONS.contains(&value) || BROWSE_CATEGORIES.contains(&value)
}

/// Build a browse URL for page 1 of a section or category for the given base URL.
///
/// # Arguments
/// * `base_url` - Site base URL, e.g. `"https://www.empflix.com"`.
/// * `slug` - A browse section ("new") or category slug ("teen-porn").
///
/// # Returns
/// `{base_url}/{slug}`
pub fn build_browse_url_for(base_url: &str, slug: &str) -> String {
    format!("{base_url}/{slug}")
}

/// Build a browse URL for a specific page number for the given base URL.
///
/// Sections use `/{slug}/{page}`, categories use `/{slug}?page={page}`.
///
/// # Arguments
/// * `base_url` - Site base URL, e.g. `"https://www.empflix.com"`.
/// * `slug` - A browse section or category slug.
/// * `page` - 1-based page number.
pub fn build_browse_url_page_for(base_url: &str, slug: &str, page: usize) -> String {
    if BROWSE_SECTIONS.contains(&slug) {
        format!("{base_url}/{slug}/{page}")
    } else {
        format!("{base_url}/{slug}?page={page}")
    }
}

/// Build a TNAFlix browse URL for page 1 of a section or category.
///
/// # Arguments
/// * `slug` - A browse section ("new") or category slug ("teen-porn").
///
/// # Returns
/// `https://www.tnaflix.com/{slug}`
pub fn build_browse_url(slug: &str) -> String {
    build_browse_url_for("https://www.tnaflix.com", slug)
}

/// Build a TNAFlix browse URL for a specific page number.
///
/// Sections use `/{slug}/{page}`, categories use `/{slug}?page={page}`.
///
/// # Arguments
/// * `slug` - A browse section or category slug.
/// * `page` - 1-based page number.
pub fn build_browse_url_page(slug: &str, page: usize) -> String {
    build_browse_url_page_for("https://www.tnaflix.com", slug, page)
}

/// Convert a kebab-case slug to a human-readable label.
///
/// Strips noise suffixes (`-porn`, `-videos`, `-sex`, `-shows`), title-cases
/// words, and uppercases known abbreviations (HD, VR, BDSM, POV, BBW).
///
/// `"teen-porn"` → `"Teen"`, `"hd-videos"` → `"HD"`, `"threesome-sex"` → `"Threesome"`
pub fn slug_to_label(slug: &str) -> String {
    const NOISE_SUFFIXES: &[&str] = &["-porn", "-videos", "-sex", "-shows"];
    const UPPERCASE_WORDS: &[&str] = &["hd", "vr", "bdsm", "pov", "bbw"];

    let mut stripped = slug;
    for suffix in NOISE_SUFFIXES {
        if let Some(s) = stripped.strip_suffix(suffix) {
            stripped = s;
            break;
        }
    }

    stripped
        .split('-')
        .map(|word| {
            if UPPERCASE_WORDS.contains(&word) {
                word.to_ascii_uppercase()
            } else {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => {
                        let mut s = c.to_uppercase().to_string();
                        s.extend(chars);
                        s
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the static filter descriptors for TNAFlix search.
///
/// # Supported Filters
/// - `ordering` - Sort order (featured, newest, duration, rating)
/// - `category` - Browse section or category slug
pub fn search_filter_descriptors() -> Vec<rdlp_types::SearchFilterDescriptor> {
    let category_values: Vec<rdlp_types::SearchFilterValue> = BROWSE_SECTIONS
        .iter()
        .chain(BROWSE_CATEGORIES.iter())
        .map(|&slug| rdlp_types::SearchFilterValue {
            value: slug.to_string(),
            label: slug_to_label(slug),
        })
        .collect();

    vec![
        rdlp_types::SearchFilterDescriptor {
            key: "ordering".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: vec![
                rdlp_types::SearchFilterValue {
                    value: "featured".to_string(),
                    label: "Featured".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "newest".to_string(),
                    label: "Newest".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "duration".to_string(),
                    label: "Longest".to_string(),
                },
                rdlp_types::SearchFilterValue {
                    value: "rating".to_string(),
                    label: "Top rated".to_string(),
                },
            ],
            default: Some("featured".to_string()),
        },
        rdlp_types::SearchFilterDescriptor {
            key: "category".to_string(),
            display_name: "Category".to_string(),
            allowed_values: category_values,
            default: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url_basic() {
        let query = rdlp_types::SearchQuery {
            query: "test query".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert_eq!(url, "https://www.tnaflix.com/search?what=test+query&tab=");
    }

    #[test]
    fn test_build_search_url_with_filter() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![rdlp_types::SearchFilter {
                key: "ordering".to_string(),
                value: "newest".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(url.contains("what=test"));
        assert!(url.contains("ordering=newest"));
    }

    #[test]
    fn test_build_search_url_no_raw_spaces() {
        let query = rdlp_types::SearchQuery {
            query: "hello world test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url(&query);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
        assert!(url.contains("hello+world+test"));
    }

    #[test]
    fn test_build_search_url_page() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![],
            max_results: None,
            page: None,
        };
        let url = build_search_url_page(&query, 3);
        assert!(url.contains("&page=3"));
    }

    #[test]
    fn test_build_search_url_page_with_filter() {
        let query = rdlp_types::SearchQuery {
            query: "test".to_string(),
            filters: vec![rdlp_types::SearchFilter {
                key: "ordering".to_string(),
                value: "rating".to_string(),
            }],
            max_results: None,
            page: None,
        };
        let url = build_search_url_page(&query, 5);
        assert!(url.contains("ordering=rating"));
        assert!(url.contains("&page=5"));
    }

    #[test]
    fn test_search_filter_descriptors_not_empty() {
        let filters = search_filter_descriptors();
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0].key, "ordering");
        assert_eq!(filters[0].allowed_values.len(), 4);
    }

    #[test]
    fn test_search_filter_descriptors_have_default() {
        let filters = search_filter_descriptors();
        assert_eq!(filters[0].default, Some("featured".to_string()));
    }

    #[test]
    fn test_search_filter_descriptors_values() {
        let filters = search_filter_descriptors();
        let values: Vec<&str> = filters[0]
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"featured"));
        assert!(values.contains(&"newest"));
        assert!(values.contains(&"duration"));
        assert!(values.contains(&"rating"));
    }

    #[test]
    fn test_is_valid_category_sections() {
        assert!(is_valid_category("new"));
        assert!(is_valid_category("toprated"));
        assert!(is_valid_category("featured"));
    }

    #[test]
    fn test_is_valid_category_slugs() {
        assert!(is_valid_category("teen-porn"));
        assert!(is_valid_category("milf-porn"));
        assert!(is_valid_category("hd-videos"));
        assert!(is_valid_category("threesome-sex"));
        assert!(is_valid_category("webcam-shows"));
        assert!(is_valid_category("big-boobs"));
    }

    #[test]
    fn test_is_valid_category_rejects_unknown() {
        assert!(!is_valid_category("unknown"));
        assert!(!is_valid_category(""));
        assert!(!is_valid_category("not-a-category"));
    }

    #[test]
    fn test_build_browse_url() {
        assert_eq!(build_browse_url("new"), "https://www.tnaflix.com/new");
        assert_eq!(
            build_browse_url("teen-porn"),
            "https://www.tnaflix.com/teen-porn"
        );
    }

    #[test]
    fn test_build_browse_url_page_section() {
        // Sections use /{slug}/{page}
        assert_eq!(
            build_browse_url_page("new", 3),
            "https://www.tnaflix.com/new/3"
        );
        assert_eq!(
            build_browse_url_page("toprated", 1),
            "https://www.tnaflix.com/toprated/1"
        );
    }

    #[test]
    fn test_build_browse_url_page_category() {
        // Categories use /{slug}?page={page}
        assert_eq!(
            build_browse_url_page("teen-porn", 2),
            "https://www.tnaflix.com/teen-porn?page=2"
        );
        assert_eq!(
            build_browse_url_page("milf-porn", 5),
            "https://www.tnaflix.com/milf-porn?page=5"
        );
    }

    #[test]
    fn test_slug_to_label_basic() {
        assert_eq!(slug_to_label("teen-porn"), "Teen");
        assert_eq!(slug_to_label("milf-porn"), "Milf");
        assert_eq!(slug_to_label("amateur-porn"), "Amateur");
    }

    #[test]
    fn test_slug_to_label_multi_word() {
        assert_eq!(slug_to_label("big-boobs"), "Big Boobs");
        assert_eq!(slug_to_label("big-cock"), "Big Cock");
        assert_eq!(slug_to_label("double-penetration"), "Double Penetration");
        assert_eq!(slug_to_label("gang-bang"), "Gang Bang");
        assert_eq!(slug_to_label("porn-compilations"), "Porn Compilations");
        assert_eq!(slug_to_label("porn-stars"), "Porn Stars");
    }

    #[test]
    fn test_slug_to_label_uppercase_abbreviations() {
        assert_eq!(slug_to_label("hd-videos"), "HD");
        assert_eq!(slug_to_label("vr-porn"), "VR");
        assert_eq!(slug_to_label("bdsm-porn"), "BDSM");
        assert_eq!(slug_to_label("pov-porn"), "POV");
        assert_eq!(slug_to_label("bbw-porn"), "BBW");
    }

    #[test]
    fn test_slug_to_label_video_suffix() {
        assert_eq!(slug_to_label("babe-videos"), "Babe");
        assert_eq!(slug_to_label("blowjob-videos"), "Blowjob");
        assert_eq!(slug_to_label("creampie-videos"), "Creampie");
    }

    #[test]
    fn test_slug_to_label_sex_suffix() {
        assert_eq!(slug_to_label("threesome-sex"), "Threesome");
        assert_eq!(slug_to_label("oral-sex"), "Oral");
        assert_eq!(slug_to_label("vintage-sex"), "Vintage");
        assert_eq!(slug_to_label("strapon-sex"), "Strapon");
    }

    #[test]
    fn test_slug_to_label_shows_suffix() {
        assert_eq!(slug_to_label("webcam-shows"), "Webcam");
    }

    #[test]
    fn test_slug_to_label_sections() {
        assert_eq!(slug_to_label("new"), "New");
        assert_eq!(slug_to_label("toprated"), "Toprated");
        assert_eq!(slug_to_label("featured"), "Featured");
    }

    #[test]
    fn test_category_filter_descriptor() {
        let filters = search_filter_descriptors();
        let cat = &filters[1];
        assert_eq!(cat.key, "category");
        assert_eq!(cat.display_name, "Category");
        assert!(cat.default.is_none());
        assert!(cat.allowed_values.len() > 74); // sections + categories
    }
}
