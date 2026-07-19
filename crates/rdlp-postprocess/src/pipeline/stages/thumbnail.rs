//! `ThumbnailStage` — embeds thumbnail images into media files.
//!
//! This stage runs at index 7 (last) when `config.embed_thumbnail` is true.
//! Uses `msg.original_stem` for thumbnail discovery (not the UUID-renamed stem).
//! Non-fatal: failure logs a warning and passes through.
//!
//! For MP4-family containers: two-pass embedding —
//! 1. `FFmpeg` `attached_pic` stream (media player support)
//! 2. `mp4ameta` iTunes `covr` atom (Windows Explorer visibility)

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info, warn};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};
use rdlp_types::{ContainerFormat, THUMBNAIL_EXTENSIONS, ThumbnailFormat, sniff_thumbnail_format};

use crate::pipeline::{DiscoveredSidecar, PipelineMessage, PipelineStage, SidecarOwnership};

/// Embeds thumbnail into the primary current file.
///
/// `should_run` triggers when `config.embed_thumbnail` is true.
/// Non-fatal: failures push a warning and pass through unchanged.
pub struct ThumbnailStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl ThumbnailStage {
    /// Create a new `ThumbnailStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Find a thumbnail file using `original_stem` for discovery.
    ///
    /// Searches `{parent}/{original_stem}.{ext}` for each `ext` in
    /// [`rdlp_types::THUMBNAIL_EXTENSIONS`]. Also tries the current file's stem
    /// as a fallback.
    fn find_thumbnail(media_file: &Path, original_stem: &str) -> Option<DiscoveredSidecar> {
        let parent = media_file.parent()?;

        // Try original_stem first (most accurate after UUID renames).
        if let Some(path) = THUMBNAIL_EXTENSIONS
            .iter()
            .map(|ext| parent.join(format!("{original_stem}.{ext}")))
            .find(|path| path.exists())
        {
            return Some(DiscoveredSidecar::new(path));
        }

        // Fallback: try current file stem.
        let current_stem = media_file.file_stem()?.to_str()?;
        if current_stem != original_stem {
            return THUMBNAIL_EXTENSIONS
                .iter()
                .map(|ext| parent.join(format!("{current_stem}.{ext}")))
                .find(|path| path.exists())
                .map(DiscoveredSidecar::new);
        }

        None
    }

    /// Whether `extension`'s container can hold an iTunes `covr` metadata
    /// atom via `mp4ameta`.
    ///
    /// This is NOT the same question as thumbnail-embed support
    /// (`rdlp_ffmpeg::supports_thumbnail_embed`) or native-attachment support
    /// (`rdlp_ffmpeg::uses_native_attachment`): `write_covr_atom` never
    /// touches `FFmpeg` at all — it is pure `mp4ameta`, whose real
    /// precondition is a parseable ISO-BMFF `ftyp` atom (`mp4ameta` returns
    /// `ErrorKind::NoFtyp` otherwise). The four containers accepted here
    /// happen to coincide with `rdlp-ffmpeg`'s `Mp4FamilyAttachedPic`
    /// strategy today, but that is incidental, not an identity — so this
    /// predicate is NOT derived from `rdlp-ffmpeg` and must be kept in sync
    /// with `mp4ameta`'s own container support, not `FFmpeg`'s.
    ///
    /// `.mov` is the weak member of this set: `QuickTime`'s native metadata
    /// atom is `udta`, not the iTunes `ilst` atom `mp4ameta` writes, so a
    /// `.mov` file may or may not expose the cover the same way a real `.mp4`
    /// does. The existing call site already hedges for exactly this kind of
    /// uncertainty — `Tag::read_from_path` falls back to `Tag::default()` on
    /// read failure, and a write failure only logs a `warn!` (non-fatal).
    ///
    /// `ThreeGp` is `false` here, but NOT because `mp4ameta` rejects it as a
    /// capability matter — `mp4ameta`'s own `ftyp` parsing
    /// (`atom/ftyp.rs:13`) only errors when the atom is entirely missing, so
    /// a `.3gp` file would very likely be accepted if this predicate were
    /// ever consulted for one. It never is: the `write_covr_atom` call site asks
    /// this question using the POST-REMUX extension, and `ThreeGp` fails
    /// `rdlp_ffmpeg::supports_thumbnail_embed`, so any `.3gp` input is
    /// auto-remuxed to `.mp4` before `supports_covr_atom` is ever reached —
    /// the value can never actually be tested for `ThreeGp`. `false` is kept
    /// here as the conservative, unreachable placeholder rather than `true`,
    /// so a future reader doesn't mistake it for a verified capability claim
    /// and "fix" it to `true` believing they changed real behavior.
    ///
    /// Matched exhaustively over every [`ContainerFormat`] variant (no
    /// catch-all arm), mirroring the discipline `write_covr_atom`'s own
    /// `ThumbnailFormat` match already uses: a newly-added container format
    /// fails to compile here, forcing an explicit decision about whether
    /// `mp4ameta` can represent it, rather than silently inheriting `false`
    /// (or, worse, `true`) from a catch-all.
    const fn supports_covr_atom(format: ContainerFormat) -> bool {
        match format {
            ContainerFormat::Mp4
            | ContainerFormat::Mov
            | ContainerFormat::M4v
            | ContainerFormat::M4a => true,
            ContainerFormat::Mkv
            | ContainerFormat::WebM
            | ContainerFormat::Ts
            | ContainerFormat::Flv
            | ContainerFormat::Avi
            | ContainerFormat::ThreeGp
            | ContainerFormat::Mpg
            | ContainerFormat::F4v
            | ContainerFormat::Asf
            | ContainerFormat::Mxf
            | ContainerFormat::Vob
            | ContainerFormat::Dv
            | ContainerFormat::Nut
            | ContainerFormat::Ivf
            | ContainerFormat::Ogg
            | ContainerFormat::Mp3
            | ContainerFormat::Wav
            | ContainerFormat::Flac
            | ContainerFormat::Opus
            | ContainerFormat::Aac
            | ContainerFormat::Aiff
            | ContainerFormat::Mka
            | ContainerFormat::Wv
            | ContainerFormat::Caf
            | ContainerFormat::Ac3 => false,
        }
    }

