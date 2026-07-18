//! Thumbnail embedding into media containers.
//!
//! Container-specific strategies:
//! - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
//! - **MKV/MKA**: Native Matroska attachment via raw FFI
//! - **MP3**: Map audio only + thumbnail as video with `ID3v2` metadata
//! - **FLAC**: Map all streams + thumbnail as video with `ATTACHED_PIC` (the
//!   FLAC muxer accepts an attached-pic stream and converts it internally to
//!   a native FLAC `PICTURE` block)
//! - **OGG/Opus**: Map all streams; the thumbnail rides a base64
//!   `METADATA_BLOCK_PICTURE` `VorbisComment` field (see [`vorbis_picture`])
//!   instead of a stream — `FFmpeg`'s Ogg muxer hard-rejects any stream whose
//!   codec isn't Vorbis/Theora/Speex/FLAC/Opus/VP8, so an `ATTACHED_PIC`
//!   image stream fails with `Unsupported codec id in stream N` (#531)
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types. All casts are
//!   audited and within valid ranges for `FFmpeg`-returned values.
//! - `clippy::expect_used`: post-construction stream access after `add_stream_copy`
//!   is guaranteed valid by construction.
//! - `clippy::indexing_slicing`: `stream_mapping[ist_index]` and `ist_time_bases[ist_index]`
//!   are pre-allocated to stream count and indexed only during stream iteration.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::similar_names,      // ist_index/ost_index, thumb_ist/thumb_ost are FFmpeg convention
    clippy::option_if_let_else, // ok_or_else pattern with complex closures is clearer as if let
)]

mod embed_strategy;
mod mkv_raw_ffi;
mod vorbis_picture;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use rdlp_core::PostProcessCallback;
use rdlp_types::ContainerFormat;

use crate::error::{PostProcessError, Result};

use self::embed_strategy::ThumbnailEmbedStrategy;
use super::{FFmpegRunner, ensure_init};

impl FFmpegRunner {
    /// Ask the target container's muxer whether it can store the image's codec.
    ///
    /// Every non-Matroska embed strategy stream-copies the thumbnail's codec
    /// into the target container, so the question "does this thumbnail need
    /// transcoding first?" is really "does this muxer have a tag for this
    /// codec?". That is not a fact to hardcode — it is a property of the linked
    /// `FFmpeg` build's muxer tables, and it is the exact lookup that produced
    /// the original failure (`Could not find tag for codec webp in stream #2`).
    /// So ask `FFmpeg` rather than maintaining a parallel whitelist that can
    /// drift from the build.
    ///
    /// Uses `avformat_query_codec`, whose contract is: `1` the codec can be
    /// stored, `0` it cannot, negative when the information is unavailable.
    /// Only an explicit `1` is treated as supported — "unknown" resolves to
    /// "transcode first", the safe direction, since a needless transcode costs
    /// a little work while a wrong answer costs the embed.
    ///
    /// That query is authoritative for tag-table muxers (MP4/MOV) but
    /// *under-reports* elsewhere: muxers that answer through a `query_codec`
    /// callback (mp3 returns the `APIC` tag rather than `1`) or that carry
    /// neither mechanism fall through to `AVERROR_PATCHWELCOME`, reporting
    /// "cannot store" for images they demonstrably do store. JPEG and PNG are
    /// therefore accepted outright as a verified baseline, and the query serves
    /// only to widen that — never to narrow it below what is known to work.
    ///
    /// # Errors
    ///
    /// Returns an error if the image cannot be opened or contains no decodable
    /// image stream. An unrecognized container yields `Ok(false)` rather than
    /// an error — that is a "cannot store it" answer, not a failure.
    pub async fn container_accepts_image_codec(
        &self,
        container: &str,
        image: impl AsRef<Path>,
    ) -> Result<bool> {
        let container = container.to_string();
        let image = image.as_ref().to_path_buf();
        Self::spawn_blocking("container_accepts_image_codec", move || -> Result<bool> {
            Ok(Self::container_accepts_image_codec_sync(
                &container, &image,
            )?)
        })
        .await
    }

