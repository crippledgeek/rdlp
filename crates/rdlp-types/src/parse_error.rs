//! Typed error for enum `FromStr` parsing failures

use std::fmt;

/// Error returned when parsing a string into a typed enum fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseEnumError {
    /// The type name being parsed (e.g., "ContainerFormat").
    pub type_name: &'static str,
    /// The invalid input value.
    pub input: String,
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported {}: {}", self.type_name, self.input)
    }
}

impl std::error::Error for ParseEnumError {}
