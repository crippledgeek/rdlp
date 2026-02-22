//! URL and extraction patterns for RedTube
//!
//! Static regex patterns compiled once at first use via `std::sync::LazyLock`.
//! Includes search URL builders and filter descriptors for the JSON API.

use rdlp_core::{SearchFilter, SearchFilterDescriptor, SearchFilterValue};
use regex::Regex;
use std::sync::LazyLock;
use url::form_urlencoded;

/// Static URL pattern regex for RedTube (initialized once at first use)
///
/// Supports:
/// - Standard URLs: https://www.redtube.com/123456
/// - Brazilian domain: https://www.redtube.com.br/123456
/// - Embed URLs: https://embed.redtube.com/?id=123456
pub static REDTUBE_URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?:(?:\w+\.)?redtube\.com(?:\.br)?/|embed\.redtube\.com/\?.*\bid=)(?P<id>\d+)",
    )
    .expect("Valid RedTube URL pattern")
});

/// Regex to extract JavaScript sources object: sources: {"720": "url", ...}
pub static SOURCES_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"sources\s*:\s*(\{[^}]+\})"#).expect("Valid sources pattern"));

/// Regex to extract mediaDefinition array: mediaDefinition: [{...}, ...]
pub static MEDIA_DEF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)mediaDefinition\s*:\s*(\[.+?\])").expect("Valid mediaDefinition pattern")
});

/// Regex to extract video cards from HTML search results.
///
/// Captures:
/// - `url`: Video page URL (href)
/// - `title`: Video title (title attribute)
/// - `thumb`: Thumbnail image URL (src attribute)
/// - `duration`: Duration text (e.g. "12:34")
pub static HTML_VIDEO_CARD_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a[^>]+href="(?P<url>https?://(?:www\.)?redtube\.com/\d+)"[^>]+title="(?P<title>[^"]*)"[^>]*>.*?(?:<img[^>]+src="(?P<thumb>[^"]*)")?.*?(?:<span[^>]*class="[^"]*duration[^"]*"[^>]*>(?P<duration>[\d:]+)</span>)?"#,
    )
    .expect("Valid HTML video card pattern")
});

/// Number of results per API page.
pub(crate) const API_RESULTS_PER_PAGE: u32 = 20;

/// Build the API search URL for the first page.
///
/// Format: `https://api.redtube.com/?data=redtube.Videos.searchVideos&output=json
///          &search={query}&thumbsize=big&{filters}`
pub(crate) fn build_api_search_url(query: &str, filters: &[SearchFilter]) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    let mut url = format!(
        "https://api.redtube.com/?data=redtube.Videos.searchVideos\
         &output=json&search={encoded_query}&thumbsize=big"
    );

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
                // Tags are comma-separated, each sent as tags[]
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
            _ => {} // Unknown filters are silently ignored (validation is done elsewhere)
        }
    }

    url
}

/// Append a page parameter to an existing API search URL.
pub(crate) fn build_api_search_url_page(base_url: &str, page: u32) -> String {
    format!("{base_url}&page={page}")
}

/// Build the HTML search fallback URL.
///
/// Format: `https://www.redtube.com/?search={query}`
pub(crate) fn build_html_search_url(query: &str) -> String {
    let encoded_query: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    format!("https://www.redtube.com/?search={encoded_query}")
}

