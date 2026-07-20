//! Typed error for enum `FromStr` parsing failures

use std::fmt;

/// Error returned when parsing a string into a typed enum fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    /// Human-readable name of the vocabulary being parsed — prose, NOT a Rust
    /// type name. Rendered as `unsupported {type_name}: {input}`, so callers
    /// pass e.g. "container format", giving
    /// `unsupported container format: mkvv`.
    type_name: &'static str,
    /// The invalid input value, with control characters already stripped.
    ///
    /// Private so [`ParseEnumError::new`] is the only way to build one — the
    /// sanitisation is then an invariant of the type rather than a convention
    /// every future call site has to remember.
    input: String,
}

impl ParseEnumError {
    /// Builds the error, stripping control characters from `input`.
    ///
    /// The input is attacker-influencable in the sense that matters here: it is
    /// echoed back to a terminal, and a config file can be copied from a dotfile
    /// repo or a gist. A rejected value like `mkv\u{1b}[2J` would otherwise
    /// carry a raw ANSI escape straight to stderr and clear the user's screen
    /// (CWE-150). `toml`'s own source-line echo is safe because it reproduces
    /// the file's *escaped* text; this message carries the decoded value, so it
    /// is this type's job to neutralise it.
    ///
    /// Sanitised at construction rather than at each display site, because this
    /// type exists only to be shown to a user — there is no consumer that wants
    /// the raw control bytes, and a boundary-by-boundary fix would need every
    /// future call site to remember.
    #[must_use]
    pub fn new(type_name: &'static str, input: &str) -> Self {
        Self {
            type_name,
            input: input.chars().filter(|c| !c.is_control()).collect(),
        }
    }
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported {}: {}", self.type_name, self.input)
    }
}

impl std::error::Error for ParseEnumError {}

#[cfg(test)]
mod tests {
    use super::ParseEnumError;

    #[test]
    fn display_names_the_type_and_the_input() {
        let e = ParseEnumError::new("container format", "mkvv");
        assert_eq!(e.to_string(), "unsupported container format: mkvv");
    }

    /// A rejected config value must not carry terminal control sequences into
    /// stderr (CWE-150).
    #[test]
    fn control_characters_are_stripped_from_the_input() {
        let e = ParseEnumError::new("container format", "mkv\u{1b}[2J\u{7}\n");
        assert_eq!(
            e.to_string(),
            "unsupported container format: mkv[2J",
            "escape, bell and newline must not survive into the message"
        );
        assert!(
            !e.to_string().chars().any(char::is_control),
            "no control character may reach the terminal"
        );
    }

    #[test]
    fn ordinary_input_is_preserved_verbatim() {
        for input in ["libOpus", "e-ac-3", "3gp", "wavpack"] {
            let e = ParseEnumError::new("audio format", input);
            assert!(
                e.to_string().ends_with(input),
                "{input} must survive sanitisation unchanged"
            );
        }
    }
}
