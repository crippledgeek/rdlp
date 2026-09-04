//! Matroska/WebM container corruption detection and salvage remux.
//!
//! Detects structurally corrupt Matroska/WebM containers by capturing `FFmpeg`
//! demuxer log output during packet iteration and checking for known EBML
//! error markers. When corruption is detected, a salvage remux (stream copy)
//! produces a structurally valid container that can be processed normally.
//!
//! ## Why EBML corruption causes downstream ENOSPC
//!
//! Malformed EBML elements (invalid IDs, truncated sizes, elements exceeding
//! their parent) cause the Matroska demuxer to produce packets with broken
//! timestamps (out-of-order DTS, negative durations, implausible PTS values).
//! When these packets enter the muxer's interleave queue, the queue grows
//! unboundedly waiting for "future" packets that will never arrive at the
//! expected timestamps. Once the muxer's internal buffer exceeds its limit,
//! `av_interleaved_write_frame` returns AVERROR(ENOSPC) — "Not enough space"
//! — which is a muxer queue overflow, not a disk space error.
//!
//! ## Why salvage remux fixes it deterministically
//!
//! `FFmpeg`'s Matroska demuxer is resilient to many structural errors — it logs
//! warnings but continues reading. A stream-copy remux reads all recoverable
//! packets from the corrupt input and writes them into a fresh container with
//! valid EBML structure, monotonic timestamps, and correct cluster boundaries.
//! The salvaged output is structurally sound, so subsequent transcoding or
//! normalization operates on clean data without triggering the ENOSPC cascade.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use log::{debug, info, warn};

use crate::error::{CorruptionKind, PostProcessError, Result};

use super::ffi_helpers::{cleanup_partial_output, packet_unref};
use super::log_capture::{LogCaptureGuard, LogSuppressGuard};
use super::{FFmpegRunner, ensure_init};

/// EBML corruption marker substrings that indicate malformed Matroska/WebM containers.
///
/// These are emitted by `FFmpeg`'s Matroska demuxer at WARNING/ERROR level when
/// it encounters structural problems in the EBML byte stream.
const EBML_CORRUPTION_MARKERS: &[&str] = &[
    "invalid as first byte of an EBML number",
    "exceeds containing master element",
    "Duplicate element",
];

/// Open an input file with resilient format flags (`discardcorrupt+genpts`).
///
/// This is the library-equivalent of `FFmpeg` CLI's `-fflags +discardcorrupt+genpts`.
/// The flags instruct the demuxer to skip corrupt packets and regenerate PTS/DTS
/// timestamps, allowing decode of partially corrupt containers that would otherwise
/// produce invalid packets and cascade into muxer failures.
///
/// Used as Tier 3 recovery in `super::normalize::helpers::with_mux_retry` when
/// both normal encode and salvage remux fail.
///
/// # Errors
///
/// Returns an error if `FFmpeg` fails to open `path` even with the resilient
/// flags applied (e.g. the file does not exist or is not a media container).
pub fn open_input_resilient(path: &Path) -> Result<ffmpeg_the_third::format::context::Input> {
    ensure_init()?;

    let mut opts = ffmpeg_the_third::Dictionary::new();
    opts.set("fflags", "+discardcorrupt+genpts");

    let ictx = ffmpeg_the_third::format::input_with_dictionary(path, opts).map_err(|e| {
        PostProcessError::FFmpegLibraryError {
            message: format!("resilient open failed for {}: {e}", path.display()),
        }
    })?;

    debug!(
        "[resilient] opened {} with discardcorrupt+genpts",
        path.display()
    );
    Ok(ictx)
}

