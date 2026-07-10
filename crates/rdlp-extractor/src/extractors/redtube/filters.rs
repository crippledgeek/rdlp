//! Search filter descriptors for RedTube.
//!
//! Contains the filter definitions (ordering, period, category, tags) and their
//! allowed value lists, sourced from the RedTube JSON API endpoints.

use rdlp_types::{SearchFilterDescriptor, SearchFilterValue};

/// Build the list of known RedTube category filter values.
///
/// Sourced from the `redtube.Categories.getCategoriesList` API endpoint.
/// Test/spam entries and non-adult categories are excluded.
fn category_values() -> Vec<SearchFilterValue> {
    SearchFilterValue::list([
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
    ])
}

/// Build the list of common RedTube tag filter values.
///
/// Sourced from the `redtube.Tags.getTagList` API endpoint.
/// Only widely-useful generic tags are included; performer names,
/// non-English text, and overly-specific compound tags are excluded.
/// Users can still type custom tags not in this list (free-text bypass).
fn tag_values() -> Vec<SearchFilterValue> {
    SearchFilterValue::list([
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
    ])
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