    /// Blocking body of [`Self::container_accepts_image_codec`].
    fn container_accepts_image_codec_sync(container: &str, image: &Path) -> anyhow::Result<bool> {
        ensure_init()?;

        let ictx = ffmpeg_the_third::format::input(image)
            .with_context(|| format!("failed to open thumbnail image {}", image.display()))?;
        let stream = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .with_context(|| format!("thumbnail {} contains no image stream", image.display()))?;
        let codec_id = stream.parameters().id();

        // `av_guess_format` resolves a muxer from a filename's extension; the
        // stem is irrelevant and no file is created.
        let probe_name = std::ffi::CString::new(format!("thumbnail.{container}"))
            .context("container extension contained an interior NUL byte")?;

        // SAFETY: `av_guess_format` takes three optional C strings and returns
        // either NULL or a pointer to a muxer descriptor in libavformat's
        // static, compiled-in table — never an allocation the caller owns or
        // frees, and valid for the process lifetime. `probe_name` outlives the
        // call.
        let ofmt = unsafe {
            ffmpeg_the_third::ffi::av_guess_format(
                std::ptr::null(),
                probe_name.as_ptr(),
                std::ptr::null(),
            )
        };

        // An unresolvable container cannot store anything. Checked before the
        // baseline below so "is jpeg embeddable in a container that does not
        // exist?" stays `false` rather than being waved through.
        if ofmt.is_null() {
            return Ok(false);
        }

        // Baseline: JPEG and PNG embed successfully in every container rdlp
        // supports, verified end to end. `avformat_query_codec` does NOT report
        // that — it answers `1` only for muxers carrying a `codec_tag` table,
        // so mp3 (which answers through a `query_codec` callback returning the
        // `APIC` tag), flac, and m4a all report "cannot store" for images they
        // demonstrably do store. Trusting the query alone would re-encode a
        // lossless PNG cover to lossy JPEG on those containers.
        //
        // So the query below only ever WIDENS this baseline; it can never
        // narrow the answer below what is known to work.
        if matches!(
            codec_id,
            ffmpeg_the_third::codec::Id::MJPEG | ffmpeg_the_third::codec::Id::PNG
        ) {
            return Ok(true);
        }

        // SAFETY: `ofmt` is the non-null static descriptor returned above and
        // remains valid for the process lifetime. `avformat_query_codec` only
        // reads that descriptor's codec-tag tables — no allocation, no
        // ownership transfer, no mutation.
        let query = unsafe {
            ffmpeg_the_third::ffi::avformat_query_codec(
                ofmt,
                codec_id.into(),
                ffmpeg_the_third::ffi::FF_COMPLIANCE_NORMAL,
            )
        };

        Ok(query == 1)
    }

