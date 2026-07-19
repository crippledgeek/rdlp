//! `METADATA_BLOCK_PICTURE` builder for Ogg/Opus cover art (RFC 9639 §8.8).
//!
//! `FFmpeg`'s Ogg muxer (`oggenc.c` `ogg_init()`) hard-rejects any stream
//! whose codec isn't Vorbis/Theora/Speex/FLAC/Opus/VP8 — so a thumbnail can
//! never ride an `ATTACHED_PIC` video stream in an Ogg/Opus container (#531),
//! unlike FLAC's native muxer, which does accept one and converts it
//! internally. The container-independent workaround is to carry the cover as
//! a base64-encoded FLAC `PICTURE` metadata block inside a `VorbisComment`
//! field named `METADATA_BLOCK_PICTURE` — the Ogg muxer writes arbitrary
//! metadata keys verbatim into the `VorbisComment` header, and readers
//! (including `FFmpeg` itself, on demux) recognize the field by name.
//!
//! Pure byte-level builder — no `FFmpeg` calls — so it is unit-tested
//! directly against a known-good byte vector rather than through a mux/demux
//! round trip.
//!
//! [`build_picture_block`] validates that the MIME type and image data each
//! fit the RFC 9639 `u32` length-prefix field itself, via `u32::try_from`,
//! rather than trusting that a caller upholds some size cap enforced
//! elsewhere: this module has more than one caller path (a streamed download
//! capped by `rdlp-api`'s `MAX_THUMBNAIL_BYTES`, but also
//! `ThumbnailStage::find_thumbnail`'s uncapped on-disk glob), so the only
//! guarantee this builder can honestly rely on is the one it checks itself.
//! See `image_convert.rs`'s `MAX_THUMBNAIL_DIMENSION` doc-comment for the same
//! "don't cite a distant invariant" discipline applied to decode dimensions.

use anyhow::Context as _;
use base64::Engine as _;

use rdlp_types::ThumbnailFormat;

/// FLAC/Vorbis picture type: "Cover (front)" (RFC 9639 §8.8, picture type table).
const PICTURE_TYPE_FRONT_COVER: u32 = 3;

/// Declared colour depth in bits per pixel (true colour). `FFmpeg`'s Ogg
/// demuxer derives the real value from the decoded image and ignores this
/// declaration entirely (verified empirically against this build); it is
/// written only for the benefit of readers that DO trust the declared value
/// (e.g. `mutagen`, some players).
const COLOR_DEPTH_TRUE_COLOR: u32 = 24;

/// Declared indexed-colour palette size. `0` means "not a palette image" —
/// correct for every format `rdlp` embeds this way (JPEG/PNG/GIF/BMP/TIFF/WebP
/// are all read and re-served as true-colour or grayscale raster data).
const INDEXED_COLORS_NONE: u32 = 0;

/// The IANA media type for a sniffed [`ThumbnailFormat`].
///
/// Matched exhaustively (no catch-all arm) so that a new [`ThumbnailFormat`]
/// variant fails to compile here, forcing a decision about its MIME type —
/// the same pattern `ThumbnailStage::write_covr_atom` uses for `mp4ameta`
/// (#525).
#[must_use]
pub(super) const fn mime_type(format: ThumbnailFormat) -> &'static str {
    match format {
        ThumbnailFormat::Jpeg => "image/jpeg",
        ThumbnailFormat::Png => "image/png",
        ThumbnailFormat::Gif => "image/gif",
        ThumbnailFormat::Bmp => "image/bmp",
        ThumbnailFormat::Tiff => "image/tiff",
        ThumbnailFormat::WebP => "image/webp",
    }
}

/// Validate that `len` fits the RFC 9639 `u32` length-prefix field, naming
/// `field` in the error so a rejection identifies which value (MIME type or
/// image data) was too large. This is the builder's own, locally-enforced
/// contract — see the module docs for why it must not rely on a cap enforced
/// by (only some of) its callers.
fn u32_length(len: usize, field: &str) -> anyhow::Result<u32> {
    u32::try_from(len)
        .with_context(|| format!("{field} length {len} exceeds the RFC 9639 u32 length-prefix"))
}

