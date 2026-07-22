//! Muxer/codec pairs FFmpeg implements but never declares (#633).
//!
//! `oformat_can_represent` accepts only two positive-evidence channels — a hit
//! in the muxer's `codec_tag` table, or `avformat_query_codec(..) > 0`. Some
//! muxers implement a codec through neither: `mxfenc` carries a full H.264
//! essence mapping (`mxf_h264_codec_uls`) with no `query_codec` callback and a
//! NULL top-level `codec_tag`, and `mpegts` maps AAC internally in
//! `mpegts.c`. Both report `AVERROR_PATCHWELCOME` — "the muxer declared
//! nothing" — which the predicate correctly reads as no evidence, so rdlp
//! re-encodes a stream `ffmpeg -c copy` would have copied.
//!
//! Research (2026-07-22, FFmpeg master `libavformat/mux_utils.c:32-69`,
//! `mux.c`, `fftools/ffmpeg_mux_init.c`) established there is no third API to
//! consult: `avformat_query_codec` is the entire surface and is advisory, and
//! the `ffmpeg` CLI does not pre-check stream copy at all — `streamcopy_init`
//! uses `av_codec_get_tag2` only to *pick* a tag, never to reject. A curated
//! allow-list is therefore the only remedy.
//!
//! **This suite is the audit for that table.** Every entry must be proven by a
//! real mux, because the whole point of the table is to assert something
//! FFmpeg's own metadata does not. An entry that is wrong routes to a stream
//! copy the muxer then refuses mid-mux — the #630 failure class — so the table
//! is worthless without this test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod common;

use std::path::Path;
use std::process::Command;

use common::{decoded_frames, ffmpeg_available};
use rdlp_ffmpeg::{MediaKind, muxer_can_represent};
use rdlp_types::ContainerFormat;

/// One allow-list entry, paired with how to build a source carrying that codec.
struct KnownAccept {
    container: ContainerFormat,
    codec: &'static str,
    kind: MediaKind,
    /// `ffmpeg` args producing a one-second source in `codec`.
    source_args: &'static [&'static str],
    /// Extension for the generated source file.
    source_ext: &'static str,
}

/// Mirrors `KNOWN_UNDECLARED_SUPPORT` in `muxer_defaults`. Kept as a separate
/// list on purpose: a test that read the production table would assert the table
/// against itself and prove nothing. This list is the independent claim; the
/// assertions below check the production table agrees AND that ffmpeg agrees.
const CASES: &[KnownAccept] = &[
    KnownAccept {
        container: ContainerFormat::Mxf,
        codec: "h264",
        kind: MediaKind::Video,
        source_args: &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=320x240:r=25",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
        source_ext: "mp4",
    },
    KnownAccept {
        container: ContainerFormat::Ts,
        codec: "aac",
        kind: MediaKind::Audio,
        source_args: &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:a",
            "aac",
        ],
        source_ext: "m4a",
    },
];

fn build_source(case: &KnownAccept, path: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(case.source_args)
        .arg(path)
        .status()
        .is_ok_and(|s| s.success())
}

/// Ground truth: does `ffmpeg -c copy` actually accept this pairing?
///
/// Exit status, never file size — a refused mux still leaves a partial header
/// on disk, so a size check scores a failure as a success.
fn ffmpeg_copies(src: &Path, dst: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-c", "copy"])
        .arg(dst)
        .status()
        .is_ok_and(|s| s.success())
}

/// `CASES` must cover the whole production table.
///
/// The three table-driven tests below iterate `CASES`, which is deliberately
/// an independent list — reading the production table would make the predicate
/// assertion a tautology, and `CASES` additionally carries the `ffmpeg` args
/// needed to synthesise a source. But independence cuts both ways: without
/// this check, adding a row to `KNOWN_UNDECLARED_SUPPORT` and forgetting
/// `CASES` ships an entirely unproven entry and **no test fails** — the exact
/// #630 hazard the table's own rules warn about, with the audit silently
/// opted out.
#[test]
fn cases_cover_every_row_of_the_production_table() {
    rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");

    // Compare (container, codec) PAIRS, not just containers. Container-only
    // matching plus a length check looks sufficient but is not: adding
    // `(Ts, ac3)` to the table and copy-pasting the existing `Ts` block in
    // `CASES` without editing its `codec` field keeps both the count and the
    // container set intact, while the three ground-truth tests below prove
    // `aac → ts` twice and `ac3 → ts` never. The new row would ship with the
    // real-mux proof — the table's whole justification — never having run.
    // Compared as `(extension, codec)` string pairs because `ContainerFormat`
    // is not `Ord`; the extension identifies the variant uniquely, which is
    // all the comparison needs.
    let mut table: Vec<(&str, &str)> = rdlp_ffmpeg::known_undeclared_support_named()
        .into_iter()
        .map(|(container, codec)| (container.as_ext(), codec))
        .collect();
    let mut proven: Vec<(&str, &str)> = CASES
        .iter()
        .map(|c| (c.container.as_ext(), c.codec))
        .collect();

    table.sort_unstable();
    proven.sort_unstable();

    assert_eq!(
        proven, table,
        "this suite and KNOWN_UNDECLARED_SUPPORT describe different (container, codec) sets — \
         a row is either unproven here or proven here without being in the table"
    );

    // Duplicates would let two identical CASES entries satisfy a two-row
    // table while leaving one real row unproven.
    let mut deduped = proven.clone();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        proven.len(),
        "CASES contains a duplicate (container, codec) pair — one production row is unproven"
    );
}

