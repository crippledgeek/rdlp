//! The one way a stage runs `FFmpegRunner::extract_audio`.
//!
//! Two stages convert a single audio stream into a new container:
//! `AudioExtractStage` (`--extract-audio`) and `RecodeStage`'s audio-only
//! route (`--recode-video` on a source with no video, #637). Both need the
//! same five steps around the call — build the progress callback, bridge
//! `FFmpeg`'s logs, run the extract, stamp `encoding_tool`, hand the new file to
//! the tracker — and both got them independently at first.
//!
//! Keeping one implementation is not tidiness here. The `encoding_tool` stamp
//! in particular is a contract downstream stages read
//! (`metadata`/`thumbnail`/`fixup`/`remux` all propagate it), and the two
//! copies had already drifted: the recode side fell back to the *container
//! extension* when no encoder name was resolved, putting `"mkv"` in a slot
//! that names an encoder. This module owns that decision once, via
//! `audio_tag_component`, which is the same helper the video recode path uses.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use log::info;

use rdlp_ffmpeg::{AudioExtractOptions, FFmpegRunner};
use rdlp_types::CodecName;

use crate::pipeline::PipelineMessage;

/// One audio-extract run: what to convert, and how to describe it.
pub(super) struct AudioExtractJob<'a> {
    /// Stage name, for the callback factory and the completion log.
    ///
    /// Borrowed rather than `&'static str` because `PipelineStage::name`
    /// returns a `&str` tied to `&self`.
    pub stage_name: &'a str,
    /// Source file. Must contain an audio stream.
    pub input: PathBuf,
    /// Destination, already registered with the tracker via `temp_path`.
    pub output: PathBuf,
    /// Copy-or-encode decision and encoder selection.
    pub opts: AudioExtractOptions,
    /// Extra line for the UI log, describing what this run is doing.
    pub summary: Option<String>,
    /// Codec name to record when re-encoding without a named encoder — i.e.
    /// when `extract_audio` defers to the output muxer's own default (the
    /// `wav`/PCM row). A **codec** name, never a container: this field is what
    /// stops the `encoding_tool` slot being filled with something that does
    /// not name an encoder.
    pub fallback_codec: Option<CodecName>,
    /// Context prefix for a failure, e.g. `"audio extract stage failed"`.
    pub error_context: &'static str,
}

/// Run `job` against `msg`, replacing the tracker's current file on success.
///
/// On failure the tracker is left untouched, so `FileTracker`'s `Drop` cleans
/// up the temp path that `temp_path` issued.
///
/// # Errors
///
/// Propagates any failure from `extract_audio`, prefixed with
/// `job.error_context`.
pub(super) async fn run_audio_extract(
    ffmpeg: &FFmpegRunner,
    msg: &mut PipelineMessage,
    job: AudioExtractJob<'_>,
) -> anyhow::Result<()> {
    let stage_callback = msg.callback_factory.as_ref().map(|f| f(job.stage_name));
    let _log_bridge = stage_callback
        .as_ref()
        .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
    if let (Some(cb), Some(summary)) = (stage_callback.as_ref(), job.summary.as_deref()) {
        cb.on_log(summary);
    }
    let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
        Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
    });

    ffmpeg
        .extract_audio(
            &job.input,
            &job.output,
            &job.opts,
            callback,
            Some(msg.cancel.clone()),
        )
        .await
        .context(job.error_context)?;

    // The single source of truth for this field, shared with the video recode
    // path: "copy", the encoder name, or "none" — never a container name.
    let encoder_for_tag = job
        .opts
        .encoder_name
        .as_deref()
        .or(job.fallback_codec.as_deref());
    msg.encoding_tool =
        Some(rdlp_ffmpeg::ffmpeg::audio_tag_component(job.opts.copy, encoder_for_tag).to_string());

    info!(
        "{}: audio written to {}",
        job.stage_name,
        job.output.display()
    );
    msg.tracker.replace(vec![job.output]);

    Ok(())
}
