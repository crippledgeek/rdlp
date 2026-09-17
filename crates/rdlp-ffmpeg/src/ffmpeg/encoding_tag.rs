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

/// Component string for one stream of the `encoding_tool` tag.
///
/// The tag names the *tool* that produced each stream (Matroska
/// `WritingApp`, MP4 `©too`), so a stream copy says `copy` — never the
/// encoder that might have been used, and never the codec that happens to
/// be inside (`FFmpeg` itself writes no `encoder` tag for a copied stream:
/// `fftools/ffmpeg_mux_init.c`, `set_encoder_id` is only reached when
/// `ost->enc` is set). `copy` takes precedence over `encoder` — matches the
/// documented contract on `VideoConvertOptions` (`audio_copy` wins when both
/// are set). A resolved encoder name is only consulted when `copy` is
/// `false`; otherwise `copy` distinguishes a genuine stream copy (`"copy"`)
/// from no such stream at all (`"none"`) — a video-only source must resolve
/// here, not stamp a false `"copy"`.
#[must_use]
pub const fn stream_tag_component(copy: bool, encoder: Option<&str>) -> &str {
    if copy {
        "copy"
    } else if let Some(encoder) = encoder {
        encoder
    } else {
        "none"
    }
}

/// The `encoding_tool` components of a video conversion.
///
/// The single place the `"<video> + <audio>"` pair is assembled, so the tag
/// written into the file and the tag carried to downstream stages cannot
/// disagree (#626: they did — a remux stamped `libx264` for a video it never
/// encoded). Each `(copy, encoder)` pair follows [`stream_tag_component`];
/// the transcode path passes the encoder it actually opened.
#[must_use]
pub fn encoding_tool_components(
    video: (bool, Option<&str>),
    audio: (bool, Option<&str>),
) -> String {
    format!(
        "{} + {}",
        stream_tag_component(video.0, video.1),
        stream_tag_component(audio.0, audio.1)
    )
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
    fn stream_tag_component_matrix() {
        assert_eq!(stream_tag_component(false, None), "none");
        assert_eq!(stream_tag_component(true, None), "copy");
        assert_eq!(stream_tag_component(false, Some("libopus")), "libopus");
        // `copy` wins even when an encoder name is also present.
        assert_eq!(stream_tag_component(true, Some("libopus")), "copy");
    }

    /// #626: a remux stream-copies the video, so `video_codec` is `None` and
    /// the old `unwrap_or("libx264")` fallback claimed an encoder that never
    /// ran, for a codec that might not even be H.264. The tag names tools,
    /// not codecs (Matroska `WritingApp`, MP4 `©too`; `FFmpeg` writes no
    /// `encoder` for a copied stream — `ffmpeg_mux_init.c:1440`), so a copy
    /// says `copy`, exactly as the audio half already does.
    #[test]
    fn encoding_tool_components_remux_says_copy_not_libx264() {
        // A remux: video copied, audio copied.
        assert_eq!(
            encoding_tool_components((true, None), (true, None)),
            "copy + copy"
        );
        // A remux of a video-only source.
        assert_eq!(
            encoding_tool_components((true, None), (false, None)),
            "copy + none"
        );
        // A transcode names the encoders that ran.
        assert_eq!(
            encoding_tool_components((false, Some("libvpx-vp9")), (false, Some("libopus"))),
            "libvpx-vp9 + libopus"
        );
    }
}
