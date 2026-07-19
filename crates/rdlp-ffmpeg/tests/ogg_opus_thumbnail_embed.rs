//! Regression test for #531: thumbnail embed into Ogg/Opus.
//!
//! `ogg`/`opus` are listed in `SUPPORTED_CONTAINERS`
//! (`rdlp-postprocess/src/pipeline/stages/thumbnail.rs`), but embedding via
//! `FFmpegRunner::embed_thumbnail` failed with `Unsupported codec id in
//! stream 1` — `FFmpeg`'s Ogg muxer (`oggenc.c` `ogg_init()`) hard-rejects
//! any stream whose codec isn't Vorbis/Theora/Speex/FLAC/Opus/VP8, so the
//! cover art can never ride an `ATTACHED_PIC` video stream in these
//! containers (unlike FLAC, which does accept one).
//!
//! The fix carries the cover as a base64 FLAC `PICTURE` block in a
//! `METADATA_BLOCK_PICTURE` `VorbisComment` field instead of a stream; on
//! read, `FFmpeg` re-exposes it as an `attached_pic` video stream.
//!
//! Self-skips when the `ffmpeg` CLI is absent (used only to build fixtures),
//! mirroring `container_accepts_image_codec.rs` — unless
//! [`REQUIRE_FFMPEG_ENV`] is set, in which case a missing CLI is a hard
//! failure rather than a silent no-assertions-made green run.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use rdlp_ffmpeg::FFmpegRunner;

/// Env var that turns a missing-`ffmpeg`-CLI skip into a hard test failure —
/// set by CI (or a local strict run) to guarantee this module's #531
/// regression coverage actually executed at least once, rather than
/// silently reporting green with zero assertions made.
const REQUIRE_FFMPEG_ENV: &str = "RDLP_REQUIRE_FFMPEG_TESTS";

/// The still-cover fixture's fixed pixel dimensions (see [`make_cover`]) —
/// the "real" dimensions the width/height pin below checks the declared
/// `METADATA_BLOCK_PICTURE` fields against.
const COVER_DIMENSION_PX: u32 = 32;

fn ffmpeg_cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Skip the calling test unless [`REQUIRE_FFMPEG_ENV`] is set, in which case
/// a missing `ffmpeg` CLI is a hard failure instead of a silent skip.
fn skip_or_panic_without_ffmpeg_cli(test_name: &str) {
    assert!(
        std::env::var_os(REQUIRE_FFMPEG_ENV).is_none(),
        "{REQUIRE_FFMPEG_ENV} is set but the ffmpeg CLI is unavailable — \
         {test_name} cannot run its #531 regression coverage"
    );
    eprintln!("skipping {test_name}: ffmpeg CLI not available");
}