    /// Whether `extension`'s container attaches the thumbnail's source codec
    /// natively (Matroska), and therefore never needs image normalization.
    ///
    /// Thin wrapper over `rdlp_ffmpeg::uses_native_attachment` — the
    /// production-code gateway to the real strategy table (#533). An
    /// unparseable extension answers `false`, the safe direction: it routes
    /// into the normal transcode-to-jpeg path rather than assuming native
    /// support for a container `rdlp-ffmpeg` doesn't recognize at all.
    fn is_native_attachment(extension: &str) -> bool {
        ContainerFormat::from_str(extension).is_ok_and(rdlp_ffmpeg::uses_native_attachment)
    }

    /// Whether `extension`'s container can carry an embedded thumbnail at all.
    /// Thin wrapper over `rdlp_ffmpeg::supports_thumbnail_embed` (#533).
    fn supports_thumbnail(extension: &str) -> bool {
        ContainerFormat::from_str(extension).is_ok_and(rdlp_ffmpeg::supports_thumbnail_embed)
    }

    /// Normalize `thumbnail_file` to a tracker-owned temp `.jpg` when the
    /// target container can't consume its codec directly.
    ///
    /// Matroska (`rdlp_ffmpeg::uses_native_attachment`) is handled entirely
    /// separately (see [`Self::mkv_attachment_renders_natively`]): it carries
    /// the image as a file attachment rather than a stream, so the
    /// stream-codec-tag question below does not govern it at all.
    /// `jpeg`/`png`/`gif`/`tiff` attachments
    /// render natively and return early untouched; `bmp`/`webp` are not
    /// recognized by `FFmpeg`'s own attachment-mimetype read-back table
    /// (`ThumbnailFormat::matroska_attachment`) and are unconditionally
    /// transcoded to jpeg first — producing an invisible, non-rendering cover
    /// otherwise (#530). This branch never consults
    /// `container_accepts_image_codec`, which answers an unrelated
    /// stream-codec-tag question that does not apply to attachments.
    ///
    /// MP3 gets the same conservative treatment as a deliberate *policy*
    /// carve-out, layered on top of (not a correction to) the capability
    /// query below — see [`Self::mp3_apic_renders_natively`] for why "the
    /// muxer can store it" and "a player renders it" are different
    /// questions for `ID3v2` `APIC` covers, same as they were for Matroska
    /// attachments in #530.
    ///
    /// Every non-Matroska embed strategy stream-copies the thumbnail's source
    /// codec into the target container (see `rdlp_ffmpeg::thumbnail`), so a
    /// codec the target muxer has no tag for (e.g. `webp` in MP4) must be
    /// transcoded first or the mux fails. Whether a tag exists is asked of
    /// `FFmpeg` via `container_accepts_image_codec` rather than hardcoded, so
    /// the answer always tracks the linked build instead of a list that can
    /// drift from it.
    ///
    /// On transcode failure, falls back to the original thumbnail path so the
    /// caller's existing non-fatal embed-failure handling still applies —
    /// which is why downstream steps re-check the bytes rather than assuming
    /// this returned something normalized.
    async fn normalize_thumbnail_for_embed(
        &self,
        msg: &mut PipelineMessage,
        extension: &str,
        thumbnail_file: &Path,
    ) -> PathBuf {
        if Self::is_native_attachment(extension) {
            return if Self::mkv_attachment_renders_natively(thumbnail_file).await {
                thumbnail_file.to_path_buf()
            } else {
                self.transcode_thumbnail_to_jpg(msg, extension, thumbnail_file)
                    .await
            };
        }

        // MP3 policy carve-out (#549 follow-up): `container_accepts_image_codec`'s
        // `> 0` fix (#549) makes mp3's own `query_codec` callback (`mp3enc.c`)
        // correctly report every image mime it lists — gif/jpeg/png/tiff/bmp/webp
        // — as representable, since it answers with the shared `APIC` tag
        // rather than a bare `1`. That answers "can the muxer store these
        // bytes", not "will an `ID3v2` reader display them as a cover". #530
        // found exactly that gap for Matroska (bmp/webp attach fine, render
        // invisible); no equivalent verification exists for `ID3v2` `APIC`
        // readers, so mp3 mirrors the same conservative policy rather than
        // trusting the widened capability query at face value.
        if ContainerFormat::from_str(extension) == Ok(ContainerFormat::Mp3)
            && !Self::mp3_apic_renders_natively(thumbnail_file).await
        {
            return self
                .transcode_thumbnail_to_jpg(msg, extension, thumbnail_file)
                .await;
        }

        // Ask the muxer whether it can carry this image's codec, rather than
        // consulting a hardcoded list (#525). This is the same codec-tag lookup
        // that produced the original failure, and it reads the image's real
        // codec — FFmpeg probes content, so a mislabeled `.jpg` holding webp is
        // identified as webp here regardless of its name. An error (unopenable
        // or undecodable image) answers "not accepted", so normalization, the
        // safe direction, is the default.
        if self
            .ffmpeg
            .container_accepts_image_codec(extension, thumbnail_file)
            .await
            .unwrap_or(false)
        {
            return thumbnail_file.to_path_buf();
        }

        self.transcode_thumbnail_to_jpg(msg, extension, thumbnail_file)
            .await
    }

