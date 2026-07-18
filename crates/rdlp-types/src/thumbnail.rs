//! Thumbnail sidecar format policy.
//!
//! A single source of truth for the raster image extensions rdlp treats as
//! embeddable thumbnail sidecars. It is shared by two layers that MUST agree,
//! or a downloaded thumbnail is saved under one name and never found under
//! another (the discovery/download drift this const exists to prevent):
//!
//! - the **download layer** (`rdlp-api`), which chooses the on-disk extension
//!   for a fetched thumbnail — an extension not listed here is not trusted from
//!   the URL and the file is saved as `jpg`; and
//! - the **post-process discovery gate** (`rdlp-postprocess`), which locates the
//!   sidecar next to the media file before embedding it.

/// Raster image extensions rdlp discovers as thumbnail sidecars and preserves
/// when naming a downloaded thumbnail.
///
/// Every extension here is decodable by `rdlp_ffmpeg::FFmpegRunner::transcode_image`,
/// which normalizes any non-`jpg`/`png` sidecar to baseline JPEG before
/// MP4-family embedding (MP4/MOV/M4A/M4V have no muxer tag for `webp`, `gif`,
/// `bmp`, or `tiff`). `jpg`/`jpeg`/`png` embed without transcoding; the rest
/// flow through `transcode_image`. Matroska attaches the source codec natively
/// and needs no normalization.
///
/// `jpeg`/`jpg` and `tiff`/`tif` are the two standard extension spellings of the
/// same format, so both are listed — omitting an alias would save a `.tif` URL
/// as `.jpg` with TIFF bytes (a codec/extension mislabel).
///
/// `avif` is intentionally excluded: the still-image codecs above are
/// `FFmpeg`-core demuxers present in every build, whereas AVIF decoding requires
/// an AVIF *demuxer* that the current `FFmpeg` build does not ship (it can only
/// mux/write AVIF). An `.avif` sidecar would therefore fail to normalize and be
/// gracefully skipped; keeping it out of discovery avoids saving AVIF bytes
/// under an extension we cannot decode. Re-add it here once the linked `FFmpeg`
/// gains AVIF demux support.
pub const THUMBNAIL_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif"];

#[cfg(test)]
mod tests {
    use super::THUMBNAIL_EXTENSIONS;

    #[test]
    fn includes_the_widened_decodable_formats() {
        // The formats #521 added: FFmpeg-core still-image decoders that
        // `transcode_image` can normalize. Guards against a silent narrowing.
        for ext in ["gif", "bmp", "tiff", "tif"] {
            assert!(
                THUMBNAIL_EXTENSIONS.contains(&ext),
                "{ext} must be a discoverable thumbnail extension"
            );
        }
    }

    #[test]
    fn retains_the_original_baseline_formats() {
        for ext in ["jpg", "jpeg", "png", "webp"] {
            assert!(
                THUMBNAIL_EXTENSIONS.contains(&ext),
                "{ext} is a load-bearing baseline thumbnail extension"
            );
        }
    }

    /// avif has an `FFmpeg` muxer but no demuxer in the current build, so it
    /// cannot be decoded/normalized — it must NOT be advertised as discoverable
    /// or the download layer would save undecodable AVIF bytes to disk.
    #[test]
    fn excludes_undecodable_avif() {
        assert!(
            !THUMBNAIL_EXTENSIONS.contains(&"avif"),
            "avif is not decodable by the current FFmpeg build; keep it out of discovery"
        );
    }

    /// All entries are lowercase, bare (no leading dot) extensions — callers
    /// compare case-insensitively against a URL/path extension token that has no
    /// dot, so a stray `.jpg` or `JPG` entry here would silently never match.
    #[test]
    fn entries_are_lowercase_and_dotless() {
        for ext in THUMBNAIL_EXTENSIONS {
            assert!(
                !ext.starts_with('.') && ext.chars().all(|c| c.is_ascii_lowercase()),
                "extension {ext:?} must be a bare lowercase token"
            );
        }
    }
}