/// Build the raw (pre-base64) FLAC `PICTURE` metadata block (RFC 9639 §8.8).
///
/// Field order, all big-endian `u32`: picture type; MIME length + MIME
/// bytes; description length + description bytes (`rdlp` always writes an
/// empty description, but still emits its zero length); width; height;
/// colour depth; indexed-colour count; picture-data length + picture bytes.
///
/// # Errors
///
/// Returns an error if `mime` or `image_data` is too long to fit the `u32`
/// length-prefix field (see [`u32_length`]).
fn build_picture_block(
    mime: &str,
    width: u32,
    height: u32,
    image_data: &[u8],
) -> anyhow::Result<Vec<u8>> {
    const DESCRIPTION: &[u8] = b"";
    const FIELD_OVERHEAD: usize = 32; // 8 u32 fields, 4 bytes each.

    let mime_bytes = mime.as_bytes();
    let mime_len = u32_length(mime_bytes.len(), "MIME type")?;
    let image_len = u32_length(image_data.len(), "image data")?;

    let mut block = Vec::with_capacity(FIELD_OVERHEAD + mime_bytes.len() + image_data.len());

    block.extend_from_slice(&PICTURE_TYPE_FRONT_COVER.to_be_bytes());

    block.extend_from_slice(&mime_len.to_be_bytes());
    block.extend_from_slice(mime_bytes);

    let description_len = u32_length(DESCRIPTION.len(), "description")?;
    block.extend_from_slice(&description_len.to_be_bytes());
    block.extend_from_slice(DESCRIPTION);

    block.extend_from_slice(&width.to_be_bytes());
    block.extend_from_slice(&height.to_be_bytes());
    block.extend_from_slice(&COLOR_DEPTH_TRUE_COLOR.to_be_bytes());
    block.extend_from_slice(&INDEXED_COLORS_NONE.to_be_bytes());

    block.extend_from_slice(&image_len.to_be_bytes());
    block.extend_from_slice(image_data);

    Ok(block)
}

/// Build the `METADATA_BLOCK_PICTURE` `VorbisComment` value: the FLAC
/// `PICTURE` block above, base64-encoded per RFC 4648 §4 (standard alphabet,
/// `=`-padded, no line breaks).
///
/// # Errors
///
/// Returns an error if `mime` or `image_data` is too long to fit the RFC 9639
/// `u32` length-prefix field (see [`build_picture_block`]).
pub(super) fn build_metadata_block_picture(
    mime: &str,
    width: u32,
    height: u32,
    image_data: &[u8],
) -> anyhow::Result<String> {
    let block = build_picture_block(mime, width, height, image_data)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(block))
}

#[cfg(test)]
mod tests {
    use super::{build_metadata_block_picture, build_picture_block, mime_type, u32_length};
    use base64::Engine as _;
    use rdlp_types::ThumbnailFormat;

    /// Pins the exact big-endian field layout against a known-good byte
    /// vector: picture type 3, mime "image/png" (9 bytes), empty description
    /// (length 0, no bytes), 10x20, depth 24, indexed 0, then 4 bytes of
    /// "fake" image data.
    #[test]
    fn picture_block_matches_known_good_byte_layout() {
        let image_data = b"DATA";
        let block =
            build_picture_block("image/png", 10, 20, image_data).expect("valid-length input");

        let mut expected = Vec::new();
        expected.extend_from_slice(&3u32.to_be_bytes()); // picture type
        expected.extend_from_slice(&9u32.to_be_bytes()); // mime length
        expected.extend_from_slice(b"image/png");
        expected.extend_from_slice(&0u32.to_be_bytes()); // description length
        expected.extend_from_slice(&10u32.to_be_bytes()); // width
        expected.extend_from_slice(&20u32.to_be_bytes()); // height
        expected.extend_from_slice(&24u32.to_be_bytes()); // color depth
        expected.extend_from_slice(&0u32.to_be_bytes()); // indexed colors
        expected.extend_from_slice(&4u32.to_be_bytes()); // data length
        expected.extend_from_slice(image_data);

        assert_eq!(block, expected);
    }

