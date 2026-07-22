//! Container/codec issue detection and repair.
//!
//! # Lint allowances
//!
//! - `clippy::indexing_slicing`: `stream_mapping[ist_index]` and
//!   `ist_time_bases[ist_index]` are pre-allocated to `ictx.streams().count()`
//!   and indexed only during iteration over those same streams, so bounds are
//!   guaranteed by construction.
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types. All casts are
//!   audited and within valid FFmpeg-returned value ranges.
//! - `clippy::expect_used`: `octx.stream_mut(idx)` after just-added stream is
//!   guaranteed valid by construction.

#![allow(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::expect_used
)]

use std::path::Path;

use anyhow::Context as _;
use log::info;

use super::probe::{MediaInfo, StreamKind};
use super::{FFmpegRunner, RemuxOptions};
use crate::error::Result;

/// A detected issue in a media file.
#[derive(Debug, Clone, PartialEq)]
pub enum FixupIssue {
    /// Video stream has a non-square or broken sample aspect ratio.
    StretchedVideo {
        /// Index of the affected stream.
        stream_index: usize,
        /// SAR numerator.
        sar_num: i32,
        /// SAR denominator.
        sar_den: i32,
    },
    /// MP4 moov atom is missing or placed at the end of the file.
    MissingMoovAtom,
    /// A video or audio stream reports zero duration or zero frames.
    ZeroDurationStream {
        /// Index of the affected stream.
        stream_index: usize,
        /// Codec type of the affected stream (video or audio).
        codec_type: StreamKind,
    },
    /// The file is shorter than the expected duration by more than 10%.
    TruncatedFile {
        /// Expected duration from the source metadata (seconds).
        expected_secs: f64,
        /// Actual duration reported by the container (seconds).
        actual_secs: f64,
    },
    /// The file contains a codec that is not supported by its container.
    UnsupportedCodecForContainer {
        /// Codec name (e.g. "vp9").
        codec: String,
        /// Container format (e.g. "mp4").
        container: String,
    },
}

impl FixupIssue {
    /// Returns `true` if this issue can be repaired via a re-mux pass.
    #[must_use]
    pub const fn is_repairable(&self) -> bool {
        matches!(
            self,
            Self::StretchedVideo { .. } | Self::MissingMoovAtom | Self::ZeroDurationStream { .. }
        )
    }
}

impl std::fmt::Display for FixupIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StretchedVideo {
                stream_index,
                sar_num,
                sar_den,
            } => {
                write!(
                    f,
                    "stream {stream_index}: stretched SAR {sar_num}:{sar_den}"
                )
            }
            Self::MissingMoovAtom => write!(f, "missing or misplaced moov atom (MP4)"),
            Self::ZeroDurationStream {
                stream_index,
                codec_type,
            } => {
                write!(f, "stream {stream_index} ({codec_type}): zero duration")
            }
            Self::TruncatedFile {
                expected_secs,
                actual_secs,
            } => {
                write!(
                    f,
                    "truncated: expected {expected_secs:.1}s, got {actual_secs:.1}s"
                )
            }
            Self::UnsupportedCodecForContainer { codec, container } => {
                write!(f, "codec {codec} not supported in {container}")
            }
        }
    }
}

/// Detect issues in a probed media file.
///
/// `expected_duration` is the duration from `InfoDict.duration` (if available).
#[must_use]
pub fn detect_issues(info: &MediaInfo, expected_duration: Option<f64>) -> Vec<FixupIssue> {
    let mut issues = Vec::new();

    for stream in &info.streams {
        // Stretched SAR: only flag clearly wrong values
        // Conservative: 0:x, x:0, or extreme ratios (>10:1)
        if stream.codec_type == StreamKind::Video
            && let (Some(num), Some(den)) = (stream.sar_num, stream.sar_den)
        {
            let is_broken = num == 0
                || den == 0
                || (num > 0
                    && den > 0
                    && (f64::from(num) / f64::from(den) > 10.0
                        || f64::from(den) / f64::from(num) > 10.0));

            if is_broken {
                issues.push(FixupIssue::StretchedVideo {
                    stream_index: stream.index,
                    sar_num: num,
                    sar_den: den,
                });
            }
        }

        // Zero-duration stream (video or audio)
        if matches!(stream.codec_type, StreamKind::Video | StreamKind::Audio) {
            let zero_dur = stream.duration.is_some_and(|d| d <= 0.0);
            let zero_frames =
                stream.codec_type == StreamKind::Video && stream.nb_frames.is_some_and(|n| n == 0);

            if zero_dur || zero_frames {
                issues.push(FixupIssue::ZeroDurationStream {
                    stream_index: stream.index,
                    codec_type: stream.codec_type,
                });
            }
        }
    }

    // Truncated file: actual < 90% of expected
    if let Some(expected) = expected_duration
        && expected > 0.0
        && let Some(actual) = info.duration
        && actual > 0.0
        && actual < expected * 0.9
    {
        issues.push(FixupIssue::TruncatedFile {
            expected_secs: expected,
            actual_secs: actual,
        });
    }

    issues
}

