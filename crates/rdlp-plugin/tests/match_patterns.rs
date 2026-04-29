use rdlp_plugin::dispatch::{HostMatch, MatchPattern, MatchTrie, SchemeMatch};
use url::Url;

#[test]
fn parse_basic_https() {
    let p = MatchPattern::parse("https://example.com/*").unwrap();
    assert!(matches!(p.scheme, SchemeMatch::Https));
    assert!(matches!(p.host, HostMatch::Exact(ref h) if h == "example.com"));
}

#[test]
fn parse_subdomain_wildcard() {
    let p = MatchPattern::parse("https://*.example.com/*").unwrap();
    assert!(matches!(p.host, HostMatch::SubdomainWildcard(ref h) if h == "example.com"));
}

#[test]
fn parse_full_tld_wildcard() {
    let p = MatchPattern::parse("https://*/*").unwrap();
    assert!(matches!(p.host, HostMatch::Any));
}

#[test]
fn parse_either_scheme() {
    let p = MatchPattern::parse("*://example.com/*").unwrap();
    assert!(matches!(p.scheme, SchemeMatch::Either));
}

#[test]
fn parse_file_scheme() {
    let p = MatchPattern::parse("file:///*").unwrap();
    assert!(matches!(p.scheme, SchemeMatch::File));
}

#[test]
fn parse_invalid_scheme_fails() {
    assert!(MatchPattern::parse("ftp://x/*").is_err());
}

#[test]
fn parse_no_path_fails() {
    assert!(MatchPattern::parse("https://example.com").is_err());
}

#[test]
fn matches_exact_host() {
    let p = MatchPattern::parse("https://youtube.com/*").unwrap();
    assert!(p.matches(&Url::parse("https://youtube.com/watch?v=1").unwrap()));
    assert!(!p.matches(&Url::parse("https://www.youtube.com/").unwrap()));
}

#[test]
fn matches_subdomain_wildcard() {
    let p = MatchPattern::parse("https://*.youtube.com/*").unwrap();
    assert!(p.matches(&Url::parse("https://www.youtube.com/").unwrap()));
    assert!(p.matches(&Url::parse("https://m.youtube.com/").unwrap()));
    assert!(p.matches(&Url::parse("https://youtube.com/").unwrap())); // bare host also matches
    assert!(!p.matches(&Url::parse("https://vimeo.com/").unwrap()));
}

#[test]
fn matches_path_glob() {
    let p = MatchPattern::parse("https://example.com/api/*").unwrap();
    assert!(p.matches(&Url::parse("https://example.com/api/users").unwrap()));
    assert!(p.matches(&Url::parse("https://example.com/api/").unwrap()));
    assert!(!p.matches(&Url::parse("https://example.com/").unwrap()));
}

#[test]
fn matches_specific_path() {
    let p = MatchPattern::parse("https://example.com/exact").unwrap();
    assert!(p.matches(&Url::parse("https://example.com/exact").unwrap()));
    assert!(!p.matches(&Url::parse("https://example.com/exactly").unwrap()));
}

#[test]
fn either_scheme_matches_http_and_https() {
    let p = MatchPattern::parse("*://example.com/*").unwrap();
    assert!(p.matches(&Url::parse("http://example.com/").unwrap()));
    assert!(p.matches(&Url::parse("https://example.com/").unwrap()));
}

#[test]
fn trie_lookup_returns_matching_values() {
    let mut t: MatchTrie<&'static str> = MatchTrie::default();
    t.insert(MatchPattern::parse("https://*.youtube.com/*").unwrap(), "youtube");
    t.insert(MatchPattern::parse("https://vimeo.com/*").unwrap(), "vimeo");
    t.insert(MatchPattern::parse("https://example.com/api/*").unwrap(), "api");

    let results = t.lookup(&Url::parse("https://m.youtube.com/watch").unwrap());
    assert_eq!(results, vec!["youtube"]);

    let results = t.lookup(&Url::parse("https://vimeo.com/123").unwrap());
    assert_eq!(results, vec!["vimeo"]);

    let results = t.lookup(&Url::parse("https://nope.com/").unwrap());
    assert!(results.is_empty());
}

#[test]
fn trie_lookup_can_return_multiple_values() {
    let mut t: MatchTrie<&'static str> = MatchTrie::default();
    t.insert(MatchPattern::parse("https://*/*").unwrap(), "fallback");
    t.insert(MatchPattern::parse("https://specific.com/*").unwrap(), "specific");
    let results = t.lookup(&Url::parse("https://specific.com/").unwrap());
    assert_eq!(results.len(), 2);
}