    /// Boundary pin: an empty description must still emit its `u32` length
    /// of `0` — a description-handling bug could skip the length field
    /// entirely when the description is empty, corrupting every subsequent
    /// offset.
    #[test]
    fn empty_description_still_emits_its_zero_length_field() {
        let block = build_picture_block("image/jpeg", 1, 1, b"X").expect("valid-length input");

        // Offset 0..4 picture type, 4..8 mime length, 8..8+10 mime bytes
        // ("image/jpeg" is 10 bytes), then the description length u32.
        let mime_len_offset = 4;
        let mime_len = u32::from_be_bytes(
            block[mime_len_offset..mime_len_offset + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            mime_len, 10,
            "mime length field must match \"image/jpeg\".len()"
        );

        let description_len_offset = mime_len_offset + 4 + mime_len as usize;
        let description_len = u32::from_be_bytes(
            block[description_len_offset..description_len_offset + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            description_len, 0,
            "empty description must still emit an explicit zero-length field"
        );

        // Width immediately follows the (zero-length, zero-byte) description.
        let width_offset = description_len_offset + 4;
        let width = u32::from_be_bytes(block[width_offset..width_offset + 4].try_into().unwrap());
        assert_eq!(width, 1);
    }

    /// Data-length field must equal the actual embedded byte count, not the
    /// declared image dimensions or any other unrelated value.
    #[test]
    fn data_length_field_matches_actual_image_bytes() {
        let image_data = b"0123456789";
        let block =
            build_picture_block("image/png", 999, 999, image_data).expect("valid-length input");

        let data_len_offset = block.len() - image_data.len() - 4;
        let data_len = u32::from_be_bytes(
            block[data_len_offset..data_len_offset + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(data_len, image_data.len() as u32);
        assert_eq!(&block[data_len_offset + 4..], image_data);
    }

    /// The public entry point base64-encodes the exact same block with the
    /// standard, padded RFC 4648 alphabet — decoding it must round-trip.
    #[test]
    fn metadata_block_picture_base64_round_trips_to_the_same_block() {
        let image_data = b"round-trip-bytes";
        let encoded = build_metadata_block_picture("image/jpeg", 4, 8, image_data)
            .expect("valid-length input");

        assert!(
            !encoded.contains('\n') && !encoded.contains('\r'),
            "base64 value must not contain line breaks"
        );

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("must be valid standard-alphabet base64");
        assert_eq!(
            decoded,
            build_picture_block("image/jpeg", 4, 8, image_data).expect("valid-length input")
        );
    }

    /// Boundary pin: a length of exactly `u32::MAX` still fits the RFC 9639
    /// length-prefix field and must be accepted.
    #[test]
    fn u32_length_accepts_the_u32_max_boundary() {
        assert_eq!(
            u32_length(u32::MAX as usize, "image data").expect("u32::MAX must fit a u32 field"),
            u32::MAX
        );
    }

    /// Boundary pin: `u32::MAX + 1` does not fit the RFC 9639 length-prefix
    /// field and must be rejected rather than silently truncated — the exact
    /// failure mode this builder now defends against instead of trusting a
    /// distant size cap (see module docs).
    ///
    /// 64-bit-only: on a 32-bit target `usize` cannot represent `u32::MAX + 1`
    /// at all (the literal itself overflows `usize`), so the boundary this
    /// test pins does not exist there.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn u32_length_rejects_one_past_the_u32_max_boundary() {
        let oversized = u32::MAX as usize + 1;
        let err = u32_length(oversized, "image data").expect_err("must reject oversize length");
        assert!(
            err.to_string().contains("image data"),
            "error must name the offending field"
        );
    }

    #[test]
    fn mime_type_covers_every_thumbnail_format() {
        assert_eq!(mime_type(ThumbnailFormat::Jpeg), "image/jpeg");
        assert_eq!(mime_type(ThumbnailFormat::Png), "image/png");
        assert_eq!(mime_type(ThumbnailFormat::Gif), "image/gif");
        assert_eq!(mime_type(ThumbnailFormat::Bmp), "image/bmp");
        assert_eq!(mime_type(ThumbnailFormat::Tiff), "image/tiff");
        assert_eq!(mime_type(ThumbnailFormat::WebP), "image/webp");
    }
}