/// Build the list of known RedTube category filter values.
///
/// Sourced from the `redtube.Categories.getCategoriesList` API endpoint.
/// Test/spam entries and non-adult categories are excluded.
fn category_values() -> Vec<SearchFilterValue> {
    [
        ("Amateur", "Amateur"),
        ("Anal", "Anal"),
        ("Arab", "Arab"),
        ("Asian", "Asian"),
        ("BBW", "BBW"),
        ("Big Ass", "Big Ass"),
        ("Big Dick", "Big Dick"),
        ("Big Tits", "Big Tits"),
        ("Bisexual", "Bisexual"),
        ("Bisexual Male", "Bisexual Male"),
        ("Blonde", "Blonde"),
        ("Blowjob", "Blowjob"),
        ("Bondage", "Bondage"),
        ("Brazilian", "Brazilian"),
        ("Brunette", "Brunette"),
        ("Bukkake", "Bukkake"),
        ("Cartoon", "Cartoon"),
        ("Casting", "Casting"),
        ("Celebrity", "Celebrity"),
        ("College", "College"),
        ("Compilation", "Compilation"),
        ("Cosplay", "Cosplay"),
        ("Creampie", "Creampie"),
        ("Cuckold", "Cuckold"),
        ("Cumshot", "Cumshot"),
        ("Double Penetration", "Double Penetration"),
        ("Ebony", "Ebony"),
        ("Erotic", "Erotic"),
        ("European", "European"),
        ("Facials", "Facials"),
        ("Feet", "Feet"),
        ("Female Orgasm", "Female Orgasm"),
        ("Fetish", "Fetish"),
        ("Fingering", "Fingering"),
        ("Fisting", "Fisting"),
        ("French", "French"),
        ("Gangbang", "Gangbang"),
        ("German", "German"),
        ("Group", "Group"),
        ("Handjob", "Handjob"),
        ("Hardcore", "Hardcore"),
        ("HD", "HD"),
        ("Hentai", "Hentai"),
        ("Indian", "Indian"),
        ("Interracial", "Interracial"),
        ("Japanese", "Japanese"),
        ("Latina", "Latina"),
        ("Lesbian", "Lesbian"),
        ("Lingerie", "Lingerie"),
        ("Massage", "Massage"),
        ("Masturbation", "Masturbation"),
        ("Mature", "Mature"),
        ("MILF", "MILF"),
        ("Muscle", "Muscle"),
        ("Orgy", "Orgy"),
        ("Parody", "Parody"),
        ("Party", "Party"),
        ("Pissing", "Pissing"),
        ("Popular With Women", "Popular With Women"),
        ("POV", "POV"),
        ("Pussy Licking", "Pussy Licking"),
        ("Reality", "Reality"),
        ("Redhead", "Redhead"),
        ("Romantic", "Romantic"),
        ("Rough", "Rough"),
        ("SFW", "SFW"),
        ("Shemale", "Shemale"),
        ("Small Tits", "Small Tits"),
        ("Solo girl", "Solo Girl"),
        ("Solo Male", "Solo Male"),
        ("Squirting", "Squirting"),
        ("Step Fantasy", "Step Fantasy"),
        ("Striptease", "Striptease"),
        ("Tattoos", "Tattoos"),
        ("Teens", "Teens"),
        ("Threesome", "Threesome"),
        ("Toys", "Toys"),
        ("Transgender", "Transgender"),
        ("Verified Amateurs", "Verified Amateurs"),
        ("Vintage", "Vintage"),
        ("Virtual Reality", "Virtual Reality"),
        ("Webcam", "Webcam"),
        ("Young and Old", "Young and Old"),
    ]
    .into_iter()
    .map(|(value, label)| SearchFilterValue {
        value: value.to_string(),
        label: label.to_string(),
    })
    .collect()
}