/// Render a 1-frame solid-color still cover image at [`COVER_DIMENSION_PX`].
fn make_cover(dir: &Path, format: &str) -> PathBuf {
    let path = dir.join(format!("cover.{format}"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=red:s={COVER_DIMENSION_PX}x{COVER_DIMENSION_PX}:d=1"),
        ])
        .args(["-frames:v", "1"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build {format} cover fixture");
    path
}

/// Build a 1-second audio-only fixture in `container` using `encoder`.
fn make_audio(dir: &Path, container: &str, encoder: &str) -> PathBuf {
    let path = dir.join(format!("audio.{container}"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-c:a", encoder])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(
        status.success(),
        "failed to build {container} audio fixture"
    );
    path
}

/// Demux `media`'s attached-picture video stream to `dst` via the `ffmpeg` CLI.
fn extract_cover(media: &Path, dst: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-i"])
        .arg(media)
        .args(["-an", "-map", "0:v", "-c", "copy"])
        .arg(dst)
        .status()
        .expect("spawn ffmpeg");
    assert!(
        status.success(),
        "failed to extract cover from {}",
        media.display()
    );
}

/// Read `media`'s raw `METADATA_BLOCK_PICTURE` `VorbisComment` tag directly
/// from the file's bytes and base64-decode it back to the FLAC `PICTURE`
/// block — independent of both `vorbis_picture::build_picture_block` (so
/// this doesn't just re-check the builder against itself) AND of `FFmpeg`'s
/// own demuxer (`ffprobe`/`ffmpeg -i` cannot be used here: `FFmpeg`'s Ogg
/// demuxer consumes this exact tag on read and re-exposes it as a decoded
/// `attached_pic` video stream instead of leaving it in `format_tags`,
/// verified empirically against this build — see `vorbis_picture.rs`'s
/// module docs on `FFmpeg` deriving dimensions from the decoded frame). The
/// `VorbisComment` comment header stores each `KEY=value` pair as plain
/// ASCII/UTF-8 preceded by its own 4-byte little-endian length (Xiph's
/// "Vorbis comment field and header specification"), so — unlike scanning
/// forward for where the base64 "looks like it stops" (fragile: raw binary
/// bytes belonging to the *next* field can coincidentally pass as base64
/// alphabet) — the exact value length is read from those 4 length bytes
/// immediately preceding the marker.
///
/// Assumes the comment packet fits on a single Ogg page (payload under the
/// ~65025-byte max page size): this reads the tag's bytes directly out of
/// `media` as one contiguous span, which an `OggS` page header would split
/// if the field spanned pages. Not an issue at `COVER_DIMENSION_PX`'s
/// current size, but raising it enough to push the base64 payload past a
/// page boundary would need this helper reassembled page-aware first.
fn read_metadata_block_picture(media: &Path) -> Vec<u8> {
    const MARKER: &[u8] = b"METADATA_BLOCK_PICTURE=";

    let bytes = std::fs::read(media).expect("read output media file");
    let marker_start = bytes
        .windows(MARKER.len())
        .position(|w| w == MARKER)
        .unwrap_or_else(|| panic!("{} has no METADATA_BLOCK_PICTURE tag", media.display()));
    let length_prefix_start = marker_start - 4;
    let field_len =
        u32::from_le_bytes(bytes[length_prefix_start..marker_start].try_into().unwrap()) as usize;
    let value_start = marker_start + MARKER.len();
    let value_len = field_len
        .checked_sub(MARKER.len())
        .expect("comment field length must cover at least the key");
    let encoded = std::str::from_utf8(&bytes[value_start..value_start + value_len])
        .expect("base64 span must be ascii");
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("METADATA_BLOCK_PICTURE must be valid base64")
}

/// Declared width/height from a FLAC `PICTURE` block (RFC 9639 §8.8), at
/// their fixed offset following the picture-type, MIME, and description
/// fields. `FFmpeg` ignores these on read (it derives real dimensions from
/// the decoded frame instead — see `vorbis_picture.rs`'s module docs), but
/// other readers (e.g. `mutagen`) trust the declared value, so nothing else
/// in this test suite would catch a regression here.
fn declared_dimensions(block: &[u8]) -> (u32, u32) {
    let mime_len = u32::from_be_bytes(block[4..8].try_into().unwrap()) as usize;
    let description_len_offset = 8 + mime_len;
    let description_len = u32::from_be_bytes(
        block[description_len_offset..description_len_offset + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    let width_offset = description_len_offset + 4 + description_len;
    let width = u32::from_be_bytes(block[width_offset..width_offset + 4].try_into().unwrap());
    let height = u32::from_be_bytes(
        block[width_offset + 4..width_offset + 8]
            .try_into()
            .unwrap(),
    );
    (width, height)
}

/// One (container, audio encoder, cover extension) case for the table-driven
/// round-trip test below.
struct OggOpusCase {
    /// Container/remux target passed to `embed_thumbnail`.
    container: &'static str,
    /// `FFmpeg` audio encoder used to build the fixture audio stream.
    audio_encoder: &'static str,
    /// Cover image file extension (drives the source format under test).
    cover_ext: &'static str,
    /// Whether this container carries the cover as a `METADATA_BLOCK_PICTURE`
    /// tag (Ogg/Opus, #531) rather than an `ATTACHED_PIC` stream (FLAC) — the
    /// declared-dimensions pin only applies to the former.
    uses_metadata_block_picture: bool,
}

/// Embed `case`'s cover into a freshly-built audio fixture and assert the
/// extracted cover round-trips byte-identical to the source. For containers
/// using `METADATA_BLOCK_PICTURE`, additionally pins that the tag's declared
/// width/height match the cover's real pixel dimensions.
async fn run_case(case: &OggOpusCase, test_name: &str) {
    if !ffmpeg_cli_available() {
        skip_or_panic_without_ffmpeg_cli(test_name);
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let cover = make_cover(dir.path(), case.cover_ext);
    let audio = make_audio(dir.path(), case.container, case.audio_encoder);
    let output = dir.path().join(format!("out.{}", case.container));

    let runner = FFmpegRunner::new().expect("FFmpeg");
    let result = runner
        .embed_thumbnail(&audio, &cover, &output, case.container, None, None)
        .await;

    if let Err(e) = &result {
        let msg = format!("{e:#}");
        assert!(
            !msg.contains("Unsupported codec id"),
            "the #531 mux failure recurred: {msg}"
        );
    }
    result.unwrap_or_else(|e| {
        panic!(
            "thumbnail embed into {} must succeed: {e:#}",
            case.container
        )
    });

    let extracted = dir.path().join(format!("extracted.{}", case.cover_ext));
    extract_cover(&output, &extracted);

    let original = std::fs::read(&cover).unwrap();
    let round_tripped = std::fs::read(&extracted).unwrap();
    assert_eq!(
        original, round_tripped,
        "extracted cover must be byte-identical to the source thumbnail"
    );

    if case.uses_metadata_block_picture {
        let block = read_metadata_block_picture(&output);
        let (width, height) = declared_dimensions(&block);
        assert_eq!(
            (width, height),
            (COVER_DIMENSION_PX, COVER_DIMENSION_PX),
            "declared METADATA_BLOCK_PICTURE dimensions must match the cover's real pixel size"
        );
    }
}

/// Load-bearing regression: embedding a thumbnail into Opus must succeed and
/// must NOT reproduce the old "Unsupported codec id" mux failure.
#[tokio::test]
async fn embed_thumbnail_into_opus_round_trips_byte_identical_cover() {
    run_case(
        &OggOpusCase {
            container: "opus",
            audio_encoder: "libopus",
            cover_ext: "png",
            uses_metadata_block_picture: true,
        },
        "embed_thumbnail_into_opus_round_trips_byte_identical_cover",
    )
    .await;
}

/// Same regression for `.ogg` (Vorbis audio) with a JPEG cover, pinning the
/// other codec/container combination named in #531.
#[tokio::test]
async fn embed_thumbnail_into_ogg_round_trips_byte_identical_cover() {
    run_case(
        &OggOpusCase {
            container: "ogg",
            audio_encoder: "libvorbis",
            cover_ext: "jpg",
            uses_metadata_block_picture: true,
        },
        "embed_thumbnail_into_ogg_round_trips_byte_identical_cover",
    )
    .await;
}

/// Non-regression: FLAC's `ATTACHED_PIC`-stream strategy is untouched by the
/// Ogg/Opus fix (`is_ogg_opus` is `false` for `flac`) and must keep working —
/// the shared thumbnail-stream/packet code path is a common seam between the
/// two branches.
#[tokio::test]
async fn embed_thumbnail_into_flac_still_round_trips_byte_identical_cover() {
    run_case(
        &OggOpusCase {
            container: "flac",
            audio_encoder: "flac",
            cover_ext: "png",
            uses_metadata_block_picture: false,
        },
        "embed_thumbnail_into_flac_still_round_trips_byte_identical_cover",
    )
    .await;
}
