//! Metadata and chapter embedding via stream copy (remux).

use std::collections::HashMap;
use std::path::Path;

use log::debug;

use crate::error::{PostProcessError, Result};

use super::{ChapterEntry, FFmpegRunner, ensure_init};

impl FFmpegRunner {
    /// Embed metadata and chapters into a media file via stream copy (remux).
    ///
    /// Copies all streams without re-encoding, sets format-level metadata via
    /// `Dictionary`, and adds chapters via `add_chapter()`. No temporary
    /// FFMETADATA1 file is needed.
    pub async fn embed_metadata(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let metadata = metadata.clone();
        let chapters = chapters.to_vec();
        Self::spawn_blocking("embed_metadata", move || {
            Self::embed_metadata_sync(&input, &output, &metadata, &chapters)
        })
        .await
    }

    /// Embed metadata and chapters synchronously.
    ///
    /// Remuxes (stream copies) the input to output while:
    /// - Setting format-level metadata from the provided `HashMap`
    /// - Adding chapters with millisecond precision (time_base = 1/1000)
    fn embed_metadata_sync(
        input: &Path,
        output: &Path,
        metadata: &HashMap<String, String>,
        chapters: &[ChapterEntry],
    ) -> Result<()> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
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

            let mut ost = octx
                .add_stream(ffmpeg_the_third::encoder::find(
                    ffmpeg_the_third::codec::Id::None,
                ))
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add output stream: {e}"),
                })?;
            ost.set_parameters(ist.parameters());
            Self::clear_codec_tag(ost.parameters().as_ptr());
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

        // Add chapters (time_base = 1/1000 for millisecond precision)
        for ch in chapters {
            octx.add_chapter(
                ch.id,
                ffmpeg_the_third::Rational(1, 1000),
                ch.start_ms,
                ch.end_ms,
                &ch.title,
            )
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add chapter '{}': {e}", ch.title),
            })?;
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
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
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
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }
}