/// Check a media input for Matroska/WebM container-level corruption.
///
/// Opens the input with log capture active, reads all packets (no decoding),
/// and checks for EBML structural error markers in the demuxer log output.
///
/// Returns `Ok(())` if the container is clean or not Matroska/WebM.
/// Returns `Err(InputCorrupt { kind: MatroskaEbml, .. })` if corruption is detected.
///
/// # Errors
///
/// Returns [`PostProcessError::InputCorrupt`] when EBML structural error
/// markers are found in the demuxer log, or any other [`PostProcessError`]
/// if `input` cannot be opened or log capture cannot be started.
pub fn check_matroska_integrity(input: &Path) -> Result<()> {
    ensure_init()?;

    let guard = LogCaptureGuard::begin()?;

    // Open input — this reads the Matroska header and may already trigger EBML warnings
    let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
        PostProcessError::FFmpegLibraryError {
            message: format!(
                "failed to open input for integrity check {}: {e}",
                input.display()
            ),
        }
    })?;

    // Only check Matroska/WebM containers — other formats are not affected
    let format_name = ictx.format().name().to_string();
    if !format_name.contains("matroska") && !format_name.contains("webm") {
        drop(guard);
        return Ok(());
    }

    debug!(
        "Checking matroska container integrity: {} (format: {})",
        input.display(),
        format_name,
    );

    // Read all packets to trigger demuxer EBML validation.
    // This is a packet-level scan (no decoding), so it's fast — bounded
    // only by disk I/O speed.
    for result in ictx.packets() {
        if result.is_err() {
            break; // Demuxer hard error — check logs below
        }
    }

    // Check captured logs for EBML corruption markers
    let lines = guard.take_captured()?;
    drop(guard);

    let corrupt_logs: Vec<String> = lines
        .into_iter()
        .filter(|line| {
            EBML_CORRUPTION_MARKERS
                .iter()
                .any(|marker| line.contains(marker))
        })
        .collect();

    if corrupt_logs.is_empty() {
        Ok(())
    } else {
        Err(PostProcessError::InputCorrupt {
            kind: CorruptionKind::MatroskaEbml,
            path: input.to_path_buf(),
            logs: corrupt_logs,
        })
    }
}

/// Maximum number of `.salvageN.` disambiguation slots probed before giving up.
///
/// Bounds the collision search so a directory already full of salvage files
/// cannot spin unbounded; when every slot is occupied the last slot is reused
/// (matching the historical `attempt >= 99` cap).
const MAX_SALVAGE_ATTEMPTS: u32 = 99;

