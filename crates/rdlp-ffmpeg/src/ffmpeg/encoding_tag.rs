//! Encoder identification metadata for output files.
//!
//! Three helpers cover all postprocessing paths:
//! - [`set_encoding_tool`] — high-level API (safe `octx`)
//! - [`set_encoding_tool_ffi`] — raw FFI (`*mut AVFormatContext`)
//! - [`set_stream_encoder`] — per-stream encoder tag
//!
//! # Lint allowances
//!
//! - `clippy::expect_used`: `CString::new("static literal")` cannot fail (NUL-free
//!   compile-time constants). `octx.stream_mut(index)` is valid by construction:
//!   the stream was just added by `add_stream_copy` immediately before this call.

#![allow(clippy::expect_used)]

use std::ffi::CString;

/// Build the format-level `encoding_tool` metadata value.
///
/// ```ignore
/// encoding_tool_tag("libx264 + libfdk_aac")  // → "rdlp/0.1.0 (libx264 + libfdk_aac)"
/// encoding_tool_tag("remux")                  // → "rdlp/0.1.0 (remux)"
/// ```
#[must_use]
pub fn encoding_tool_tag(components: &str) -> String {
    format!("rdlp/{} ({components})", env!("CARGO_PKG_VERSION"))
}

/// Set the `encoding_tool` format-level tag on a high-level output context.
///
/// Unconditionally sets the tag. Use for stages that create content
/// (recode, audio extract, normalize).
pub fn set_encoding_tool(octx: &mut ffmpeg_the_third::format::context::Output, components: &str) {
    let mut meta = octx.metadata().to_owned();
    meta.set("encoding_tool", &encoding_tool_tag(components));
    octx.set_metadata(meta);
}

/// Set the `encoding_tool` format-level tag only if the output context
/// doesn't already have one (inherited from input metadata copy).
///
/// Use for pass-through stages (remux, merge, metadata embed, thumbnail
/// embed, salvage) that should preserve the primary stage's tag.
pub fn set_encoding_tool_if_missing(
    octx: &mut ffmpeg_the_third::format::context::Output,
    components: &str,
) {
    let has_tag = octx.metadata().get("encoding_tool").is_some();
    if !has_tag {
        set_encoding_tool(octx, components);
    }
}

/// Set the `encoding_tool` format-level tag on a raw FFI output context.
///
/// Unconditionally sets the tag. Use for stages that create content.
///
/// # Safety
///
/// `ofmt_ctx` must be a valid, non-null `AVFormatContext` pointer.
pub unsafe fn set_encoding_tool_ffi(
    ofmt_ctx: *mut ffmpeg_the_third::ffi::AVFormatContext,
    components: &str,
) {
    let key = CString::new("encoding_tool").expect("static string");
    let val = CString::new(encoding_tool_tag(components)).expect("no null bytes in version string");
    unsafe {
        ffmpeg_the_third::ffi::av_dict_set(
            &raw mut (*ofmt_ctx).metadata,
            key.as_ptr(),
            val.as_ptr(),
            0,
        );
    }
}

/// Set the `encoding_tool` tag on a raw FFI output context only if not
/// already present (inherited from input via `av_dict_copy`).
///
/// # Safety
///
/// `ofmt_ctx` must be a valid, non-null `AVFormatContext` pointer.
pub unsafe fn set_encoding_tool_ffi_if_missing(
    ofmt_ctx: *mut ffmpeg_the_third::ffi::AVFormatContext,
    components: &str,
) {
    let key = CString::new("encoding_tool").expect("static string");
    unsafe {
        let existing = ffmpeg_the_third::ffi::av_dict_get(
            (*ofmt_ctx).metadata,
            key.as_ptr(),
            std::ptr::null(),
            0,
        );
        if existing.is_null() {
            set_encoding_tool_ffi(ofmt_ctx, components);
        }
    }
}

/// Component string for the `encoding_tool` tag's audio segment.
///
/// `audio_copy` takes precedence over `audio_codec` — matches the documented
/// contract on `VideoConvertOptions` (`audio_copy` wins when both are set: a
/// stream copy happens even if `audio_codec` names an encoder). A resolved
/// codec name is only consulted when `audio_copy` is `false`; otherwise
/// `audio_copy` distinguishes a genuine stream copy (`"copy"`) from no audio
/// stream at all (`"none"`) — a video-only source must resolve here, not
/// stamp a false `"copy"`.
#[must_use]
pub const fn audio_tag_component(audio_copy: bool, audio_codec: Option<&str>) -> &str {
    if audio_copy {
        "copy"
    } else if let Some(codec) = audio_codec {
        codec
    } else {
        "none"
    }
}

/// Set the `encoder` per-stream tag on a high-level output stream.
pub fn set_stream_encoder(
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

    /// Pins the `encoding_tool` tag's audio component for the four
    /// `(audio_copy, audio_codec)` combinations, including the video-only
    /// case that used to stamp a false "copy" and the `(true, Some(_))`
    /// case where `audio_copy` must win over a resolved codec name (matches
    /// `VideoConvertOptions`'s documented precedence).
    #[test]
    fn audio_tag_component_matrix() {
        assert_eq!(audio_tag_component(false, None), "none");
        assert_eq!(audio_tag_component(true, None), "copy");
        assert_eq!(audio_tag_component(false, Some("libopus")), "libopus");
        // `audio_copy` wins even when a codec name is also present.
        assert_eq!(audio_tag_component(true, Some("libopus")), "copy");
    }
}