/// Build the list of common RedTube tag filter values.
///
/// Sourced from the `redtube.Tags.getTagList` API endpoint.
/// Only widely-useful generic tags are included; performer names,
/// non-English text, and overly-specific compound tags are excluded.
/// Users can still type custom tags not in this list (free-text bypass).
fn tag_values() -> Vec<SearchFilterValue> {
    [
        ("18 year old", "18 Year Old"),
        ("amateur", "Amateur"),
        ("amateur couple", "Amateur Couple"),
        ("amateur threesome", "Amateur Threesome"),
        ("anal", "Anal"),
        ("anal compilation", "Anal Compilation"),
        ("anal creampie compilation", "Anal Creampie Compilation"),
        ("anal fisting", "Anal Fisting"),
        ("anime", "Anime"),
        ("asmr", "ASMR"),
        ("bbc", "BBC"),
        ("best blowjob", "Best Blowjob"),
        ("big clit", "Big Clit"),
        ("blowjob deepthroat", "Blowjob Deepthroat"),
        ("cheating wife", "Cheating Wife"),
        ("compilation", "Compilation"),
        ("creampie compilation", "Creampie Compilation"),
        ("cum in mouth", "Cum in Mouth"),
        ("cum in pussy", "Cum in Pussy"),
        ("curvy", "Curvy"),
        ("deep throat", "Deep Throat"),
        ("doggy", "Doggy"),
        ("dp", "DP"),
        ("face fuck", "Face Fuck"),
        ("facesitting", "Facesitting"),
        ("fake taxi", "Fake Taxi"),
        ("femdom", "Femdom"),
        ("first time anal", "First Time Anal"),
        ("glory hole", "Glory Hole"),
        ("high heels", "High Heels"),
        ("homemade", "Homemade"),
        ("huge tits", "Huge Tits"),
        ("joi", "JOI"),
        ("ladyboy", "Ladyboy"),
        ("lesbian massage", "Lesbian Massage"),
        ("missionary", "Missionary"),
        ("monster cock", "Monster Cock"),
        ("multiple creampie", "Multiple Creampie"),
        ("onlyfans", "OnlyFans"),
        ("pegging", "Pegging"),
        ("pregnant", "Pregnant"),
        ("prostate massage", "Prostate Massage"),
        ("public sex", "Public Sex"),
        ("real orgasm", "Real Orgasm"),
        ("reverse cowgirl", "Reverse Cowgirl"),
        ("rough anal", "Rough Anal"),
        ("sex machine", "Sex Machine"),
        ("shower", "Shower"),
        ("slave", "Slave"),
        ("solo girl", "Solo Girl"),
        ("teacher", "Teacher"),
        ("trans", "Trans"),
        ("vr", "VR"),
        ("yoga", "Yoga"),
    ]
    .into_iter()
    .map(|(value, label)| SearchFilterValue {
        value: value.to_string(),
        label: label.to_string(),
    })
    .collect()
}