/// Every entry must be something ffmpeg genuinely copies. A wrong entry is
/// worse than a missing one: it routes to a copy the muxer refuses mid-mux.
#[test]
fn every_known_accept_entry_is_confirmed_by_a_real_mux() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");

    for case in CASES {
        let src = dir
            .path()
            .join(format!("src_{}.{}", case.codec, case.source_ext));
        assert!(
            build_source(case, &src),
            "could not build a {} source",
            case.codec
        );

        let dst = dir
            .path()
            .join(format!("out_{}.{}", case.codec, case.container.as_ext()));
        assert!(
            ffmpeg_copies(&src, &dst),
            "{:?} + {} is in the allow-list but ffmpeg refuses the copy — the entry is WRONG \
             and would route to a mux that fails",
            case.container,
            case.codec
        );
    }
}

/// The predicate must now answer yes for these, which is the fix (#633).
/// Fails against unpatched code, where both report `AVERROR_PATCHWELCOME`.
#[test]
fn muxer_can_represent_accepts_the_known_undeclared_pairs() {
    rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
    for case in CASES {
        assert!(
            muxer_can_represent(case.container, case.codec, case.kind),
            "{:?} + {} ({:?}) must be accepted — ffmpeg copies it, and the allow-list exists \
             precisely because the muxer declares it through neither evidence channel",
            case.container,
            case.codec,
            case.kind
        );
    }
}

/// rdlp's own mux path must complete the copy — specifically, enforcement
/// must not refuse what routing chose.
///
/// `ffmpeg -c copy` agreeing is necessary but not sufficient: rdlp routes
/// through `resolve_codec_tag`, which can refuse a pairing mid-mux
/// independently of routing. A wrong allow-list entry produces exactly that —
/// routing picks a copy, enforcement refuses, and the user gets a fatal error
/// where a transcode was available (#630 class).
///
/// Scope, stated precisely: `convert_video` with `remux_only` dispatches to
/// `remux_sync`, which does exercise `resolve_codec_tag` for both rows, so the
/// enforcement half is genuinely proven here. It is **not** the production
/// route the `aac → mpegts` row changes — that one flows through
/// `can_copy_audio_only` → `recode_audio_only`, and the *routing* decision is
/// pinned by
/// `recode_audio_only::tests::audio_only_input_copies_when_the_container_carries_its_audio_codec`,
/// which includes `Ts`.
///
/// `recode_audio_only_source_matrix` exercises that route end-to-end but does
/// **not** pin copy-vs-re-encode — its own `Expected::Produces` doc says which
/// one happens "is an optimisation, not a contract" — so it is not evidence
/// for this row and is deliberately not cited as such.
///
/// `audio_copy: true` is inert on the remux branch and is set only for
/// clarity.
#[tokio::test]
async fn every_known_accept_entry_survives_rdlps_own_remux() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = rdlp_ffmpeg::FFmpegRunner::new().expect("FFmpegRunner");

    for case in CASES {
        let src = dir
            .path()
            .join(format!("rdlp_src_{}.{}", case.codec, case.source_ext));
        assert!(
            build_source(case, &src),
            "could not build a {} source",
            case.codec
        );

        let out = dir.path().join(format!(
            "rdlp_out_{}.{}",
            case.codec,
            case.container.as_ext()
        ));
        let opts = rdlp_ffmpeg::VideoConvertOptions {
            remux_only: true,
            audio_copy: true,
            ..Default::default()
        };

        runner
            .convert_video(&src, &out, &opts, None, None, None)
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "{:?} + {} is allow-listed but rdlp's remux failed: {e:#} — routing chose a \
                     copy that enforcement or the muxer refused (#630 class)",
                    case.container, case.codec
                )
            });

        assert!(
            decoded_frames(&out, None).is_some_and(|n| n > 0),
            "{:?} + {}: rdlp reported success but the output does not decode",
            case.container,
            case.codec
        );
    }
}

/// The table must not have loosened anything beyond its own rows.
///
/// Asymmetry itself is guaranteed by the code shape — `muxer_can_represent`
/// is `oformat_can_represent(..) || is_known_undeclared_support(..)`, and the
/// latter is a positive `.any()` with no other caller, so it is structurally
/// incapable of refusing. What this pins is the blast radius: a codec the
/// muxer genuinely rejects is still refused, and the media kind is still
/// honoured — `muxer_can_represent`'s `kind` check is what
/// stops the `"avc"`/ON2AVC audio-vs-video alias collision, and an allow-list
/// that bypassed it would reintroduce exactly that.
#[test]
fn the_allow_list_does_not_loosen_refusals_or_cross_media_kinds() {
    rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");

    // WebM genuinely cannot carry AAC — unchanged by the allow-list.
    assert!(
        !muxer_can_represent(ContainerFormat::WebM, "aac", MediaKind::Audio),
        "webm rejects aac; the allow-list must not have loosened that"
    );

    // The allow-listed pairs must not leak across media kinds.
    assert!(
        !muxer_can_represent(ContainerFormat::Mxf, "h264", MediaKind::Audio),
        "h264 is video; asking the audio question must still answer no"
    );
    assert!(
        !muxer_can_represent(ContainerFormat::Ts, "aac", MediaKind::Video),
        "aac is audio; asking the video question must still answer no"
    );

    // Pairings that were already true must stay true.
    assert!(muxer_can_represent(
        ContainerFormat::Mp4,
        "h264",
        MediaKind::Video
    ));
    assert!(muxer_can_represent(
        ContainerFormat::Mkv,
        "aac",
        MediaKind::Audio
    ));
}
