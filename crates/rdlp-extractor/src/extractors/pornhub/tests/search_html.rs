use crate::extractors::pornhub::search_html::parse_html_search_results;

const FIXTURE: &str = include_str!("fixtures/search_html_page1.html");

#[test]
fn parses_results_with_uploader_populated() {
    let results = parse_html_search_results(FIXTURE).expect("parser must succeed on real fixture");
    assert!(results.len() >= 30, "expected ≥30 results, got {}", results.len());
    let with_uploader = results.iter().filter(|r| r.uploader.is_some()).count();
    assert!(
        with_uploader >= results.len() * 95 / 100,
        "expected ≥95% of results to carry uploader, got {with_uploader}/{}",
        results.len()
    );
}

#[test]
fn results_carry_titles_and_thumbnails() {
    let results = parse_html_search_results(FIXTURE).unwrap();
    for r in &results {
        assert!(!r.title.is_empty(), "title must not be empty");
        assert!(r.video_url.contains("view_video.php?viewkey="));
    }
}

#[test]
fn at_least_one_result_per_uploader_namespace() {
    let results = parse_html_search_results(FIXTURE).unwrap();
    let mut ns_seen = std::collections::HashSet::new();
    for r in &results {
        if let Some(u) = &r.uploader {
            ns_seen.insert(u.clone());
        }
    }
    assert!(ns_seen.len() >= 10, "uploader names should be diverse");
}
