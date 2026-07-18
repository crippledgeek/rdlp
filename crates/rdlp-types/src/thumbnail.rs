//! Thumbnail sidecar format policy.
//!
//! A single source of truth for the raster image extensions rdlp treats as
//! embeddable thumbnail sidecars. It is shared by two layers that MUST agree,
//! or a downloaded thumbnail is saved under one name and never found under
//! another (the discovery/download drift this const exists to prevent):
//!
//! - the **download layer** (`rdlp-api`), which chooses the on-disk extension
//!   for a fetched thumbnail. Since #525 that name comes from the bytes via
//!   [`sniff_thumbnail_format`], because a CDN may serve one format from
//!   another format's URL path; the URL's extension is only a fallback for
//!   unrecognized content, and one not listed here still degrades to `jpg`; and
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

/// Identify a thumbnail's real image format from its leading bytes.
///
/// A URL's extension is a claim, not a fact: a CDN may serve WebP bytes from a
/// `.jpg` path (the defect in #525). Naming a sidecar from that claim writes
/// one format under another format's extension, and every later stage that
/// treats the extension as a proxy for the codec — discovery, the embed
/// normalization gate, the `covr` atom's image type — then makes the wrong
/// call. Content is the only trustworthy source, so callers that have the
/// bytes should name the file from this rather than from the URL.
///
/// Returns the identified [`ThumbnailFormat`], or `None` when the bytes match
/// no known signature — including a short or empty buffer. `None` means
/// "unrecognized", never "definitely not an image": callers should fall back to
/// their previous behavior rather than discarding the file.
///
/// Matching is table-driven over [`SIGNATURES`]; see [`SignatureFragment`] for
/// why patterns carry an offset rather than being leading-byte prefixes.
#[must_use]
pub fn sniff_thumbnail_format(bytes: &[u8]) -> Option<ThumbnailFormat> {
    SIGNATURES
        .iter()
        .find(|(fragments, _)| fragments.iter().all(|fragment| fragment.matches(bytes)))
        .map(|(_, format)| *format)
}

/// One contiguous byte pattern that must appear at a fixed offset.
///
/// Signatures are expressed as fragments rather than a single leading pattern
/// because not every format announces itself in its first bytes: WebP's `WEBP`
/// marker sits at offset 8, behind the RIFF chunk length, and BMP's stronger
/// form checks reserved fields past its 2-byte tag. Modelling "pattern at
/// offset" once keeps those cases as data in [`SIGNATURES`] instead of
/// bespoke control flow per format, and any future format with a marker behind
/// a header (AVIF's `ftyp` at offset 4, say) is then one more table row.
#[derive(Debug, Clone, Copy)]
struct SignatureFragment {
    /// Byte offset the pattern must appear at.
    offset: usize,
    /// Bytes that must match exactly at `offset`.
    pattern: &'static [u8],
}

impl SignatureFragment {
    /// Declare a fragment; `const` so [`SIGNATURES`] stays a compile-time table.
    const fn at(offset: usize, pattern: &'static [u8]) -> Self {
        Self { offset, pattern }
    }

    /// Whether `bytes` carries this fragment. A buffer too short to contain the
    /// window simply does not match — short input is never an error.
    fn matches(self, bytes: &[u8]) -> bool {
        bytes
            .get(self.offset..self.offset.saturating_add(self.pattern.len()))
            .is_some_and(|window| window == self.pattern)
    }
}

/// Byte signatures identifying each format; ALL fragments of an entry must
/// match. Verified against files produced by the linked `FFmpeg` build.
const SIGNATURES: &[(&[SignatureFragment], ThumbnailFormat)] = &[
    (
        &[SignatureFragment::at(0, &[0xFF, 0xD8, 0xFF])],
        ThumbnailFormat::Jpeg,
    ),
    (
        &[SignatureFragment::at(
            0,
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )],
        ThumbnailFormat::Png,
    ),
    (&[SignatureFragment::at(0, b"GIF87a")], ThumbnailFormat::Gif),
    (&[SignatureFragment::at(0, b"GIF89a")], ThumbnailFormat::Gif),
    // BMP's tag is only two bytes, making it the most collision-prone entry —
    // any payload beginning "BM" would otherwise qualify. `BITMAPFILEHEADER`
    // requires bfReserved1/bfReserved2 (offsets 6..10) to be zero, so match
    // those too, as libmagic does.
    (
        &[
            SignatureFragment::at(0, b"BM"),
            SignatureFragment::at(6, &[0x00, 0x00, 0x00, 0x00]),
        ],
        ThumbnailFormat::Bmp,
    ),
    // TIFF is byte-order tagged. Version 42 is classic TIFF; BigTIFF
    // (version 43) is deliberately out of scope — CDNs do not serve it, and an
    // unrecognized thumbnail degrades safely to normalization.
    (
        &[SignatureFragment::at(0, &[0x49, 0x49, 0x2A, 0x00])],
        ThumbnailFormat::Tiff,
    ),
    (
        &[SignatureFragment::at(0, &[0x4D, 0x4D, 0x00, 0x2A])],
        ThumbnailFormat::Tiff,
    ),
    // WebP is a RIFF container: "RIFF" <u32 length> "WEBP". The form type is
    // what distinguishes it from other RIFF payloads such as WAV or AVI, and
    // the intervening length field is why it needs a second fragment rather
    // than one contiguous pattern.
    (
        &[
            SignatureFragment::at(0, b"RIFF"),
            SignatureFragment::at(8, b"WEBP"),
        ],
        ThumbnailFormat::WebP,
    ),
];