    /// Whether `thumbnail_file`'s content is a format this policy treats as a
    /// verified-safe mp3 `ID3v2` `APIC` cover — a **conservative product
    /// decision**, not an `FFmpeg`-verified capability like
    /// [`Self::mkv_attachment_renders_natively`]'s Matroska read-back table.
    ///
    /// Only `jpeg`/`png` are accepted; everything else is normalized.
    ///
    /// ID3v2.3 §4.15 / ID3v2.4 §4.14 (id3.org): *"The 'image/png' or
    /// 'image/jpeg' picture format should be used when interoperability is
    /// wanted."* Advisory rather than mandatory, but every maintained reader
    /// converges on it — `TagLib`'s `AttachedPictureFrame` docs restate it
    /// verbatim, `mutagen` and `jaudiotagger` expose only JPEG/PNG MIME
    /// constants, and `Mp3tag`'s "Adjust Cover" offers exactly Original,
    /// JPEG, PNG as conversion targets. No surveyed tool treats `gif`/`tiff`
    /// as a safer tier than `bmp`/`webp`.
    ///
    /// An earlier version of this predicate did, passing `gif`/`tiff`
    /// through. That tier was imported from `FFmpeg`'s `matroskadec.c`
    /// `mkv_image_mime_tags[]` (#530), which is a Matroska **decoder** table
    /// resolving attachments into `attached_pic` streams. The mp3 muxer has
    /// no analogue: it writes whatever MIME string and bytes it is handed,
    /// so acceptance rests entirely on third-party `ID3v2` readers that have
    /// no such table. The carve-out does not transfer between the two
    /// mechanisms, so it is not reused here.
    ///
    /// `FFmpeg` muxes all six formats as valid, correctly-typed `APIC`
    /// frames — verified, including read-back by a non-`FFmpeg` parser. That
    /// establishes well-formedness, not that a player decodes the payload;
    /// #530 is the precedent for those being different questions. A read
    /// failure or unrecognized signature answers `false`, the same safe
    /// direction.
    async fn mp3_apic_renders_natively(thumbnail_file: &Path) -> bool {
        let Ok(bytes) = tokio::fs::read(thumbnail_file).await else {
            return false;
        };
        matches!(
            rdlp_types::sniff_thumbnail_format(&bytes),
            Some(ThumbnailFormat::Jpeg | ThumbnailFormat::Png)
        )
    }

    /// Transcode `thumbnail_file` to a tracker-owned temp `.jpg`, falling
    /// back to the original path on failure (non-fatal — the caller's
    /// existing embed-failure handling still applies).
    ///
    /// Shared by both branches of [`Self::normalize_thumbnail_for_embed`] so
    /// the transcode-and-fallback logic exists in exactly one place.
    async fn transcode_thumbnail_to_jpg(
        &self,
        msg: &mut PipelineMessage,
        extension: &str,
        thumbnail_file: &Path,
    ) -> PathBuf {
        let normalized = msg.tracker.temp_path(thumbnail_file, "jpg");
        debug!(
            "ThumbnailStage: normalizing thumbnail {} → jpg for {extension} embed",
            thumbnail_file.display()
        );
        match self
            .ffmpeg
            .transcode_image(thumbnail_file, &normalized)
            .await
        {
            Ok(()) => {
                msg.tracker.mark_temp(normalized.clone());
                normalized
            }
            Err(e) => {
                // Mark the (possibly partial) temp jpg for cleanup on the success
                // path too, mirroring the auto-remux-failure path below — otherwise
                // a failed-transcode partial lingers until the TempRegistry sweep.
                msg.tracker.mark_temp(normalized);
                warn!("ThumbnailStage: thumbnail normalization to jpg failed, using original: {e}");
                msg.warnings
                    .push(format!("Thumbnail normalization to jpg failed: {e}"));
                thumbnail_file.to_path_buf()
            }
        }
    }

