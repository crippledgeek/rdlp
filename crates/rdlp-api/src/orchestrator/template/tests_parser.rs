//! Parser tests for `OutputTemplate`
#![allow(clippy::indexing_slicing, clippy::match_wildcard_for_single_variants)]

use super::*;

#[test]
fn test_parse_default_template() {
    let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
    assert_eq!(t.segments.len(), 3); // field, literal ".", field
}

#[test]
fn test_parse_literal_only() {
    let t = OutputTemplate::parse("hello world").unwrap();
    assert_eq!(t.segments.len(), 1);
    assert!(matches!(&t.segments[0], Segment::Literal(s) if s == "hello world"));
}

#[test]
fn test_parse_field_with_default() {
    let t = OutputTemplate::parse("%(uploader|Unknown)s").unwrap();
    assert_eq!(t.segments.len(), 1);
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "uploader");
            assert_eq!(spec.default.as_deref(), Some("Unknown"));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_numeric_format() {
    let t = OutputTemplate::parse("%(playlist_index)03d").unwrap();
    assert_eq!(t.segments.len(), 1);
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "playlist_index");
            assert!(spec.format.zero_pad);
            assert_eq!(spec.format.width, Some(3));
            assert_eq!(spec.format.type_char, FormatType::Integer);
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_subdirectory_template() {
    let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
    assert_eq!(t.segments.len(), 5); // field, "/", field, ".", field
}

#[test]
fn test_parse_error_unclosed_field() {
    let err = OutputTemplate::parse("%(title").unwrap_err();
    assert!(matches!(err, TemplateError::UnclosedField { position: 0 }));
}

#[test]
fn test_parse_error_empty_field_name() {
    let err = OutputTemplate::parse("%()s").unwrap_err();
    assert!(matches!(err, TemplateError::EmptyFieldName { position: 0 }));
}

#[test]
fn test_parse_error_invalid_specifier() {
    let err = OutputTemplate::parse("%(title)x").unwrap_err();
    assert!(matches!(err, TemplateError::InvalidFormatSpecifier { .. }));
}

#[test]
fn test_parse_empty_default() {
    let t = OutputTemplate::parse("%(uploader|)s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "uploader");
            assert_eq!(spec.default.as_deref(), Some(""));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_integer_no_padding() {
    let t = OutputTemplate::parse("%(view_count)d").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert!(!spec.format.zero_pad);
            assert_eq!(spec.format.width, None);
            assert_eq!(spec.format.type_char, FormatType::Integer);
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_literal_percent() {
    let t = OutputTemplate::parse("100%% done").unwrap();
    assert_eq!(t.segments.len(), 1);
    assert!(matches!(&t.segments[0], Segment::Literal(s) if s == "100% done"));
}

#[test]
fn test_parse_fallback_fields() {
    let t = OutputTemplate::parse("%(uploader,channel,title)s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names.len(), 3);
            assert_eq!(spec.names[0].name, "uploader");
            assert_eq!(spec.names[1].name, "channel");
            assert_eq!(spec.names[2].name, "title");
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_conditional_replacement() {
    let t = OutputTemplate::parse("%(uploader&by {})s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "uploader");
            assert_eq!(spec.replacement.as_deref(), Some("by {}"));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_date_format() {
    let t = OutputTemplate::parse("%(upload_date>%Y-%m-%d)s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "upload_date");
            assert_eq!(spec.names[0].date_format.as_deref(), Some("%Y-%m-%d"));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_arithmetic() {
    let t = OutputTemplate::parse("%(playlist_index+10)d").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "playlist_index");
            assert_eq!(spec.names[0].arithmetic.len(), 1);
            assert!(matches!(spec.names[0].arithmetic[0].0, ArithOp::Add));
            assert!((spec.names[0].arithmetic[0].1 - 10.0).abs() < f64::EPSILON);
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_object_traversal() {
    let t = OutputTemplate::parse("%(tags.0)s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "tags");
            assert_eq!(spec.names[0].accessors.len(), 1);
            assert!(matches!(spec.names[0].accessors[0], Accessor::Index(0)));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_string_slice() {
    let t = OutputTemplate::parse("%(title.:50)s").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.names[0].name, "title");
            assert_eq!(spec.names[0].accessors.len(), 1);
            assert!(matches!(
                spec.names[0].accessors[0],
                Accessor::Slice {
                    start: None,
                    end: Some(50)
                }
            ));
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_hash_flag() {
    let t = OutputTemplate::parse("%(title)#j").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert!(spec.format.hash);
            assert_eq!(spec.format.type_char, FormatType::Json);
        }
        _ => panic!("Expected Field segment"),
    }
}

#[test]
fn test_parse_float_format() {
    let t = OutputTemplate::parse("%(duration).2f").unwrap();
    match &t.segments[0] {
        Segment::Field(spec) => {
            assert_eq!(spec.format.type_char, FormatType::Float);
            assert_eq!(spec.format.precision, Some(2));
        }
        _ => panic!("Expected Field segment"),
    }
}