    /// Embed a thumbnail image into a media file via stream copy (remux).
    ///
    /// Opens both the media file and thumbnail image, copies all media streams,
    /// and adds the thumbnail as a video stream with `ATTACHED_PIC` disposition.
    /// Container-specific handling for MKV (attachment) and MP3 (`ID3v2`).
    ///
    /// When `callback` is provided, `FFmpeg` C-level log messages are captured
    /// and forwarded via [`PostProcessCallback::on_log`] instead of being
    /// suppressed. When `None`, muxer trace is silently suppressed.
    ///
    /// # Errors
    ///
    /// Returns an error if `FFmpeg` fails to open the media or thumbnail input,
    /// create the output container, or write packets (including I/O errors
    /// and mux failures).
    pub async fn embed_thumbnail(
        &self,
        media: impl AsRef<Path>,
        thumbnail: impl AsRef<Path>,
        output: impl AsRef<Path>,
        container: &str,
        callback: Option<Arc<dyn PostProcessCallback>>,
        encoding_tool_override: Option<String>,
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
                encoding_tool_override.as_deref(),
            )?)
        })
        .await
    }

    /// Embed thumbnail synchronously.
    ///
    /// Strategy varies by container:
    /// - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
    /// - **MKV/MKA**: Map all streams + thumbnail as attachment with mimetype metadata
    /// - **MP3**: Map audio only + thumbnail as video with `ID3v2` metadata
    /// - **FLAC**: Map all streams + thumbnail as video with `ATTACHED_PIC`
    /// - **OGG/Opus**: Map all streams; thumbnail carried as a
    ///   `METADATA_BLOCK_PICTURE` metadata field, no video stream added (#531)
    #[allow(clippy::too_many_lines)]
    fn embed_thumbnail_sync(
        media: &Path,
        thumbnail: &Path,
        output: &Path,
        container: &str,
        callback: Option<&dyn PostProcessCallback>,
        encoding_tool_override: Option<&str>,
    ) -> anyhow::Result<()> {
        ensure_init()?;

        // Parse the container ONCE, at the boundary, into the typed
        // `ContainerFormat` + the strategy it implies — every downstream
        // decision matches on `strategy`, never on a raw string.
        let format = ContainerFormat::from_str(container).map_err(|_| {
            PostProcessError::ffmpeg_failed(format!(
                "unrecognized container '{container}' for thumbnail embed"
            ))
        })?;
        let strategy = ThumbnailEmbedStrategy::for_container(format).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "container '{container}' does not support thumbnail embedding"
            ))
        })?;

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
        if strategy == ThumbnailEmbedStrategy::MatroskaAttachment {
            let result =
                Self::embed_thumbnail_mkv_raw_ffi(media, thumbnail, output, encoding_tool_override);
            // Forward captured logs before returning
            Self::forward_captured_logs(capture.as_ref(), callback);
            return Ok(result?);
        }

        // Open media input
        let mut ictx = ffmpeg_the_third::format::input(media)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open media input for thumbnail embed {}",
                    media.display()
                )
            })?;

        // Open thumbnail input
        let mut thumb_ictx = ffmpeg_the_third::format::input(thumbnail)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to open thumbnail {}", thumbnail.display()))?;

        // Create output
        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to create output for thumbnail embed {}",
                    output.display()
                )
            })?;

        let is_mp3 = strategy == ThumbnailEmbedStrategy::Id3Apic;
        // Ogg/Opus reject any stream whose codec isn't in a small fixed set
        // (Vorbis/Theora/Speex/FLAC/Opus/VP8) at `write_header` time, so the
        // thumbnail must NEVER become a stream for these containers (#531).
        let is_ogg_opus = strategy == ThumbnailEmbedStrategy::VorbisComment;

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

            let ost_idx =
                Self::add_stream_copy(&mut octx, ist.parameters(), "for thumbnail embed")?;
            octx.stream_mut(ost_idx)
                .expect("just-added stream")
                .set_metadata(ist.metadata().to_owned());
        }

        // Read the thumbnail's video stream up front: every strategy below
        // needs its dimensions and/or codec, and the metadata-block-picture
        // strategy needs its raw bytes read before `thumb_ictx` goes out of
        // scope.
        let thumb_ist = thumb_ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .ok_or_else(|| PostProcessError::ffmpeg_failed("no video stream found in thumbnail"))?;
        let thumb_ist_index = thumb_ist.index();
        let thumb_ist_time_base = thumb_ist.time_base();
        let thumb_width = thumb_ist.parameters().width();
        let thumb_height = thumb_ist.parameters().height();

        // Ogg/Opus: carry the cover as a `METADATA_BLOCK_PICTURE` VorbisComment
        // field (see `vorbis_picture`) instead of an `ATTACHED_PIC` stream — no
        // video stream is added for these containers at all (#531).
        let thumb_ost_index = if is_ogg_opus {
            None
        } else {
            let ost_idx =
                Self::add_stream_copy(&mut octx, thumb_ist.parameters(), "for thumbnail")?;
            {
                let mut thumb_ost = octx
                    .stream_mut(ost_idx)
                    .expect("just-added thumbnail stream");
                // SAFETY: thumb_ost is a valid output stream in a live output context.
                Self::set_attached_pic_disposition(unsafe { thumb_ost.as_mut_ptr() });

                // For MP3: set ID3v2 metadata on the thumbnail stream
                if is_mp3 {
                    let mut dict = ffmpeg_the_third::Dictionary::new();
                    dict.set("title", "Album cover");
                    dict.set("comment", "Cover (Front)");
                    thumb_ost.set_metadata(dict);
                }
            }
            Some(ost_idx)
        };

        // Copy format-level metadata from media input
        octx.set_metadata(ictx.metadata().to_owned());
        if let Some(tag) = encoding_tool_override {
            crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, tag);
        } else {
            crate::ffmpeg::encoding_tag::set_encoding_tool_if_missing(&mut octx, "thumbnail");
        }

        // Ogg/Opus: build and set the METADATA_BLOCK_PICTURE tag BEFORE
        // `write_header` — the Ogg muxer copies format-level metadata into
        // the header it writes, and `avformat_write_header` is the point at
        // which a stream-based ATTACHED_PIC would otherwise be rejected.
        if is_ogg_opus {
            // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking
            // from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
            #[allow(clippy::disallowed_methods)]
            let image_bytes = std::fs::read(thumbnail)
                .with_context(|| format!("failed to read thumbnail {}", thumbnail.display()))?;
            let format = rdlp_types::sniff_thumbnail_format(&image_bytes).ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "thumbnail {} is not a recognized image format",
                    thumbnail.display()
                ))
            })?;
            let picture = vorbis_picture::build_metadata_block_picture(
                vorbis_picture::mime_type(format),
                thumb_width,
                thumb_height,
                &image_bytes,
            );
            let mut meta = octx.metadata().to_owned();
            meta.set("METADATA_BLOCK_PICTURE", &picture);
            octx.set_metadata(meta);
        }

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV family: enable faststart (moov atom at beginning) for Windows Explorer thumbnail visibility
        if strategy == ThumbnailEmbedStrategy::Mp4FamilyAttachedPic {
            dict.set("movflags", "+faststart");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for thumbnail embed")?;

        // FLAC: write the thumbnail packet BEFORE media packets. FLAC stores
        // the ATTACHED_PIC stream as a native FLAC PICTURE block in the file
        // header, so the muxer needs the picture data before audio frames are
        // flushed. For other stream-based formats (MP4, MP3), order doesn't
        // matter. Ogg/Opus need no packet at all — their picture already rode
        // into the header as METADATA_BLOCK_PICTURE metadata above.
        let needs_thumbnail_packet_before_header =
            strategy == ThumbnailEmbedStrategy::FlacAttachedPic;

        if needs_thumbnail_packet_before_header && let Some(ost_idx) = thumb_ost_index {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                ost_idx,
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
        // MKV: handled by embed_thumbnail_mkv_raw_ffi. FLAC: already written above.
        // Ogg/Opus: no stream was ever added, so there is no packet to write.
        if !needs_thumbnail_packet_before_header && let Some(ost_idx) = thumb_ost_index {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                ost_idx,
            )?;
        }

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for thumbnail embed")?;

        // Forward captured FFmpeg logs to the callback
        Self::forward_captured_logs(capture.as_ref(), callback);

        Ok(())
    }

    /// Drain captured `FFmpeg` C-level log messages and forward via callback.
    fn forward_captured_logs(
        capture: Option<&super::log_capture::LogCaptureGuard>,
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
