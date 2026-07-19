//! Metadata and chapter embedding via stream copy (remux).
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: Chapter start/end times require `i64`→`usize` conversions
//!   for stream time-base arithmetic. Values are validated chapter timestamps
//!   and always within safe ranges.
//! - `clippy::expect_used`: post-construction stream access after `add_stream_copy`
//!   is guaranteed valid by construction.
//! - `clippy::indexing_slicing`: `stream_mapping[ist_index]` pre-allocated to
//!   stream count and only indexed during stream iteration.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use log::debug;
use rdlp_core::PostProcessCallback;

use crate::error::{PostProcessError, Result};

use super::ffi_helpers::cleanup_partial_output;
use super::{ChapterEntry, FFmpegRunner, ensure_init};

impl FFmpegRunner {
    /// Embed metadata and chapters into a media file via stream copy (remux).
    ///
    /// Copies all streams without re-encoding, sets format-level metadata via
    /// `Dictionary`, and adds chapters via `add_chapter()`. No temporary
    /// FFMETADATA1 file is needed.
    ///
    /// When `callback` is provided, `FFmpeg` C-level log messages are captured
    /// and forwarded via [`PostProcessCallback::on_log`] instead of being
    /// suppressed. When `None`, muxer trace is silently suppressed.
    ///
    /// # Errors
    ///
    /// Returns an error if `FFmpeg` fails to open the input file, create the
    /// output container, or write packets (including I/O errors and mux failures).
    pub async fn embed_metadata(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
        callback: Option<Arc<dyn PostProcessCallback>>,
        encoding_tool_override: Option<String>,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let metadata = metadata.clone();
        let chapters = chapters.to_vec();
        Self::spawn_blocking("embed_metadata", move || -> Result<()> {
            Ok(Self::embed_metadata_sync(
                &input,
                &output,
                &metadata,
                &chapters,
                callback.as_deref(),
                encoding_tool_override.as_deref(),
            )?)
        })
        .await
    }

    /// Embed metadata and chapters synchronously.
    ///
    /// Remuxes (stream copies) the input to output while:
    /// - Setting format-level metadata from the provided `HashMap`
    /// - Adding chapters with millisecond precision (`time_base` = 1/1000)
    #[allow(clippy::too_many_lines)]
    fn embed_metadata_sync(
        input: &Path,
        output: &Path,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
        callback: Option<&dyn PostProcessCallback>,
        encoding_tool_override: Option<&str>,
    ) -> anyhow::Result<()> {
        ensure_init()?;

        // When a callback is provided, capture FFmpeg logs and forward them;
        // otherwise suppress muxer trace/debug spam.
        let capture = if callback.is_some() {
            Some(super::log_capture::LogCaptureGuard::begin()?)
        } else {
            None
        };
        let _suppress = if callback.is_none() {
            Some(super::log_capture::LogSuppressGuard::error_level())
        } else {
            None
        };

        let mut ictx = ffmpeg_the_third::format::input(input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open input for metadata embed {}",
                    input.display()
                )
            })?;

        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open output for metadata embed {}",
                    output.display()
                )
            })?;

        // Map all streams (stream copy)
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

            // `format::output` above already created (truncated) `output` on
            // disk via `avio_open` — a codec-tag rejection here must not
            // leave that empty file behind as if a metadata embed had run
            // and produced nothing.
            let ost_idx = Self::add_stream_copy(&mut octx, ist.parameters(), "for metadata embed")
                .inspect_err(|_| cleanup_partial_output(output))?;
            octx.stream_mut(ost_idx)
                .expect("just-added stream")
                .set_metadata(ist.metadata().to_owned());
        }

        // Build metadata dictionary from input metadata + provided overrides
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // Copy existing metadata from input first
        for (k, v) in ictx.metadata().iter() {
            dict.set(k, v);
        }

        // Apply provided metadata (overrides existing keys)
        for (k, v) in metadata {
            dict.set(k, v);
        }

        octx.set_metadata(dict);
        if let Some(tag) = encoding_tool_override {
            crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, tag);
        } else {
            crate::ffmpeg::encoding_tag::set_encoding_tool_if_missing(&mut octx, "metadata");
        }

        // Add chapters (time_base = 1/1000 for millisecond precision)
        for ch in chapters {
            octx.add_chapter(
                ch.id,
                ffmpeg_the_third::Rational(1, 1000),
                ch.start_ms,
                ch.end_ms,
                &ch.title,
            )
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to add chapter '{}'", ch.title))?;
        }

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MKV: set cluster_time_limit for smoother playback/seeking in players like VLC
        let is_mkv = output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            dict.set("cluster_time_limit", "500");
            debug!("MKV detected, setting cluster_time_limit=500ms via dictionary");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for metadata embed")?;

        // Copy packets
        for result in ictx.packets() {
            let (stream, mut packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during metadata embed")?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
            let ost_time_base = octx
                .stream(ost_idx)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("output stream {ost_idx} not found"))
                })?
                .time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
            packet
                .write_interleaved(&mut octx)
                .map_err(PostProcessError::from)
                .context("failed to write packet during metadata embed")?;
        }

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for metadata embed")?;

        // Forward captured FFmpeg logs to the callback
        if let (Some(guard), Some(cb)) = (&capture, callback)
            && let Ok(logs) = guard.take_captured()
        {
            for line in logs {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    cb.on_log(trimmed);
                }
            }
        }

        Ok(())
    }
}
