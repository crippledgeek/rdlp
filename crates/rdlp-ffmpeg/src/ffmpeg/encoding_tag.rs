//! Encoder identification metadata for output files.

/// Build the format-level `encoding_tool` metadata value.
///
/// # Examples
///
/// ```ignore
/// encoding_tool_tag("libx264 + libfdk_aac")  // → "rdlp/0.1.0 (libx264 + libfdk_aac)"
/// encoding_tool_tag("remux")                  // → "rdlp/0.1.0 (remux)"
/// ```
pub(crate) fn encoding_tool_tag(components: &str) -> String {
    format!("rdlp/{} ({components})", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoding_tool_tag_format() {
        let tag = encoding_tool_tag("libx264 + libfdk_aac");
        assert!(tag.starts_with("rdlp/"), "tag: {tag}");
        assert!(tag.contains("libx264 + libfdk_aac"), "tag: {tag}");
        assert!(tag.ends_with(')'), "tag: {tag}");
    }

    #[test]
    fn test_encoding_tool_tag_remux() {
        let tag = encoding_tool_tag("remux");
        assert!(tag.starts_with("rdlp/"), "tag: {tag}");
        assert!(tag.contains("(remux)"), "tag: {tag}");
    }

    #[test]
    fn test_encoding_tool_tag_single_component() {
        let tag = encoding_tool_tag("libfdk_aac");
        assert!(tag.contains("(libfdk_aac)"), "tag: {tag}");
    }
}