impl FFmpegRunner {
    /// Repair detected issues via a single re-mux pass.
    ///
    /// # Errors
    ///
    /// Returns an error if `FFmpeg` fails to open the input file, create the
    /// output container, or write packets (including I/O errors and mux failures).
    pub async fn fixup_repair(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        issues: &[FixupIssue],
        encoding_tool_override: Option<String>,
    ) -> Result<()> {
        let has_sar_fix = issues
            .iter()
            .any(|i| matches!(i, FixupIssue::StretchedVideo { .. }));
        // Faststart is a property of the container being WRITTEN, so ask about
        // `output`, not `input`.
        //
        // For any input that HAS an extension this matches the previous
        // input-derived answer, because the sole caller builds the output path
        // from the input's extension. For an extensionless input the two differ:
        // the caller defaults to `mp4`, so the file really is written through
        // the mov/mp4 muxer and faststart is correct — where the old
        // input-derived check answered `false` and silently skipped it.
        let faststart = super::options::faststart_for_output(output.as_ref());

        info!(
            "FixupStage: repairing {} issue(s) via re-mux",
            issues.iter().filter(|i| i.is_repairable()).count()
        );

        let opts = RemuxOptions {
            faststart,
            output_format: None,
            encoding_tool_override,
        };

        if has_sar_fix {
            self.fixup_remux_with_sar(input, output, &opts).await
        } else {
            self.remux(input, output, &opts, None).await
        }
    }

    /// Remux with SAR correction (sets output video SAR to 1:1).
    async fn fixup_remux_with_sar(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();

        Self::spawn_blocking("fixup_sar", move || -> Result<()> {
            use super::ensure_init;
            use super::ffi_helpers::cleanup_partial_output;
            use super::log_capture::LogSuppressGuard;

            ensure_init()?;
            let _log_suppress = LogSuppressGuard::error_level();

            let mut ictx = ffmpeg_the_third::format::input(&input)
                .with_context(|| format!("fixup: failed to open {}", input.display()))?;

            let mut octx = ffmpeg_the_third::format::output(&output)
                .with_context(|| format!("fixup: failed to create {}", output.display()))?;

            let stream_count = ictx.streams().count();
            let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
            let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
            let mut ost_index: i32 = 0;

            for (ist_index, ist) in ictx.streams().enumerate() {
                let medium = ist.parameters().medium();
                if !matches!(
                    medium,
                    ffmpeg_the_third::media::Type::Video | ffmpeg_the_third::media::Type::Audio
                ) {
                    continue;
                }

                stream_mapping[ist_index] = ost_index;
                ist_time_bases[ist_index] = ist.time_base();
                ost_index += 1;

                // `format::output` above already created (truncated) `output`
                // on disk via `avio_open` — a codec-tag rejection here must
                // not leave that empty file behind as if a fixup remux had
                // run and produced nothing.
                let ost_idx = Self::add_stream_copy(&mut octx, ist.parameters(), "for fixup")
                    .inspect_err(|_| cleanup_partial_output(&output))?;
                octx.stream_mut(ost_idx)
                    .expect("just-added stream")
                    .set_metadata(ist.metadata().to_owned());

                // Fix SAR on video streams: set to 1:1 (square pixels)
                if medium == ffmpeg_the_third::media::Type::Video
                    && let Some(mut ost) = octx.stream_mut(ost_idx)
                {
                    unsafe {
                        let ost_ptr = ost.as_mut_ptr();
                        (*ost_ptr).sample_aspect_ratio =
                            ffmpeg_the_third::ffi::AVRational { num: 1, den: 1 };
                        (*(*ost_ptr).codecpar).sample_aspect_ratio =
                            ffmpeg_the_third::ffi::AVRational { num: 1, den: 1 };
                    }
                }
            }

            // Copy format-level metadata
            octx.set_metadata(ictx.metadata().to_owned());
            if let Some(ref tag) = opts.encoding_tool_override {
                super::encoding_tag::set_encoding_tool(&mut octx, tag);
            } else {
                super::encoding_tag::set_encoding_tool_if_missing(&mut octx, "fixup");
            }

            let mut dict = ffmpeg_the_third::Dictionary::new();
            if opts.faststart {
                dict.set("movflags", "+faststart");
            }

            octx.write_header_with(dict)
                .context("fixup: failed to write header")?;

            for result in ictx.packets() {
                let (stream, mut packet) = result.context("fixup: failed to read packet")?;
                let ist_index = stream.index();
                let ost_idx = stream_mapping[ist_index];
                if ost_idx < 0 {
                    continue;
                }
                let ost_idx = ost_idx as usize;

                let ost_time_base = octx
                    .stream(ost_idx)
                    .ok_or_else(|| anyhow::anyhow!("fixup: output stream {ost_idx} not found"))?
                    .time_base();
                packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
                packet.set_position(-1);
                packet.set_stream(ost_idx);
                packet
                    .write_interleaved(&mut octx)
                    .context("fixup: failed to write packet")?;
            }

            octx.write_trailer()
                .context("fixup: failed to write trailer")?;

            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use rdlp_types::media_name::CodecName;

    use super::*;
    use crate::ffmpeg::probe::{MediaInfo, StreamInfo};

    fn make_clean_info() -> MediaInfo {
        MediaInfo {
            duration: Some(120.0),
            format: Some("mp4".to_string()),
            has_video: true,
            has_audio: true,
            streams: vec![
                StreamInfo {
                    index: 0,
                    codec_type: StreamKind::Video,
                    codec_name: Some(CodecName::from_static("h264")),
                    duration: Some(120.0),
                    sar_num: Some(1),
                    sar_den: Some(1),
                    nb_frames: Some(3000),
                    ..Default::default()
                },
                StreamInfo {
                    index: 1,
                    codec_type: StreamKind::Audio,
                    codec_name: Some(CodecName::from_static("aac")),
                    duration: Some(120.0),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn no_issues_for_clean_file() {
        let info = make_clean_info();
        let issues = detect_issues(&info, Some(120.0));
        assert!(issues.is_empty());
    }

    #[test]
    fn detects_stretched_sar_zero_num() {
        let mut info = make_clean_info();
        info.streams[0].sar_num = Some(0);
        info.streams[0].sar_den = Some(1);
        let issues = detect_issues(&info, None);
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            issues[0],
            FixupIssue::StretchedVideo {
                stream_index: 0,
                sar_num: 0,
                sar_den: 1
            }
        ));
        assert!(issues[0].is_repairable());
    }

    #[test]
    fn detects_stretched_sar_extreme_ratio() {
        let mut info = make_clean_info();
        info.streams[0].sar_num = Some(100);
        info.streams[0].sar_den = Some(1);
        let issues = detect_issues(&info, None);
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0], FixupIssue::StretchedVideo { .. }));
    }

    #[test]
    fn does_not_flag_normal_sar() {
        let mut info = make_clean_info();
        info.streams[0].sar_num = Some(4);
        info.streams[0].sar_den = Some(3);
        let issues = detect_issues(&info, None);
        assert!(issues.is_empty());
    }

    #[test]
    fn does_not_flag_square_sar() {
        let info = make_clean_info();
        let issues = detect_issues(&info, None);
        assert!(issues.is_empty());
    }

    #[test]
    fn detects_zero_duration_stream() {
        let mut info = make_clean_info();
        info.streams[0].duration = Some(0.0);
        let issues = detect_issues(&info, None);
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            issues[0],
            FixupIssue::ZeroDurationStream {
                stream_index: 0,
                ..
            }
        ));
        assert!(issues[0].is_repairable());
    }

