use crate::extractors::pornhub::PornHubExtractor;
use rdlp_core::{InfoExtractor, SearchExtractor};

#[test]
fn test_extractor_creation() {
    let extractor = PornHubExtractor::new();
    assert_eq!(InfoExtractor::name(&extractor), "PornHub");
}

#[test]
fn test_suitable_urls() {
    let extractor = PornHubExtractor::new();

    // Video URLs
    assert!(extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
    assert!(extractor.suitable("https://www.pornhub.com/embed/ph456"));
    assert!(extractor.suitable("https://de.pornhub.com/view_video.php?viewkey=ph789"));

    // Playlist URLs
    assert!(extractor.suitable("https://www.pornhub.com/playlist/123456"));

    // Invalid URLs
    assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
}

#[test]
fn test_pornhub_implements_search_extractor() {
    let extractor = PornHubExtractor::new();
    let filters = <PornHubExtractor as SearchExtractor>::supported_filters(&extractor);
    assert!(!filters.is_empty());
    assert_eq!(
        <PornHubExtractor as SearchExtractor>::name(&extractor),
        "PornHub"
    );
}

#[test]
fn test_search_filters_have_ordering() {
    let extractor = PornHubExtractor::new();
    let filters = <PornHubExtractor as SearchExtractor>::supported_filters(&extractor);
    let ordering = filters.iter().find(|f| f.key == "ordering");
    assert!(ordering.is_some());
    assert_eq!(ordering.unwrap().allowed_values.len(), 4);
}

#[test]
fn test_search_filters_have_period() {
    let extractor = PornHubExtractor::new();
    let filters = <PornHubExtractor as SearchExtractor>::supported_filters(&extractor);
    let period = filters.iter().find(|f| f.key == "period");
    assert!(period.is_some());
    assert_eq!(period.unwrap().allowed_values.len(), 3);
}

#[test]
fn test_search_filters_have_category() {
    let extractor = PornHubExtractor::new();
    let filters = <PornHubExtractor as SearchExtractor>::supported_filters(&extractor);
    let category = filters.iter().find(|f| f.key == "category");
    assert!(category.is_some());
    assert!(!category.unwrap().allowed_values.is_empty());
}
