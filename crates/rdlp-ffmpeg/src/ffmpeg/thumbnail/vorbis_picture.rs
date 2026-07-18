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
//! # Lint allowances
//!
//! - `clippy::cast_possible_truncation`: `mime`/`image_data` lengths are cast
//!   to `u32` for the RFC 9639 wire format. Thumbnails are capped at 20 MiB
//!   by the download layer (`MAX_THUMBNAIL_BYTES`,
//!   `rdlp-api/src/orchestrator/thumbnail.rs`), far below `u32::MAX`.

#![allow(clippy::cast_possible_truncation)]

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

/// Build the raw (pre-base64) FLAC `PICTURE` metadata block (RFC 9639 §8.8).
///
/// Field order, all big-endian `u32`: picture type; MIME length + MIME
/// bytes; description length + description bytes (`rdlp` always writes an
/// empty description, but still emits its zero length); width; height;
/// colour depth; indexed-colour count; picture-data length + picture bytes.
fn build_picture_block(mime: &str, width: u32, height: u32, image_data: &[u8]) -> Vec<u8> {
    const DESCRIPTION: &[u8] = b"";
    const FIELD_OVERHEAD: usize = 32; // 8 u32 fields, 4 bytes each.

    let mime_bytes = mime.as_bytes();
    let mut block = Vec::with_capacity(FIELD_OVERHEAD + mime_bytes.len() + image_data.len());

    block.extend_from_slice(&PICTURE_TYPE_FRONT_COVER.to_be_bytes());

    block.extend_from_slice(&(mime_bytes.len() as u32).to_be_bytes());
    block.extend_from_slice(mime_bytes);

    block.extend_from_slice(&(DESCRIPTION.len() as u32).to_be_bytes());
    block.extend_from_slice(DESCRIPTION);

    block.extend_from_slice(&width.to_be_bytes());
    block.extend_from_slice(&height.to_be_bytes());
    block.extend_from_slice(&COLOR_DEPTH_TRUE_COLOR.to_be_bytes());
    block.extend_from_slice(&INDEXED_COLORS_NONE.to_be_bytes());

    block.extend_from_slice(&(image_data.len() as u32).to_be_bytes());
    block.extend_from_slice(image_data);

    block
}

/// Build the `METADATA_BLOCK_PICTURE` `VorbisComment` value: the FLAC
/// `PICTURE` block above, base64-encoded per RFC 4648 §4 (standard alphabet,
/// `=`-padded, no line breaks).
pub(super) fn build_metadata_block_picture(
    mime: &str,
    width: u32,
    height: u32,
    image_data: &[u8],
) -> String {
    let block = build_picture_block(mime, width, height, image_data);
    base64::engine::general_purpose::STANDARD.encode(block)
}

#[cfg(test)]
mod tests {
    use super::{build_metadata_block_picture, build_picture_block, mime_type};
    use base64::Engine as _;
    use rdlp_types::ThumbnailFormat;

    /// Pins the exact big-endian field layout against a known-good byte
    /// vector: picture type 3, mime "image/png" (9 bytes), empty description
    /// (length 0, no bytes), 10x20, depth 24, indexed 0, then 4 bytes of
    /// "fake" image data.
    #[test]
    fn picture_block_matches_known_good_byte_layout() {
        let image_data = b"DATA";
        let block = build_picture_block("image/png", 10, 20, image_data);

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
        let block = build_picture_block("image/jpeg", 1, 1, b"X");

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
        let block = build_picture_block("image/png", 999, 999, image_data);

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
        let encoded = build_metadata_block_picture("image/jpeg", 4, 8, image_data);

        assert!(
            !encoded.contains('\n') && !encoded.contains('\r'),
            "base64 value must not contain line breaks"
        );

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("must be valid standard-alphabet base64");
        assert_eq!(decoded, build_picture_block("image/jpeg", 4, 8, image_data));
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