    /// Whether `thumbnail_file`'s content is a format the linked `FFmpeg`
    /// build's Matroska read-back recognizes as a real, player-visible cover
    /// (see [`ThumbnailFormat::matroska_attachment`]), so the native
    /// attachment path can carry it as-is with no normalization.
    ///
    /// A read failure or an unrecognized signature answers `false` — the
    /// safe direction, since it routes into the normal transcode-to-jpeg
    /// path below rather than risking a silently non-rendering attachment.
    ///
    /// This reads `thumbnail_file` a second time when the embed later
    /// reaches `rdlp_ffmpeg`'s raw-FFI path (which re-reads + re-sniffs the
    /// same bytes to decide the attachment mimetype, see
    /// `mkv_raw_ffi.rs`). Not threaded through: `embed_thumbnail`'s public
    /// signature is shared by every container strategy (MP4/MP3/FLAC/OGG/
    /// Matroska), most of which never sniff at all, so plumbing pre-read
    /// bytes through it would widen that boundary for one container's
    /// benefit. Thumbnails are small (single-frame stills), so the extra
    /// read is a few KB/µs, not a hot path — not worth the API contortion.
    async fn mkv_attachment_renders_natively(thumbnail_file: &Path) -> bool {
        let Ok(bytes) = tokio::fs::read(thumbnail_file).await else {
            return false;
        };
        sniff_thumbnail_format(&bytes).is_some_and(|format| format.matroska_attachment().is_some())
    }

    /// Write the iTunes `covr` metadata atom for Windows Explorer thumbnail visibility.
    ///
    /// Non-fatal: logs a warning on failure.
    async fn write_covr_atom(media_file: &Path, thumbnail_file: &Path) {
        let media = media_file.to_path_buf();
        let thumb = thumbnail_file.to_path_buf();

        let result = tokio::task::spawn_blocking(move || {
            // Safe: inside spawn_blocking closure — explicitly the correct place for blocking I/O.
            #[allow(clippy::disallowed_methods)]
            let cover_bytes =
                std::fs::read(&thumb).context("thumbnail stage: failed to read thumbnail file")?;
            // Pick the covr image type from the BYTES, never the extension.
            // The previous extension-based branch labelled anything not named
            // `.png` as JPEG, so a `.jpg` file holding webp bytes was written
            // into the atom tagged as JPEG — precisely the mislabel #519 set
            // out to prevent, reachable again through #525's misnamed sidecar.
            // A `debug_assert` on the extension could not catch it either,
            // since the extension was the thing that lied.
            //
            // Content outside mp4ameta's supported rasters is refused rather
            // than mislabeled. This is non-fatal: the FFmpeg `attached_pic`
            // pass has already embedded the thumbnail, so skipping covr costs
            // Windows Explorer visibility, not the thumbnail itself.
            //
            // The arms are listed exhaustively rather than with a catch-all so
            // that adding a ThumbnailFormat variant fails to compile here,
            // forcing a decision about whether mp4ameta can represent it. A
            // string match could not do that — which is how the previous
            // coupling to the embeddable set was lost silently.
            let img = match rdlp_types::sniff_thumbnail_format(&cover_bytes) {
                Some(ThumbnailFormat::Jpeg) => mp4ameta::Img::jpeg(cover_bytes),
                Some(ThumbnailFormat::Png) => mp4ameta::Img::png(cover_bytes),
                Some(ThumbnailFormat::Bmp) => mp4ameta::Img::bmp(cover_bytes),
                Some(
                    format @ (ThumbnailFormat::Gif | ThumbnailFormat::Tiff | ThumbnailFormat::WebP),
                ) => anyhow::bail!(
                    "thumbnail stage: covr atom cannot represent {} content",
                    format.extension()
                ),
                None => anyhow::bail!(
                    "thumbnail stage: covr atom requires a recognized image, found \
                     unrecognized data"
                ),
            };
            let mut tag =
                mp4ameta::Tag::read_from_path(&media).unwrap_or_else(|_| mp4ameta::Tag::default());
            tag.set_artwork(img);
            tag.write_to_path(&media)
                .context("thumbnail stage: failed to write covr atom to media file")?;
            Ok::<(), anyhow::Error>(())
        })
        .await;

        match result {
            Ok(Ok(())) => debug!("ThumbnailStage: MP4 covr atom written"),
            Ok(Err(e)) => warn!("ThumbnailStage: failed to write covr atom: {e}"),
            Err(e) => warn!("ThumbnailStage: covr atom task panicked: {e}"),
        }
    }
}

