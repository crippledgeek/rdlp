//! Homoglyph substitution tables.

/// A substitution table folding visually-identical characters onto one canonical form.
///
/// E.g. Cyrillic letters that render identically to Latin ones. Folding is
/// one-directional: several `from` characters may share a `to`, so the
/// original cannot be recovered from the folded text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HomoglyphTable(&'static [(char, char)]);

impl HomoglyphTable {
    /// Build a table from `(from, to)` pairs.
    #[must_use]
    pub const fn new(pairs: &'static [(char, char)]) -> Self {
        Self(pairs)
    }

    /// Replace every character in `s` found as a `from` in the table with its `to`.
    /// Characters not in the table pass through unchanged.
    #[must_use]
    pub fn fold(&self, s: &str) -> String {
        s.chars()
            .map(|c| {
                self.0
                    .iter()
                    .find(|&&(from, _)| from == c)
                    .map_or(c, |&(_, to)| to)
            })
            .collect()
    }
}

/// Cyrillic uppercase letters that render identically to Latin uppercase (А В С Е Н К М О Р Т Х).
pub const CYRILLIC_UPPERCASE_TO_LATIN: HomoglyphTable = HomoglyphTable::new(&[
    ('\u{0410}', 'A'), // А
    ('\u{0412}', 'B'), // В
    ('\u{0421}', 'C'), // С
    ('\u{0415}', 'E'), // Е
    ('\u{041D}', 'H'), // Н
    ('\u{041A}', 'K'), // К
    ('\u{041C}', 'M'), // М
    ('\u{041E}', 'O'), // О
    ('\u{0420}', 'P'), // Р
    ('\u{0422}', 'T'), // Т
    ('\u{0425}', 'X'), // Х
]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_uppercase_table_folds_every_pair() {
        let input = "\u{0410}\u{0412}\u{0421}\u{0415}\u{041D}\u{041A}\u{041C}\u{041E}\u{0420}\u{0422}\u{0425}";
        assert_eq!(CYRILLIC_UPPERCASE_TO_LATIN.fold(input), "ABCEHKMOPTX");
    }

    #[test]
    fn fold_leaves_other_chars_alone() {
        assert_eq!(
            CYRILLIC_UPPERCASE_TO_LATIN.fold("abc/=+\u{0430}"),
            "abc/=+\u{0430}"
        ); // lowercase Cyrillic a is NOT in the table
    }

    #[test]
    fn tables_compare_by_their_pairs() {
        // Value semantics: two tables built from the same pairs are equal
        // and a `Copy` of the constant is the constant.
        let copy = CYRILLIC_UPPERCASE_TO_LATIN;
        assert_eq!(copy, CYRILLIC_UPPERCASE_TO_LATIN);
        assert_ne!(HomoglyphTable::new(&[]), CYRILLIC_UPPERCASE_TO_LATIN);
        assert!(format!("{copy:?}").starts_with("HomoglyphTable("));
    }

    #[test]
    fn an_empty_table_is_identity() {
        assert_eq!(HomoglyphTable::new(&[]).fold("\u{0410}"), "\u{0410}");
    }
}
