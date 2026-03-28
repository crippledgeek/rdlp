//! Encoder identification metadata for output files.
//!
//! Three helpers cover all postprocessing paths:
//! - [`set_encoding_tool`] — high-level API (safe `octx`)
//! - [`set_encoding_tool_ffi`] — raw FFI (`*mut AVFormatContext`)
//! - [`set_stream_encoder`] — per-stream encoder tag

use std::ffi::CString;

/// Build the format-level `encoding_tool` metadata value.
///
/// ```ignore
/// encoding_tool_tag("libx264 + libfdk_aac")  // → "rdlp/0.1.0 (libx264 + libfdk_aac)"
/// encoding_tool_tag("remux")                  // → "rdlp/0.1.0 (remux)"
/// ```
pub(crate) fn encoding_tool_tag(components: &str) -> String {
    format!("rdlp/{} ({components})", env!("CARGO_PKG_VERSION"))
}

/// Set the `encoding_tool` format-level tag on a high-level output context.
pub(crate) fn set_encoding_tool(
    octx: &mut ffmpeg_the_third::format::context::Output,
    components: &str,
) {
    let mut meta = octx.metadata().to_owned();
    meta.set("encoding_tool", &encoding_tool_tag(components));
    octx.set_metadata(meta);
}

/// Set the `encoding_tool` format-level tag on a raw FFI output context.
///
/// # Safety
///
/// `ofmt_ctx` must be a valid, non-null `AVFormatContext` pointer.
pub(crate) unsafe fn set_encoding_tool_ffi(
    ofmt_ctx: *mut ffmpeg_the_third::ffi::AVFormatContext,
    components: &str,
) {
    let key = CString::new("encoding_tool").expect("static string");
    let val = CString::new(encoding_tool_tag(components)).expect("no null bytes in version string");
    unsafe {
        ffmpeg_the_third::ffi::av_dict_set(
            &mut (*ofmt_ctx).metadata,
            key.as_ptr(),
            val.as_ptr(),
            0,
        );
    }
}

/// Set the `encoder` per-stream tag on a high-level output stream.
pub(crate) fn set_stream_encoder(
    octx: &mut ffmpeg_the_third::format::context::Output,
    stream_index: usize,
    encoder_name: &str,
) {
    let mut dict = ffmpeg_the_third::Dictionary::new();
    dict.set("encoder", encoder_name);
    octx.stream_mut(stream_index)
        .expect("output stream exists")
        .set_metadata(dict);
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