    #[test]
    fn detects_zero_nb_frames() {
        let mut info = make_clean_info();
        info.streams[0].nb_frames = Some(0);
        let issues = detect_issues(&info, None);
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            issues[0],
            FixupIssue::ZeroDurationStream {
                stream_index: 0,
                ..
            }
        ));
    }

    #[test]
    fn detects_truncated_file() {
        let mut info = make_clean_info();
        info.duration = Some(50.0);
        let issues = detect_issues(&info, Some(120.0));
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0], FixupIssue::TruncatedFile { .. }));
        assert!(!issues[0].is_repairable());
    }

    #[test]
    fn no_truncation_when_close_enough() {
        let info = make_clean_info(); // 120s actual
        let issues = detect_issues(&info, Some(130.0)); // 92% > 90%
        assert!(issues.is_empty());
    }

    #[test]
    fn no_truncation_without_expected_duration() {
        let mut info = make_clean_info();
        info.duration = Some(50.0);
        let issues = detect_issues(&info, None);
        assert!(issues.is_empty());
    }

    #[test]
    fn issue_display_stretched() {
        let issue = FixupIssue::StretchedVideo {
            stream_index: 0,
            sar_num: 0,
            sar_den: 1,
        };
        assert_eq!(issue.to_string(), "stream 0: stretched SAR 0:1");
    }

    #[test]
    fn issue_display_truncated() {
        let issue = FixupIssue::TruncatedFile {
            expected_secs: 120.0,
            actual_secs: 50.0,
        };
        assert_eq!(issue.to_string(), "truncated: expected 120.0s, got 50.0s");
    }

    #[test]
    fn unrepairable_issues() {
        assert!(
            !FixupIssue::TruncatedFile {
                expected_secs: 120.0,
                actual_secs: 50.0
            }
            .is_repairable()
        );
        assert!(
            !FixupIssue::UnsupportedCodecForContainer {
                codec: "vp9".into(),
                container: "mp4".into()
            }
            .is_repairable()
        );
    }

    #[test]
    fn repairable_issues() {
        assert!(
            FixupIssue::StretchedVideo {
                stream_index: 0,
                sar_num: 0,
                sar_den: 1
            }
            .is_repairable()
        );
        assert!(FixupIssue::MissingMoovAtom.is_repairable());
        assert!(
            FixupIssue::ZeroDurationStream {
                stream_index: 0,
                codec_type: StreamKind::Video
            }
            .is_repairable()
        );
    }
}
