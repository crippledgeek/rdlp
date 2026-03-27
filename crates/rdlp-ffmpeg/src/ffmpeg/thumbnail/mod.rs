//! Thumbnail embedding into media containers.
//!
//! Container-specific strategies:
//! - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
//! - **MKV/MKA**: Native Matroska attachment via raw FFI
//! - **MP3**: Map audio only + thumbnail as video with ID3v2 metadata
//! - **FLAC/OGG/Opus**: Map all streams + thumbnail with `ATTACHED_PIC`

mod mkv_raw_ffi;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use rdlp_core::PostProcessCallback;

use crate::error::{PostProcessError, Result};

use super::{FFmpegRunner, ensure_init};

impl FFmpegRunner {
    /// Embed a thumbnail image into a media file via stream copy (remux).
    ///
    /// Opens both the media file and thumbnail image, copies all media streams,
    /// and adds the thumbnail as a video stream with `ATTACHED_PIC` disposition.
    /// Container-specific handling for MKV (attachment) and MP3 (ID3v2).
    ///
    /// When `callback` is provided, FFmpeg C-level log messages are captured
    /// and forwarded via [`PostProcessCallback::on_log`] instead of being
    /// suppressed. When `None`, muxer trace is silently suppressed.
    pub async fn embed_thumbnail(
        &self,
        media: impl AsRef<Path>,
        thumbnail: impl AsRef<Path>,
        output: impl AsRef<Path>,
        container: &str,
        callback: Option<Arc<dyn PostProcessCallback>>,
    ) -> Result<()> {
        let media = media.as_ref().to_path_buf();
        let thumbnail = thumbnail.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let container = container.to_string();
        Self::spawn_blocking("embed_thumbnail", move || -> Result<()> {
            Ok(Self::embed_thumbnail_sync(
                &media,
                &thumbnail,
                &output,
                &container,
                callback.as_deref(),
            )?)
        })
        .await
    }

    /// Embed thumbnail synchronously.
    ///
    /// Strategy varies by container:
    /// - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
    /// - **MKV/MKA**: Map all streams + thumbnail as attachment with mimetype metadata
    /// - **MP3**: Map audio only + thumbnail as video with ID3v2 metadata
    /// - **FLAC/OGG/Opus**: Map all streams + thumbnail with `ATTACHED_PIC`
    fn embed_thumbnail_sync(
        media: &Path,
        thumbnail: &Path,
        output: &Path,
        container: &str,
        callback: Option<&dyn PostProcessCallback>,
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

        // MKV: use raw FFI with proper stream property copying for VLC compatibility
        let is_mkv = container.eq_ignore_ascii_case("mkv") || container.eq_ignore_ascii_case("mka");
        if is_mkv {
            let result = Self::embed_thumbnail_mkv_raw_ffi(media, thumbnail, output);
            // Forward captured logs before returning
            Self::forward_captured_logs(&capture, callback);
            return Ok(result?);
        }

        // Open media input
        let mut ictx = ffmpeg_the_third::format::input(media)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to open media input for thumbnail embed {}", media.display()))?;

        // Open thumbnail input
        let mut thumb_ictx = ffmpeg_the_third::format::input(thumbnail)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to open thumbnail {}", thumbnail.display()))?;

        // Create output
        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to create output for thumbnail embed {}", output.display()))?;

        let is_mp3 = container.eq_ignore_ascii_case("mp3");

        // Map media streams to output
        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
        let mut ost_index: i32 = 0;

        for (ist_index, ist) in ictx.streams().enumerate() {
            let medium = ist.parameters().medium();

            // For MP3: only map audio streams (thumbnail replaces any video)
            if is_mp3 && medium != ffmpeg_the_third::media::Type::Audio {
                continue;
            }

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
                .map_err(PostProcessError::from)
                .context("failed to add output stream for thumbnail embed")?;
            ost.set_parameters(ist.parameters());
            Self::clear_codec_tag(ost.parameters().as_ptr());
        }

        // Add thumbnail stream
        let thumb_ist = thumb_ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .ok_or(PostProcessError::ffmpeg_failed(
                "no video stream found in thumbnail",
            ))?;
        let thumb_ist_index = thumb_ist.index();
        let thumb_ist_time_base = thumb_ist.time_base();
        let thumb_params = thumb_ist.parameters();

        // Add thumbnail as video stream with ATTACHED_PIC disposition
        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(PostProcessError::from)
            .context("failed to add thumbnail stream")?;
        let thumb_ost_index = ost.index();
        ost.set_parameters(thumb_params);
        // SAFETY: ost is a valid output stream in a live output context.
        Self::set_attached_pic_disposition(unsafe { ost.as_mut_ptr() });

        // For MP3: set ID3v2 metadata on the thumbnail stream
        if is_mp3 {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("title", "Album cover");
            dict.set("comment", "Cover (Front)");
            ost.set_metadata(dict);
        }

        // Copy format-level metadata from media input
        octx.set_metadata(ictx.metadata().to_owned());

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV: enable faststart (moov atom at beginning) for Windows Explorer thumbnail visibility
        let is_mp4_mov = container.eq_ignore_ascii_case("mp4")
            || container.eq_ignore_ascii_case("m4a")
            || container.eq_ignore_ascii_case("m4v")
            || container.eq_ignore_ascii_case("mov");
        if is_mp4_mov {
            dict.set("movflags", "+faststart");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for thumbnail embed")?;

        // For FLAC/OGG/Opus: write thumbnail packets BEFORE media packets.
        // These formats store picture metadata in the file header (METADATA_BLOCK_PICTURE
        // for FLAC, Vorbis comment for OGG/Opus), so the muxer needs picture data before
        // audio frames are flushed. For other formats (MP4, MP3), order doesn't matter.
        let is_header_picture_format = container.eq_ignore_ascii_case("flac")
            || container.eq_ignore_ascii_case("ogg")
            || container.eq_ignore_ascii_case("opus");

        if is_header_picture_format {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                thumb_ost_index,
            )?;
        }

        // Copy media packets
        for result in ictx.packets() {
            let (stream, mut packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read media packet during thumbnail embed")?;
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
                .context("failed to write media packet during thumbnail embed")?;
        }

        // Copy thumbnail packet(s) for formats that don't need them in the header.
        // MKV: handled by embed_thumbnail_mkv_raw_ffi. FLAC/OGG/Opus: already written above.
        if !is_header_picture_format {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                thumb_ost_index,
            )?;
        }

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for thumbnail embed")?;

        // Forward captured FFmpeg logs to the callback
        Self::forward_captured_logs(&capture, callback);

        Ok(())
    }

    /// Drain captured FFmpeg C-level log messages and forward via callback.
    fn forward_captured_logs(
        capture: &Option<super::log_capture::LogCaptureGuard>,
        callback: Option<&dyn PostProcessCallback>,
    ) {
        if let (Some(guard), Some(cb)) = (capture, callback)
            && let Ok(logs) = guard.take_captured()
        {
            for line in logs {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    cb.on_log(trimmed);
                }
            }
        }
    }
}