#[async_trait]
impl PipelineStage for ThumbnailStage {
    fn name(&self) -> &'static str {
        "ThumbnailStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.embed_thumbnail
    }

    fn is_fatal(&self) -> bool {
        false
    }

    #[allow(clippy::too_many_lines)]
    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let media_file = msg.tracker.primary();
        let extension: String = media_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        // Auto-remux containers that don't support thumbnail embedding (e.g. .ts → .mp4).
        // The gate is the REAL strategy table in `rdlp-ffmpeg`
        // (`ContainerFormat::from_str` + `supports_thumbnail_embed`), not a
        // hand-copied string list — full consolidation of what used to be
        // two independently-maintained container lists (#533). This widens
        // accepted input over the old list: `ContainerFormat` is
        // ascii-case-insensitive WITH ALIASES (`"matroska"` → `Mkv`,
        // `"quicktime"` → `Mov`), so a `.matroska`/`.quicktime` extension now
        // takes the embed path instead of auto-remuxing to mp4. See
        // `matroska_and_quicktime_aliases_now_resolve_to_embed_support` for the
        // pinned behavior.
        let supports_thumbnail = Self::supports_thumbnail(&extension);

        // #548/#551: an explicitly requested container must never be silently
        // discarded by the auto-remux-to-mp4 fallback below. The request is
        // resolved by `PostProcess::explicit_container` (the single source of
        // truth for the `recode_container` > `recode_video` > `remux_container`
        // precedence chain) — #548 keyed this guard on `remux_container` alone,
        // which is why `--recode-video=ts` still lost its container (#551).
        //
        // `None` means rdlp picked the container itself (e.g. post-HLS `.ts`
        // with no explicit flag), where the auto-remux-to-mp4 fallback is
        // correct and unaffected by this guard.
        //
        // The comparison is EQUALITY against the container actually on disk,
        // not `.is_some()`: an explicit request for a DIFFERENT container must
        // still take the auto-remux path. Compared as `ContainerFormat`, never
        // a raw extension string.
        if !supports_thumbnail
            && let Ok(current) = ContainerFormat::from_str(&extension)
            && let Some(explicit) = msg.config.explicit_container()
            && explicit.format == current
        {
            let flag = explicit.source.setting_name();
            let reason = format!(
                "kept explicit {flag}={current} container; thumbnail embed skipped \
                 because {current} cannot carry an embedded thumbnail"
            );
            warn!("ThumbnailStage: {reason}");
            msg.warnings.push(reason);

            // This early return bypasses the normal embed path below, which is
            // the ONLY other place the orchestrator-downloaded thumbnail
            // sidecar is marked temp for cleanup (the embed-success path's own
            // `thumbnail_sidecar.into_disposable(..)` below, guarded on the
            // same `!write_thumbnail` condition). Without this, every run
            // that hits this guard with the default `--write-thumbnail=false`
            // leaves a stray sidecar image next to the kept-container output.
            if !msg.config.write_thumbnail
                && let Some(sidecar) = Self::find_thumbnail(&media_file, &msg.original_stem)
                && let Some(path) = sidecar.into_disposable(SidecarOwnership::of(&msg))
            {
                msg.tracker.mark_temp(path);
            }

            return Ok(msg);
        }

        let (media_file, extension) = if supports_thumbnail {
            (media_file, extension)
        } else {
            let remuxed_path = msg.tracker.temp_path(&media_file, "mp4");
            debug!(
                "ThumbnailStage: auto-remuxing {} → mp4 for thumbnail embedding",
                media_file.display()
            );

            let opts = RemuxOptions {
                faststart: true,
                encoding_tool_override: msg.encoding_tool.clone(),
                ..Default::default()
            };
            match self
                .ffmpeg
                .remux(&media_file, &remuxed_path, &opts, None)
                .await
            {
                Ok(()) => {
                    msg.tracker.replace(vec![remuxed_path.clone()]);
                    (remuxed_path, "mp4".to_string())
                }
                Err(e) => {
                    warn!("ThumbnailStage: auto-remux to MP4 failed, skipping: {e}");
                    msg.warnings
                        .push(format!("Auto-remux for thumbnail embedding failed: {e}"));
                    msg.tracker.mark_temp(remuxed_path);
                    return Ok(msg);
                }
            }
        };

        // Use original_stem for thumbnail discovery (per architecture constraint 5).
        let Some(thumbnail_sidecar) = Self::find_thumbnail(&media_file, &msg.original_stem) else {
            debug!(
                "ThumbnailStage: no thumbnail file found for stem '{}'",
                msg.original_stem
            );
            msg.warnings.push(format!(
                "Thumbnail file not found for '{}'",
                msg.original_stem
            ));
            return Ok(msg);
        };

        info!(
            "ThumbnailStage: embedding thumbnail {} into {}",
            thumbnail_sidecar.path().display(),
            media_file.display()
        );

        let embed_source = self
            .normalize_thumbnail_for_embed(&mut msg, &extension, thumbnail_sidecar.path())
            .await;

        let temp_output = msg.tracker.temp_path(&media_file, &extension);

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_forwarder = stage_callback.as_ref().map(|cb| {
            let cb = cb.clone();
            rdlp_ffmpeg::LogForwarderGuard::new(std::sync::Arc::new(
                move |level: i32, msg: String| {
                    let trimmed = msg.trim_end();
                    if trimmed.is_empty() {
                        return;
                    }
                    let prefixed = match level {
                        l if l <= 16 => format!("[ERROR] {trimmed}"),
                        24 => format!("[WARN] {trimmed}"),
                        _ => trimmed.to_string(),
                    };
                    cb.on_log(&prefixed);
                },
            ))
        });
        let log_callback = if msg.verbose { stage_callback } else { None };

        match self
            .ffmpeg
            .embed_thumbnail(
                &media_file,
                &embed_source,
                &temp_output,
                &extension,
                log_callback,
                msg.encoding_tool.clone(),
            )
            .await
        {
            Ok(()) => {
                debug!(
                    "ThumbnailStage: thumbnail embedded via FFmpeg: {}",
                    media_file.display()
                );

                // Promote output — old file becomes temp.
                msg.tracker.replace(vec![temp_output.clone()]);

                // For MP4-family: write covr atom for Windows Explorer.
                // `embed_source` is USUALLY normalized by
                // `normalize_thumbnail_for_embed` above — but not guaranteed:
                // that helper falls back to the original file when the
                // transcode fails. `write_covr_atom` therefore re-checks the
                // bytes itself and refuses what mp4ameta cannot represent,
                // rather than trusting an invariant that can be violated.
                let supports_covr =
                    ContainerFormat::from_str(&extension).is_ok_and(Self::supports_covr_atom);
                if supports_covr {
                    Self::write_covr_atom(&temp_output, &embed_source).await;
                }

                // Clean up thumbnail unless --write-thumbnail was requested —
                // and never when it is the user's own file sitting next to a
                // borrowed input (see `SidecarOwnership`).
                if !msg.config.write_thumbnail
                    && let Some(path) =
                        thumbnail_sidecar.into_disposable(SidecarOwnership::of(&msg))
                {
                    msg.tracker.mark_temp(path);
                }
            }
            Err(e) => {
                warn!("ThumbnailStage: failed to embed thumbnail: {e}");
                msg.warnings
                    .push(format!("Thumbnail embedding failed: {e}"));
                msg.tracker.mark_temp(temp_output);

                // The sidecar needs the same disposal as on the success path.
                // Marking only `temp_output` here left an rdlp-downloaded
                // thumbnail on disk whenever the embed failed — a disk leak
                // (found by the pre-push security review of #553), relying on
                // `TempRegistry`'s stale sweep to eventually collect it.
                // Ownership still governs: a user's own file is retained,
                // failure or not.
                if !msg.config.write_thumbnail
                    && let Some(path) =
                        thumbnail_sidecar.into_disposable(SidecarOwnership::of(&msg))
                {
                    msg.tracker.mark_temp(path);
                }
            }
        }

        Ok(msg)
    }
}

