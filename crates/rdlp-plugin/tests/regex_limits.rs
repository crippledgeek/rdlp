use rdlp_plugin::dispatch::compile_url_regex;
use rdlp_plugin::PluginError;

#[test]
fn normal_regex_compiles() {
    let r = compile_url_regex("test", r"^https?://example\.com/(?P<id>\d+)").unwrap();
    assert!(r.is_match("https://example.com/123"));
}

#[test]
fn regex_with_explosive_counted_repetition_rejected() {
    // (a){5}{5}{5}{5}{5} expands the compiled DFA exponentially. The size_limit
    // (64KB) should reject it.
    let evil = "a".repeat(20) + "{5}{5}{5}{5}{5}";
    let result = compile_url_regex("test", &evil);
    assert!(result.is_err(), "expected error from explosive regex");
}

#[test]
fn syntactically_invalid_regex_rejected_with_clear_error() {
    let result = compile_url_regex("test", "(unclosed group");
    assert!(matches!(
        result,
        Err(PluginError::RegexCompile { .. })
    ));
}

#[test]
fn empty_regex_compiles_but_matches_anything_at_position_zero() {
    // Empty regex is technically valid; matches at every position.
    let r = compile_url_regex("test", "").unwrap();
    assert!(r.is_match(""));
    assert!(r.is_match("https://x"));
}

#[test]
fn captures_named_groups_work() {
    let r =
        compile_url_regex("test", r"^https://(?P<host>[^/]+)/(?P<id>\d+)").unwrap();
    let caps = r.captures("https://example.com/42").unwrap();
    assert_eq!(&caps["host"], "example.com");
    assert_eq!(&caps["id"], "42");
}

#[test]
fn plugin_name_appears_in_error() {
    let result = compile_url_regex("youtube", "(invalid");
    let Err(PluginError::RegexCompile { plugin, .. }) = result else {
        panic!("expected RegexCompile error");
    };
    assert_eq!(plugin, "youtube");
}
