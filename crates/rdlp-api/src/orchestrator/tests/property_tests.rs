//! Property-based tests for filename sanitization

use super::create_test_orchestrator;
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_sanitize_filename_never_produces_invalid_chars(
        filename in "[a-zA-Z0-9 /\\\\:*?\"<>|.-]{0,100}"
    ) {
        let orchestrator = create_test_orchestrator();
        let sanitized = orchestrator.sanitize_filename(&filename);

        // No invalid filesystem characters should remain
        prop_assert!(!sanitized.contains('/'));
        prop_assert!(!sanitized.contains('\\'));
        prop_assert!(!sanitized.contains(':'));
        prop_assert!(!sanitized.contains('*'));
        prop_assert!(!sanitized.contains('?'));
        prop_assert!(!sanitized.contains('"'));
        prop_assert!(!sanitized.contains('<'));
        prop_assert!(!sanitized.contains('>'));
        prop_assert!(!sanitized.contains('|'));
        prop_assert!(!sanitized.contains('\0'));

        // No leading/trailing dots or spaces
        if !sanitized.is_empty() && sanitized != "unnamed" {
            let first = sanitized.chars().next().unwrap();
            let last = sanitized.chars().last().unwrap();
            prop_assert!(first != '.' && first != ' ', "Should not start with dot or space");
            prop_assert!(last != '.' && last != ' ', "Should not end with dot or space");
        }

        // Length should be within limit
        prop_assert!(sanitized.len() <= 200, "Filename should not exceed 200 chars");
    }

    #[test]
    fn test_sanitize_filename_never_empty(
        filename in ".{0,50}"  // Any string up to 50 chars
    ) {
        let orchestrator = create_test_orchestrator();
        let sanitized = orchestrator.sanitize_filename(&filename);

        // Result should never be empty
        prop_assert!(!sanitized.is_empty(), "Sanitized filename should never be empty");
    }

    #[test]
    fn test_sanitize_filename_preserves_alphanumeric_content(
        // Generate filenames with only alphanumeric chars and underscore (no edge cases)
        filename in "[a-zA-Z][a-zA-Z0-9_]{0,50}"
    ) {
        let orchestrator = create_test_orchestrator();
        let sanitized = orchestrator.sanitize_filename(&filename);

        // For simple alphanumeric filenames, content should be preserved
        prop_assert_eq!(sanitized, filename, "Simple alphanumeric filenames should be unchanged");
    }
}