/// Return the static filter descriptors for RedTube search.
///
/// Defines four filters:
/// - `ordering`: sort order (enum)
/// - `period`: time period (enum)
/// - `category`: category (enum, sourced from API)
/// - `tags`: comma-separated tags (enum with free-text bypass)
pub fn search_filter_descriptors() -> Vec<SearchFilterDescriptor> {
    vec![
        SearchFilterDescriptor {
            key: "ordering".to_string(),
            display_name: "Sort by".to_string(),
            allowed_values: vec![
                SearchFilterValue {
                    value: "relevance".to_string(),
                    label: "Relevance".to_string(),
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
                SearchFilterValue {
                    value: "mostfavoured".to_string(),
                    label: "Most favoured".to_string(),
                },
            ],
            default: Some("relevance".to_string()),
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
            allowed_values: tag_values(),
            default: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern_standard() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://www.redtube.com/123456"));
        assert!(REDTUBE_URL_PATTERN.is_match("https://redtube.com/12345678"));
    }

    #[test]
    fn test_url_pattern_brazilian() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://www.redtube.com.br/987654"));
    }

    #[test]
    fn test_url_pattern_embed() {
        assert!(REDTUBE_URL_PATTERN.is_match("https://embed.redtube.com/?id=123456"));
    }

    #[test]
    fn test_url_pattern_invalid() {
        assert!(!REDTUBE_URL_PATTERN.is_match("https://youtube.com/watch?v=test"));
        assert!(!REDTUBE_URL_PATTERN.is_match("https://www.tnaflix.com/video/123"));
    }

    #[test]
    fn test_sources_pattern() {
        let webpage = r#"sources: {"720": "url1", "1080": "url2"}"#;
        assert!(SOURCES_PATTERN.is_match(webpage));

        let caps = SOURCES_PATTERN.captures(webpage).unwrap();
        assert!(caps.get(1).unwrap().as_str().contains("720"));
    }

    #[test]
    fn test_media_def_pattern() {
        let webpage = r#"mediaDefinition: [{"videoUrl": "test"}]"#;
        assert!(MEDIA_DEF_PATTERN.is_match(webpage));
    }

    #[test]
    fn test_build_api_search_url_basic() {
        let url = build_api_search_url("test query", &[]);
        assert!(url.starts_with("https://api.redtube.com/"));
        assert!(url.contains("search=test+query"));
        assert!(url.contains("output=json"));
        assert!(url.contains("thumbsize=big"));
    }

    #[test]
    fn test_build_api_search_url_with_ordering() {
        let filters = vec![SearchFilter {
            key: "ordering".to_string(),
            value: "newest".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("ordering=newest"));
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
    fn test_build_api_search_url_with_category() {
        let filters = vec![SearchFilter {
            key: "category".to_string(),
            value: "Amateur".to_string(),
        }];
        let url = build_api_search_url("test", &filters);
        assert!(url.contains("category=Amateur"));
    }

    #[test]
    fn test_build_api_search_url_encodes_special_chars() {
        let url = build_api_search_url("hello world", &[]);
        assert!(!url.contains(' '), "URL must not contain raw spaces");
    }

    #[test]
    fn test_build_api_search_url_page() {
        let base = "https://api.redtube.com/?data=redtube.Videos.searchVideos&output=json&search=test&thumbsize=big";
        let paged = build_api_search_url_page(base, 3);
        assert!(paged.ends_with("&page=3"));
    }

    #[test]
    fn test_build_html_search_url() {
        let url = build_html_search_url("test query");
        assert_eq!(url, "https://www.redtube.com/?search=test+query");
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
    fn test_search_filter_descriptors_ordering_values() {
        let filters = search_filter_descriptors();
        let ordering = filters.iter().find(|f| f.key == "ordering").unwrap();
        assert_eq!(ordering.allowed_values.len(), 5);
        assert_eq!(ordering.default, Some("relevance".to_string()));
        let values: Vec<&str> = ordering
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"relevance"));
        assert!(values.contains(&"newest"));
        assert!(values.contains(&"mostviewed"));
        assert!(values.contains(&"rating"));
        assert!(values.contains(&"mostfavoured"));
    }

    #[test]
    fn test_search_filter_descriptors_period_values() {
        let filters = search_filter_descriptors();
        let period = filters.iter().find(|f| f.key == "period").unwrap();
        assert_eq!(period.allowed_values.len(), 3);
        assert_eq!(period.default, Some("alltime".to_string()));
    }

    #[test]
    fn test_search_filter_descriptors_category_values() {
        let filters = search_filter_descriptors();
        let category = filters.iter().find(|f| f.key == "category").unwrap();
        assert!(!category.allowed_values.is_empty());
        assert_eq!(category.default, None);
        // Verify well-known categories are present
        let values: Vec<&str> = category
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"Amateur"));
        assert!(values.contains(&"Anal"));
        assert!(values.contains(&"MILF"));
        assert!(values.contains(&"HD"));
        assert!(values.contains(&"Lesbian"));
        assert!(values.contains(&"Threesome"));
    }

    #[test]
    fn test_search_filter_descriptors_tag_values() {
        let filters = search_filter_descriptors();
        let tags = filters.iter().find(|f| f.key == "tags").unwrap();
        assert!(!tags.allowed_values.is_empty());
        assert_eq!(tags.default, None);
        // Verify well-known tags are present
        let values: Vec<&str> = tags
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect();
        assert!(values.contains(&"amateur"));
        assert!(values.contains(&"homemade"));
        assert!(values.contains(&"compilation"));
    }
}
