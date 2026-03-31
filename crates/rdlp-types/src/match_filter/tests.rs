//! Tests for match filter parsing and evaluation.

use serde_json::json;

use super::MatchFilter;

// ============================================================================
// Parser tests
// ============================================================================

#[test]
fn parse_unary_present() {
    let f = MatchFilter::parse("is_live").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_unary_negated() {
    let f = MatchFilter::parse("!is_live").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_comparison_gt() {
    let f = MatchFilter::parse("duration > 60").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_none_inclusive() {
    let f = MatchFilter::parse("like_count >? 100").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_and_conditions() {
    let f = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    assert_eq!(f.conditions.len(), 2);
}

#[test]
fn parse_string_contains() {
    let f = MatchFilter::parse("title *= cats").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_quoted_string() {
    let f = MatchFilter::parse("title = \"hello world\"").unwrap();
    assert_eq!(f.conditions.len(), 1);
}

#[test]
fn parse_filesize_suffix() {
    let f = MatchFilter::parse("filesize > 1M").unwrap();
    assert_eq!(f.conditions.len(), 1);
    // Should evaluate as 1_000_000
    let data = json!({"filesize": 2_000_000});
    assert!(f.evaluate(&data));
}

#[test]
fn parse_duration_value() {
    let f = MatchFilter::parse("duration > 1:30:00").unwrap();
    assert_eq!(f.conditions.len(), 1);
    // 1:30:00 = 5400 seconds
    let data = json!({"duration": 6000});
    assert!(f.evaluate(&data));
    let data = json!({"duration": 3000});
    assert!(!f.evaluate(&data));
}

#[test]
fn parse_empty_fails() {
    assert!(MatchFilter::parse("").is_err());
}

#[test]
fn parse_invalid_fails() {
    assert!(MatchFilter::parse("!!!").is_err());
}

// ============================================================================
// Unary evaluation tests
// ============================================================================

#[test]
fn eval_unary_present_true() {
    let f = MatchFilter::parse("title").unwrap();
    assert!(f.evaluate(&json!({"title": "abc"})));
}

#[test]
fn eval_unary_present_empty_string() {
    let f = MatchFilter::parse("title").unwrap();
    // yt-dlp: empty string is "present" (not None)
    assert!(f.evaluate(&json!({"title": ""})));
}

#[test]
fn eval_unary_present_missing() {
    let f = MatchFilter::parse("title").unwrap();
    assert!(!f.evaluate(&json!({})));
}

#[test]
fn eval_unary_present_null() {
    let f = MatchFilter::parse("title").unwrap();
    assert!(!f.evaluate(&json!({"title": null})));
}

#[test]
fn eval_unary_negated_missing() {
    let f = MatchFilter::parse("!title").unwrap();
    assert!(f.evaluate(&json!({})));
}

#[test]
fn eval_unary_negated_present() {
    let f = MatchFilter::parse("!title").unwrap();
    assert!(!f.evaluate(&json!({"title": "abc"})));
}

#[test]
fn eval_unary_bool_true() {
    let f = MatchFilter::parse("is_live").unwrap();
    assert!(f.evaluate(&json!({"is_live": true})));
}

#[test]
fn eval_unary_bool_false() {
    let f = MatchFilter::parse("is_live").unwrap();
    assert!(!f.evaluate(&json!({"is_live": false})));
}

#[test]
fn eval_unary_negated_bool_false() {
    let f = MatchFilter::parse("!is_live").unwrap();
    assert!(f.evaluate(&json!({"is_live": false})));
}

#[test]
fn eval_unary_negated_bool_true() {
    let f = MatchFilter::parse("!is_live").unwrap();
    assert!(!f.evaluate(&json!({"is_live": true})));
}

// ============================================================================
// Numeric comparison tests
// ============================================================================

#[test]
fn eval_numeric_gt_pass() {
    let f = MatchFilter::parse("duration > 60").unwrap();
    assert!(f.evaluate(&json!({"duration": 120})));
}

#[test]
fn eval_numeric_gt_fail() {
    let f = MatchFilter::parse("duration > 60").unwrap();
    assert!(!f.evaluate(&json!({"duration": 30})));
}

#[test]
fn eval_numeric_gt_equal_fail() {
    let f = MatchFilter::parse("duration > 60").unwrap();
    assert!(!f.evaluate(&json!({"duration": 60})));
}

#[test]
fn eval_numeric_ge() {
    let f = MatchFilter::parse("duration >= 60").unwrap();
    assert!(f.evaluate(&json!({"duration": 60})));
    assert!(f.evaluate(&json!({"duration": 120})));
    assert!(!f.evaluate(&json!({"duration": 59})));
}

#[test]
fn eval_numeric_lt() {
    let f = MatchFilter::parse("duration < 600").unwrap();
    assert!(f.evaluate(&json!({"duration": 300})));
    assert!(!f.evaluate(&json!({"duration": 600})));
}

#[test]
fn eval_numeric_le() {
    let f = MatchFilter::parse("duration <= 600").unwrap();
    assert!(f.evaluate(&json!({"duration": 600})));
    assert!(!f.evaluate(&json!({"duration": 601})));
}

#[test]
fn eval_numeric_eq() {
    let f = MatchFilter::parse("view_count = 1000").unwrap();
    assert!(f.evaluate(&json!({"view_count": 1000})));
    assert!(!f.evaluate(&json!({"view_count": 999})));
}

#[test]
fn eval_numeric_ne() {
    let f = MatchFilter::parse("view_count != 0").unwrap();
    assert!(f.evaluate(&json!({"view_count": 100})));
    assert!(!f.evaluate(&json!({"view_count": 0})));
}

#[test]
fn eval_filesize_1k() {
    let f = MatchFilter::parse("filesize > 1K").unwrap();
    assert!(f.evaluate(&json!({"filesize": 1200})));
    assert!(!f.evaluate(&json!({"filesize": 500})));
}

// ============================================================================
// Missing field / none-inclusive tests
// ============================================================================

#[test]
fn eval_missing_field_fails() {
    let f = MatchFilter::parse("duration > 60").unwrap();
    assert!(!f.evaluate(&json!({})));
}

#[test]
fn eval_none_inclusive_missing_passes() {
    let f = MatchFilter::parse("like_count >? 100").unwrap();
    assert!(f.evaluate(&json!({})));
}

#[test]
fn eval_none_inclusive_present_evaluates() {
    let f = MatchFilter::parse("like_count >? 100").unwrap();
    assert!(f.evaluate(&json!({"like_count": 200})));
    assert!(!f.evaluate(&json!({"like_count": 50})));
}

// ============================================================================
// String comparison tests
// ============================================================================

#[test]
fn eval_string_eq() {
    let f = MatchFilter::parse("title = foobar42").unwrap();
    assert!(f.evaluate(&json!({"title": "foobar42"})));
    assert!(!f.evaluate(&json!({"title": "other"})));
}

#[test]
fn eval_string_ne() {
    let f = MatchFilter::parse("title != foobar42").unwrap();
    assert!(f.evaluate(&json!({"title": "other"})));
    assert!(!f.evaluate(&json!({"title": "foobar42"})));
}

#[test]
fn eval_string_contains() {
    let f = MatchFilter::parse("title *= cats").unwrap();
    assert!(f.evaluate(&json!({"title": "I love cats"})));
    assert!(!f.evaluate(&json!({"title": "I love dogs"})));
}

#[test]
fn eval_string_negated_contains() {
    let f = MatchFilter::parse("title !*= ads").unwrap();
    assert!(f.evaluate(&json!({"title": "great video"})));
    assert!(!f.evaluate(&json!({"title": "ads everywhere"})));
}

#[test]
fn eval_string_starts_with() {
    let f = MatchFilter::parse("title ^= hello").unwrap();
    assert!(f.evaluate(&json!({"title": "hello world"})));
    assert!(!f.evaluate(&json!({"title": "world hello"})));
}

#[test]
fn eval_string_ends_with() {
    let f = MatchFilter::parse("title $= world").unwrap();
    assert!(f.evaluate(&json!({"title": "hello world"})));
    assert!(!f.evaluate(&json!({"title": "world hello"})));
}

#[test]
fn eval_regex_match() {
    let f = MatchFilter::parse(r"title ~= \d+").unwrap();
    assert!(f.evaluate(&json!({"title": "video 123"})));
    assert!(!f.evaluate(&json!({"title": "no numbers"})));
}

// ============================================================================
// AND logic tests
// ============================================================================

#[test]
fn eval_and_both_pass() {
    let f = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    assert!(f.evaluate(&json!({"duration": 120})));
}

#[test]
fn eval_and_first_fails() {
    let f = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    assert!(!f.evaluate(&json!({"duration": 30})));
}

#[test]
fn eval_and_second_fails() {
    let f = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    assert!(!f.evaluate(&json!({"duration": 120, "is_live": true})));
}

// ============================================================================
// Display
// ============================================================================

#[test]
fn display_roundtrip() {
    let f = MatchFilter::parse("duration > 60 & !is_live").unwrap();
    assert_eq!(f.to_string(), "duration > 60 & !is_live");
}