/// Pick the salvage output path for `input`, avoiding collisions with existing
/// files so a previous partial salvage is never overwritten.
///
/// Returns `stem.salvage.ext` when that slot is free; otherwise the first free
/// `stem.salvageN.ext` for `N` in `1..=MAX_SALVAGE_ATTEMPTS`. If every slot is
/// occupied, falls back to the last slot (`MAX_SALVAGE_ATTEMPTS`).
fn next_salvage_path(input: &Path, stem: &str, ext: &str) -> PathBuf {
    let base = input.with_file_name(format!("{stem}.salvage.{ext}"));
    if !base.exists() {
        return base;
    }
    (1..=MAX_SALVAGE_ATTEMPTS)
        .map(|n| input.with_file_name(format!("{stem}.salvage{n}.{ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| {
            input.with_file_name(format!("{stem}.salvage{MAX_SALVAGE_ATTEMPTS}.{ext}"))
        })
}

/// Salvage a corrupt Matroska/WebM container by remuxing with stream copy.
///
/// Reads all recoverable packets from the corrupt input and writes them into
/// a fresh container with valid EBML structure. Uses `LogSuppressGuard` to
/// suppress the expected EBML warning spam during the remux.
///
/// Returns the path to the salvaged temporary file. The caller is responsible
/// for cleaning up this file when done.
///
/// # Errors
///
/// Returns an error if `input`/the salvage output cannot be opened, if the
/// target container cannot represent one of the input's streams (see
/// [`crate::error::PostProcessError::IncompatibleContainerCodec`]), if no
/// packets at all could be recovered, or if the header/trailer cannot be
/// written.
pub fn salvage_remux_sync(input: &Path) -> anyhow::Result<PathBuf> {
    ensure_init()?;

    // Always use MKV for salvage output. Matroska writes metadata
    // incrementally (no trailer indexing), avoiding ENOMEM that occurs
    // with MP4/MOV trailer writes under memory pressure.
    let ext = "mkv";
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("salvage");
    let salvage_path = next_salvage_path(input, stem, ext);

    info!(
        "Salvage remuxing corrupt input: {} -> {}",
        input.display(),
        salvage_path.display(),
    );

    // Suppress FFmpeg log output during salvage — the corrupt input will
    // generate many EBML warnings that we've already captured and reported.
    let log_suppress = LogSuppressGuard::new();

    // Open corrupt input — FFmpeg will log warnings but continue reading
    let mut ictx = ffmpeg_the_third::format::input(input)
        .map_err(PostProcessError::from)
        .with_context(|| {
            format!(
                "failed to open corrupt input for salvage {}",
                input.display()
            )
        })?;

    let mut octx = ffmpeg_the_third::format::output(&salvage_path)
        .map_err(PostProcessError::from)
        .with_context(|| format!("failed to create salvage output {}", salvage_path.display()))?;

    // Map all input streams to output (stream copy, no re-encoding). No
    // medium filter: salvage is the corruption-recovery path of last
    // resort and must not silently drop a stream (e.g. a font attachment)
    // just because it isn't audio/video.
    //
    // `format::output` above already created (truncated) `salvage_path` on
    // disk via `avio_open` — a codec-tag rejection here must not leave
    // that empty file behind as if a salvage remux had run and produced
    // nothing.
    let stream_count = ictx.streams().count();
    for ist in ictx.streams() {
        let ost_idx = FFmpegRunner::add_stream_copy(&mut octx, ist.parameters(), "for salvage")
            .inspect_err(|_| cleanup_partial_output(&salvage_path))?;
        // Per-stream metadata must travel with the stream: for an
        // attachment, Matroska's muxer hard-requires a "mimetype" tag
        // (`matroskaenc.c`'s `mkv_write_attachments` rejects the whole
        // mux with EINVAL when it's absent) — dropping it here would
        // trade the codec-tag rejection this fix removes for a header
        // -write failure instead, on the exact fixture (a font
        // attachment) this fix exists to recover.
        if let Some(mut ost) = octx.stream_mut(ost_idx) {
            ost.set_metadata(ist.metadata().to_owned());
        }
    }

    // Use FFmpeg's default 10s max_interleave_delta for salvage. This prevents
    // unbounded queue growth from corrupt timestamps in the input while still
    // allowing reasonable interleave buffering. 0 would mean "no delta limit"
    // (not "flush immediately"), risking ENOMEM on severely corrupt inputs.
    // SAFETY: octx owns a valid, pre-header output format context.
    unsafe {
        (*octx.as_mut_ptr()).max_interleave_delta = 10_000_000;
    }
    let mut muxer_opts = ffmpeg_the_third::Dictionary::new();
    muxer_opts.set("cluster_time_limit", "500");

    crate::ffmpeg::encoding_tag::set_encoding_tool_if_missing(&mut octx, "salvage");

    octx.write_header_with(muxer_opts)
        .map_err(PostProcessError::from)
        .context("failed to write salvage output header")?;

    // Copy all recoverable packets, skipping ones that fail to read or write
    let mut copied = 0u64;
    let mut skipped = 0u64;

    for result in ictx.packets() {
        match result {
            Ok((stream, mut packet)) => {
                let in_idx = stream.index();
                if in_idx >= stream_count {
                    skipped += 1;
                    continue;
                }

                let in_tb = stream.time_base();
                let out_tb = if let Some(s) = octx.stream(in_idx) {
                    s.time_base()
                } else {
                    skipped += 1;
                    continue;
                };

                packet.rescale_ts(in_tb, out_tb);
                packet.set_stream(in_idx);
                packet.set_position(-1);

                if packet.write_interleaved(&mut octx).is_ok() {
                    copied += 1;
                } else {
                    skipped += 1;
                }
                packet_unref(&mut packet);
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }

    // Drop suppress guard before write_trailer so any trailer errors are visible
    drop(log_suppress);

    octx.write_trailer()
        .map_err(PostProcessError::from)
        .context("failed to write salvage output trailer")?;

    info!("Salvage remux complete: {copied} packets copied, {skipped} skipped");

    if copied == 0 {
        // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
        #[allow(clippy::disallowed_methods)]
        let _ = std::fs::remove_file(&salvage_path);
        return Err(PostProcessError::SalvageFailed {
            message: "no packets could be recovered from corrupt input".into(),
        }
        .into());
    }

    Ok(salvage_path)
}

/// Check container integrity and optionally salvage corrupt Matroska/WebM inputs.
///
/// Returns `(effective_input, salvage_temp)` where:
/// - `effective_input` is the path to use for processing (original if clean, salvaged if corrupt)
/// - `salvage_temp` is `Some(path)` if a temp file was created that the caller must clean up
///
/// If `salvage_enabled` is false and corruption is detected, returns the
/// `InputCorrupt` error directly.
///
/// # Errors
///
/// Returns [`PostProcessError::InputCorrupt`] when corruption is detected and
/// `salvage_enabled` is `false`, or any error [`salvage_remux_sync`] can
/// return when `salvage_enabled` is `true` and the salvage attempt itself
/// fails.
pub fn prepare_input_with_salvage(
    input: &Path,
    salvage_enabled: bool,
) -> Result<(PathBuf, Option<PathBuf>)> {
    match check_matroska_integrity(input) {
        Ok(()) => Ok((input.to_path_buf(), None)),
        Err(e @ PostProcessError::InputCorrupt { .. }) => {
            if let PostProcessError::InputCorrupt {
                ref kind, ref logs, ..
            } = e
            {
                warn!(
                    "input corrupt ({kind}): {} ({} demuxer error(s))",
                    input.display(),
                    logs.len(),
                );
                for line in logs {
                    debug!("  ebml: {}", line.trim());
                }
            }

            if !salvage_enabled {
                return Err(e);
            }

            info!("Attempting salvage remux for corrupt input...");
            let salvaged = salvage_remux_sync(input)?;
            Ok((salvaged.clone(), Some(salvaged)))
        }
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_ebml_markers_detect_invalid_first_byte() {
        let line = "[matroska,webm @ 0x1234] invalid as first byte of an EBML number";
        assert!(
            EBML_CORRUPTION_MARKERS
                .iter()
                .any(|marker| line.contains(marker))
        );
    }

    #[test]
    fn test_ebml_markers_detect_exceeds_master() {
        let line = "[matroska,webm @ 0x5678] element exceeds containing master element size";
        assert!(
            EBML_CORRUPTION_MARKERS
                .iter()
                .any(|marker| line.contains(marker))
        );
    }

    #[test]
    fn test_ebml_markers_detect_duplicate() {
        let line = "[matroska,webm @ 0x9abc] Duplicate element";
        assert!(
            EBML_CORRUPTION_MARKERS
                .iter()
                .any(|marker| line.contains(marker))
        );
    }

    #[test]
    fn test_ebml_markers_reject_normal_logs() {
        let lines = [
            "[info] loudnorm pass 1 complete",
            "[aac @ 0x1234] channel layout not set",
            "[mp4 @ 0x5678] muxing complete",
        ];
        for line in &lines {
            assert!(
                !EBML_CORRUPTION_MARKERS
                    .iter()
                    .any(|marker| line.contains(marker))
            );
        }
    }

    #[test]
    fn test_corruption_kind_display() {
        assert_eq!(CorruptionKind::MatroskaEbml.to_string(), "matroska ebml");
    }

    #[test]
    fn test_input_corrupt_error_display() {
        let err = PostProcessError::InputCorrupt {
            kind: CorruptionKind::MatroskaEbml,
            path: PathBuf::from("/tmp/test.mkv"),
            logs: vec!["Duplicate element".into()],
        };
        let display = err.to_string();
        assert!(display.contains("matroska ebml"));
        assert!(display.contains("test.mkv"));
    }

    #[test]
    fn test_salvage_failed_error_display() {
        let err = PostProcessError::SalvageFailed {
            message: "no packets recovered".into(),
        };
        assert!(err.to_string().contains("salvage failed"));
    }

    #[test]
    fn test_check_nonexistent_file_returns_error() {
        if ensure_init().is_err() {
            return; // FFmpeg not available
        }
        let result = check_matroska_integrity(Path::new("/nonexistent/file.mkv"));
        assert!(result.is_err());
    }

    #[test]
    fn next_salvage_path_uses_base_when_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("movie.mkv");
        // Nothing occupies the base slot yet → the un-numbered path is chosen.
        assert_eq!(
            next_salvage_path(&input, "movie", "mkv"),
            dir.path().join("movie.salvage.mkv")
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // test fixture: create sync files to occupy salvage slots
    fn next_salvage_path_skips_occupied_slots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("movie.mkv");
        // Base + first numbered slot are already taken by prior salvage runs;
        // the next free slot is salvage2 — a previous partial file is never reused.
        for name in ["movie.salvage.mkv", "movie.salvage1.mkv"] {
            std::fs::File::create(dir.path().join(name)).expect("create fixture");
        }
        assert_eq!(
            next_salvage_path(&input, "movie", "mkv"),
            dir.path().join("movie.salvage2.mkv")
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // test fixture: occupy every salvage slot
    fn next_salvage_path_reuses_last_slot_when_all_occupied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let input = dir.path().join("movie.mkv");
        // Occupy the base slot and every numbered slot 1..=MAX.
        std::fs::File::create(dir.path().join("movie.salvage.mkv")).expect("create base");
        for n in 1..=MAX_SALVAGE_ATTEMPTS {
            std::fs::File::create(dir.path().join(format!("movie.salvage{n}.mkv")))
                .expect("create slot");
        }
        // No free slot remains → fall back to the last slot, matching the old
        // loop's `attempt >= 99` cap that returned salvage99 even when occupied.
        assert_eq!(
            next_salvage_path(&input, "movie", "mkv"),
            dir.path()
                .join(format!("movie.salvage{MAX_SALVAGE_ATTEMPTS}.mkv"))
        );
    }
}