/// A thumbnail image format rdlp can identify from content.
///
/// Distinct from [`THUMBNAIL_EXTENSIONS`], which is a list of *filename*
/// spellings to probe on disk (including the `jpeg`/`jpg` and `tif`/`tiff`
/// aliases). This is the set of actual formats, one variant per format, so a
/// caller deciding what a file *is* matches exhaustively and the compiler
/// tracks any future addition — the coupling a `debug_assert` on a string
/// could not enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailFormat {
    /// JPEG (`FF D8 FF`).
    Jpeg,
    /// PNG (8-byte signature, ISO 15948 §5).
    Png,
    /// GIF, either `GIF87a` or `GIF89a`.
    Gif,
    /// Windows bitmap (`BM`).
    Bmp,
    /// TIFF, either byte order.
    Tiff,
    /// WebP (RIFF container with a `WEBP` form type).
    WebP,
}

impl ThumbnailFormat {
    /// The canonical extension rdlp saves this format under.
    ///
    /// One spelling per format: `jpg` (not `jpeg`) and `tiff` (not `tif`).
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::WebP => "webp",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{THUMBNAIL_EXTENSIONS, ThumbnailFormat, sniff_thumbnail_format};

    // Byte signatures below were verified empirically against files produced by
    // the linked FFmpeg build, not from recall.

    #[test]
    fn sniffs_jpeg() {
        assert_eq!(
            sniff_thumbnail_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]),
            Some(ThumbnailFormat::Jpeg)
        );
    }

    #[test]
    fn sniffs_png() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(sniff_thumbnail_format(&png), Some(ThumbnailFormat::Png));
    }

    /// The bug behind #525: a CDN serving WebP bytes under a `.jpg` URL. The
    /// `WEBP` marker sits at offset 8, AFTER the 4-byte RIFF chunk length, so a
    /// contiguous `RIFF....WEBP` match would fail.
    #[test]
    fn sniffs_webp_despite_split_marker() {
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0x4C, 0x00, 0x00, 0x00]); // chunk length
        webp.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(sniff_thumbnail_format(&webp), Some(ThumbnailFormat::WebP));
    }

    /// A RIFF container that is not WebP (e.g. WAV) must not be claimed.
    #[test]
    fn rejects_non_webp_riff() {
        let mut wav = Vec::from(*b"RIFF");
        wav.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        wav.extend_from_slice(b"WAVEfmt ");
        assert_eq!(sniff_thumbnail_format(&wav), None);
    }

    #[test]
    fn sniffs_both_gif_versions() {
        assert_eq!(
            sniff_thumbnail_format(b"GIF89a\x20\x00"),
            Some(ThumbnailFormat::Gif)
        );
        assert_eq!(
            sniff_thumbnail_format(b"GIF87a\x20\x00"),
            Some(ThumbnailFormat::Gif)
        );
    }

    #[test]
    fn sniffs_bmp() {
        assert_eq!(
            sniff_thumbnail_format(b"BM\x36\x0c\x00\x00\x00\x00\x00\x00"),
            Some(ThumbnailFormat::Bmp)
        );
    }

    /// BMP's tag is only two bytes, so it is the entry most likely to claim
    /// unrelated data. `BITMAPFILEHEADER` requires the reserved fields at
    /// offsets 6..10 to be zero, and matching them rejects payloads that merely
    /// happen to start with "BM".
    #[test]
    fn rejects_bm_prefixed_data_with_nonzero_reserved_fields() {
        assert_eq!(
            sniff_thumbnail_format(b"BMOKAY-this-is-just-text"),
            None,
            "text beginning \"BM\" must not be claimed as a bitmap"
        );
    }

    /// TIFF is byte-order tagged: `II*\0` little-endian, `MM\0*` big-endian.
    /// Both are valid and both must be recognized.
    #[test]
    fn sniffs_tiff_in_both_byte_orders() {
        assert_eq!(
            sniff_thumbnail_format(b"II\x2A\x00rest"),
            Some(ThumbnailFormat::Tiff)
        );
        assert_eq!(
            sniff_thumbnail_format(b"MM\x00\x2Arest"),
            Some(ThumbnailFormat::Tiff)
        );
    }

    #[test]
    fn returns_none_for_unrecognized_or_short_input() {
        assert_eq!(sniff_thumbnail_format(b""), None);
        assert_eq!(sniff_thumbnail_format(b"\xFF"), None);
        assert_eq!(sniff_thumbnail_format(b"RIFF"), None); // truncated
        assert_eq!(sniff_thumbnail_format(b"<!DOCTYPE html>"), None);
        assert_eq!(sniff_thumbnail_format(b"not an image at all"), None);
    }

    /// Load-bearing invariant: anything the sniffer can name must also be a
    /// discoverable sidecar extension, or the download layer would save a file
    /// under a name the post-process discovery gate never looks for.
    #[test]
    fn every_sniffable_format_is_discoverable() {
        let samples: &[(ThumbnailFormat, &[u8])] = &[
            (ThumbnailFormat::Jpeg, &[0xFF, 0xD8, 0xFF, 0xE0]),
            (
                ThumbnailFormat::Png,
                &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            ),
            (ThumbnailFormat::Gif, b"GIF89a\x00\x00"),
            (ThumbnailFormat::Bmp, b"BM\x00\x00\x00\x00\x00\x00\x00\x00"),
            (ThumbnailFormat::Tiff, b"II\x2A\x00"),
            (ThumbnailFormat::WebP, b"RIFF\x00\x00\x00\x00WEBP"),
        ];
        for (expected, bytes) in samples {
            let sniffed = sniff_thumbnail_format(bytes)
                .unwrap_or_else(|| panic!("{expected:?} sample must be sniffable"));
            assert_eq!(sniffed, *expected);
            assert!(
                THUMBNAIL_EXTENSIONS.contains(&sniffed.extension()),
                "sniffer returned {:?}, whose extension discovery does not look for",
                sniffed.extension()
            );
        }
    }

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