#[cfg(test)]
// Safe: test fixtures — no async runtime in #[test] fns.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test Video".to_string(),
                "TestExtractor".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_when_embed_thumbnail() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcess {
            embed_thumbnail: true,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);
        let config = PostProcess {
            embed_thumbnail: false,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_not_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);
        assert!(!stage.is_fatal());
    }

    #[test]
    fn supports_thumbnail_containers() {
        assert!(ThumbnailStage::supports_thumbnail("mp4"));
        assert!(ThumbnailStage::supports_thumbnail("mkv"));
        assert!(ThumbnailStage::supports_thumbnail("mp3"));
        assert!(ThumbnailStage::supports_thumbnail("flac"));
        assert!(!ThumbnailStage::supports_thumbnail("ts"));
        assert!(!ThumbnailStage::supports_thumbnail("avi"));
    }

    /// Behavior-widening regression pin (#533): `ContainerFormat` is
    /// `#[strum(ascii_case_insensitive)]` WITH ALIASES — `"matroska"` parses
    /// to `Mkv` and `"quicktime"` parses to `Mov`. The old hand-copied
    /// `SUPPORTED_CONTAINERS` string list only ever matched the short
    /// extensions (`"mkv"`, `"mov"`), so `.matroska`/`.quicktime` files
    /// previously fell into the ".ts → .mp4"-style auto-remux branch. Under
    /// the typed gate they now resolve directly to the embed path instead.
    /// A `.matroska`/`.quicktime` file extension is unrealistic in practice
    /// (real files use `.mkv`/`.mov`), and taking the embed path is safe —
    /// but it is a real, deliberate behavior change, pinned here rather than
    /// left as an unstated side effect of the migration.
    #[test]
    fn matroska_and_quicktime_aliases_now_resolve_to_embed_support() {
        assert!(
            ThumbnailStage::supports_thumbnail("matroska"),
            "'matroska' aliases to ContainerFormat::Mkv and must now take the embed path"
        );
        assert!(
            ThumbnailStage::supports_thumbnail("quicktime"),
            "'quicktime' aliases to ContainerFormat::Mov and must now take the embed path"
        );
    }

    /// Every container the real `rdlp-ffmpeg` strategy table resolves an
    /// embed strategy for must be reachable via `supports_thumbnail` — this
    /// is the identity the typed gate is now defined by, so it holds by
    /// construction; kept as an explicit assertion so a future change to
    /// either side that breaks the identity is caught here.
    #[test]
    fn every_strategy_supported_format_is_reachable() {
        use strum::IntoEnumIterator as _;

        for format in ContainerFormat::iter() {
            if rdlp_ffmpeg::supports_thumbnail_embed(format) {
                assert!(
                    ThumbnailStage::supports_thumbnail(format.as_ext()),
                    "'{}' resolves to a thumbnail strategy but supports_thumbnail returned false",
                    format.as_ext()
                );
            }
        }
    }

    #[test]
    fn supports_covr_atom_accepts_mp4_family() {
        assert!(ThumbnailStage::supports_covr_atom(ContainerFormat::Mp4));
        assert!(ThumbnailStage::supports_covr_atom(ContainerFormat::M4a));
        assert!(ThumbnailStage::supports_covr_atom(ContainerFormat::M4v));
        assert!(ThumbnailStage::supports_covr_atom(ContainerFormat::Mov));
        // Behavior-widening regression pin (#533): "quicktime" aliases to
        // ContainerFormat::Mov via `from_str`, so a `.quicktime` extension
        // now also resolves to covr-atom support, same as `.mov`.
        assert!(
            ContainerFormat::from_str("quicktime").is_ok_and(ThumbnailStage::supports_covr_atom),
            "'quicktime' aliases to ContainerFormat::Mov and must resolve to covr-atom support"
        );
    }

    /// Negative: Matroska is rejected (different atom mechanism entirely —
    /// `covr` is an iTunes/ISO-BMFF concept, Matroska carries attachments).
    #[test]
    fn supports_covr_atom_rejects_mkv() {
        assert!(!ThumbnailStage::supports_covr_atom(ContainerFormat::Mkv));
    }

    /// Negative: MP3 is rejected — it uses `ID3v2` APIC, not a covr atom.
    #[test]
    fn supports_covr_atom_rejects_mp3() {
        assert!(!ThumbnailStage::supports_covr_atom(ContainerFormat::Mp3));
    }

    /// Negative, and the load-bearing case for this predicate's existence:
    /// `F4v` is ISO-BMFF (an MP4 variant — `ContainerFormat::
    /// supports_faststart` groups it with `Mp4`/`Mov`/`M4v`) yet is NOT
    /// covr-gated today (nor, independently, does `rdlp-ffmpeg`'s
    /// `ThumbnailEmbedStrategy` resolve a thumbnail-embed strategy for it at
    /// all). This pins current behavior explicitly so a future change to
    /// widen `supports_covr_atom` to "any ISO-BMFF container" is a
    /// deliberate decision, not an accidental consequence of conflating the
    /// covr-atom axis with `rdlp-ffmpeg`'s embed-strategy axis — the two
    /// questions are independent, which is the entire point of keeping this
    /// predicate un-derived from `rdlp-ffmpeg`.
    #[test]
    fn supports_covr_atom_rejects_f4v_despite_iso_bmff_kinship() {
        assert!(!ThumbnailStage::supports_covr_atom(ContainerFormat::F4v));
    }

    #[test]
    fn find_thumbnail_returns_none_for_missing() {
        let result = ThumbnailStage::find_thumbnail(
            &PathBuf::from("/nonexistent/video.mp4"),
            "original-title",
        );
        assert!(result.is_none());
    }

    /// #521: discovery must locate the widened raster formats
    /// (`gif`/`bmp`/`tiff`), not only the original `jpg`/`jpeg`/`png`/`webp`.
    /// Fails against the pre-#521 list, which stopped at `webp` and so returned
    /// `None` for a `gif`/`bmp`/`tiff` sidecar (silent thumbnail skip).
    #[test]
    fn find_thumbnail_finds_widened_raster_formats() {
        use std::fs;
        use tempfile::TempDir;

        for ext in ["gif", "bmp", "tiff", "tif"] {
            let dir = TempDir::new().unwrap();
            let thumb = dir.path().join(format!("clip.{ext}"));
            fs::write(&thumb, b"fake-image").unwrap();

            let media = dir.path().join("clip.rdlp-tmp-abc123.mp4");
            let result = ThumbnailStage::find_thumbnail(&media, "clip");
            assert_eq!(
                result.map(|s| s.path().to_path_buf()),
                Some(thumb),
                "find_thumbnail must discover a .{ext} sidecar via original_stem"
            );
        }
    }

    #[test]
    fn find_thumbnail_finds_by_original_stem() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let thumb = dir.path().join("original-title.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        let media = dir.path().join("original-title.rdlp-tmp-abc123.mp4");
        let result = ThumbnailStage::find_thumbnail(&media, "original-title");
        assert_eq!(result.map(|s| s.path().to_path_buf()), Some(thumb));
    }

    /// Slice 2 (#406 Task 3): `original_stem` is the CLEAN stem (marker-stripped
    /// by the orchestrator), while the main file in the tracker is seam-named
    /// (`My.Video.rdlp-tmp-<uuid>.mp4`). `find_thumbnail` must resolve the
    /// sidecar `My.Video.jpg` via `original_stem`, NOT via the seam-named stem
    /// (which would produce `My.Video.rdlp-tmp-<uuid>.jpg` — a non-existent path).
    #[test]
    fn thumbnail_discovery_uses_clean_original_stem_under_seam() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Thumbnail is written next to the clean stem, as the orchestrator produces.
        let thumb = dir.path().join("My.Video.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        // The media file on disk is seam-named (as FileTracker sees it at this stage).
        let seam_media = dir
            .path()
            .join("My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678.mp4");
        // original_stem carries the clean stem — set by Task 3 in the orchestrator.
        let original_stem = "My.Video";

        let result = ThumbnailStage::find_thumbnail(&seam_media, original_stem);
        assert_eq!(
            result.map(|s| s.path().to_path_buf()),
            Some(thumb),
            "find_thumbnail must find My.Video.jpg via original_stem \
             even when the media file is seam-named"
        );
    }

    /// Negative companion to `thumbnail_discovery_uses_clean_original_stem_under_seam`:
    /// pins the regression direction. With the same clean sidecar `My.Video.jpg` on disk
    /// and the same seam-named media file, but `original_stem` set to the SEAM stem itself
    /// (what an unfixed Task 3 would have done), `find_thumbnail` returns `None` because:
    /// - Primary lookup tries `{seam_stem}.jpg` (doesn't exist; the sidecar is `My.Video.jpg`)
    /// - Fallback skips because `current_stem` == `original_stem` (both are the seam stem)
    /// - Result: `None`
    #[test]
    fn thumbnail_discovery_misses_when_original_stem_is_seam_stem() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Sidecar is still written with the clean stem.
        let thumb = dir.path().join("My.Video.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        // The media file on disk is seam-named.
        let seam_media = dir
            .path()
            .join("My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678.mp4");
        // But original_stem is SET TO THE SEAM STEM itself (the regression).
        let original_stem = "My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678";

        let result = ThumbnailStage::find_thumbnail(&seam_media, original_stem);
        assert_eq!(
            result, None,
            "find_thumbnail must return None when original_stem is the seam stem \
             (because the sidecar My.Video.jpg exists but lookup tries My.Video.rdlp-tmp-*.jpg)"
        );
    }

    #[tokio::test]
    async fn process_warns_when_no_thumbnail_found() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcess {
            embed_thumbnail: true,
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        msg.original_stem = "nonexistent-stem".to_string();

        let result = stage.process(msg).await.unwrap();
        assert!(
            !result.warnings.is_empty(),
            "expected warning about missing thumbnail"
        );
    }

    #[test]
    fn native_attachment_container_accepts_mkv_mka() {
        assert!(ThumbnailStage::is_native_attachment("mkv"));
        assert!(ThumbnailStage::is_native_attachment("mka"));
        // Behavior-widening regression pin (#533): "matroska" aliases to
        // ContainerFormat::Mkv, so a `.matroska` file now also skips
        // normalization via the native-attachment path, same as `.mkv`.
        assert!(
            ThumbnailStage::is_native_attachment("matroska"),
            "'matroska' aliases to ContainerFormat::Mkv and must resolve to native attachment support"
        );
    }

    /// Boundary test: an uppercase `.MKA` extension must still resolve to
    /// native-attachment support end to end. Under the string-list design
    /// this was `is_native_attachment_container`'s own case-insensitive
    /// comparison; under the typed design that behavior has moved into
    /// `ContainerFormat::from_str`'s `#[strum(ascii_case_insensitive)]`
    /// parsing. Kept as an explicit test so this coverage isn't silently
    /// lost in the migration (#533).
    #[test]
    fn uppercase_extension_still_resolves_native_attachment() {
        assert!(
            ThumbnailStage::is_native_attachment("MKA"),
            "an uppercase .MKA extension must still parse and resolve to native attachment support"
        );
    }

    /// Negative: MP4-family (and other non-Matroska) containers must NOT be
    /// treated as native-attachment — they need normalization for non-raster
    /// codecs. Regression guard for the bug: an unfixed check that also
    /// exempted "mp4" would silently re-introduce the webp mux failure.
    #[test]
    fn native_attachment_container_rejects_mp4_family() {
        assert!(!ThumbnailStage::is_native_attachment("mp4"));
        assert!(!ThumbnailStage::is_native_attachment("mov"));
        assert!(!ThumbnailStage::is_native_attachment("mp3"));
    }

    // #530 end-to-end coverage (gif/tiff attach natively, bmp/webp get
    // normalized) lives in `crates/rdlp-postprocess/tests/
    // thumbnail_mkv_cover_mimetype.rs` — those cases need the `ffmpeg` CLI to
    // build real decodable image fixtures, which `scripts/check-no-cli.sh`
    // forbids under `src/` (production-code CLI-usage gate); only files under
    // `tests/` are exempt.

    // #548: an explicit `--remux` container must never be silently
    // overridden by the thumbnail stage's auto-remux-to-mp4 fallback.
    //
    // The "kept container" positive cases and the "no explicit container
    // still auto-remuxes" regression guard need a REAL, decodable fixture to
    // be trustworthy: a fake/nonexistent path makes the pre-existing
    // auto-remux-failure warning ("Auto-remux for thumbnail embedding
    // failed: ...") happen to contain the fixture's own extension and the
    // word "thumbnail", passing for the wrong reason even against the
    // unpatched code. Those cases live in
    // `crates/rdlp-postprocess/tests/thumbnail_explicit_container_548.rs`
    // (real `ffmpeg` CLI fixtures; self-skips when the CLI is absent), same
    // pattern as `thumbnail_webp_mp4_embed.rs`.

    /// Negative companion: an explicit container that DOES support embedding
    /// (e.g. `--remux=mp4`) must still take the normal embed path — the
    /// keep-container guard must not intercept it. Verified by asserting the
    /// normal "thumbnail not found" warning fires (proving the embed path
    /// ran), not the keep-container skip message.
    #[tokio::test]
    async fn process_embeds_normally_when_explicit_container_supports_thumbnail() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcess {
            embed_thumbnail: true,
            remux_container: Some(ContainerFormat::Mp4),
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![PathBuf::from("/tmp/issue548-video.mp4")], config);
        msg.original_stem = "issue548-nonexistent-stem".to_string();

        let result = stage.process(msg).await.unwrap();

        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("Thumbnail file not found")),
            "expected the normal embed path (missing-thumbnail warning), got: {:?}",
            result.warnings
        );
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.contains("cannot carry an embedded thumbnail")),
            "the keep-container guard must not fire for a container that supports embedding"
        );
    }
}
