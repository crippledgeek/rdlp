use crate::extractors::pornhub::search_html::parse_html_search_results;

const FIXTURE: &str = include_str!("fixtures/search_html_page1.html");

#[test]
fn parses_results_with_uploader_populated() {
    let results = parse_html_search_results(FIXTURE).expect("parser must succeed on real fixture");
    assert!(
        results.len() >= 30,
        "expected ≥30 results, got {}",
        results.len()
    );
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
    let mut with_thumb = 0;
    for r in &results {
        assert!(!r.title.is_empty(), "title must not be empty");
        assert!(r.video_url.contains("view_video.php?viewkey="));
        // The name promised thumbnails and the body never checked one. The
        // count assertion below is the live guard: every card in this fixture
        // carries a poster, so any regression that resolves them away reds it.
        // The parse/scheme assertions cannot fail on this recording — every
        // `src` in it is already an absolute https URL, so they would pass
        // even with no resolver at all. They are here so a future fixture
        // carrying a relative or `data:` poster cannot pass silently.
        if let Some(t) = &r.thumbnail_url {
            let url = url::Url::parse(t).expect("a poster URL must be absolute");
            assert!(
                matches!(url.scheme(), "http" | "https"),
                "poster must be http(s): {t}"
            );
            with_thumb += 1;
        }
    }
    assert_eq!(
        with_thumb,
        results.len(),
        "every card in the recorded fixture carries a usable poster"
    );
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
