//! `RemuxStage` — remuxes to a target container format.
//!
//! This stage runs at index 3. It handles:
//! - Explicit remux via `config.remux_container`
//! - HLS auto-remux via `msg.is_hls` (replaces the old `ffmpeg_remux()` hack)
//!
//! Uses stream copy (no re-encoding). Skipped when audio extraction is active.

use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

use rdlp_types::ContainerFormat;

use super::policy_refusal::keep_download_on_policy_refusal;
use crate::pipeline::{PipelineMessage, PipelineStage};

/// Remuxes the current file to a different container.
///
/// Triggers when `config.remux_container.is_some() || msg.is_hls`.
/// Skipped when `config.extract_audio` is true (audio extract produces a
/// standalone audio file that must not be remuxed into a video container).
pub struct RemuxStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl RemuxStage {
    /// Create a new `RemuxStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine the target container for remuxing.
    ///
    /// Returns `None` if the file is already in the target container —
    /// meaning it already carries that container's canonical extension
    /// (`ContainerFormat::is_canonical_ext`); an alias-spelled input such as
    /// `.matroska` is remuxed, and the remux is what canonicalises its name
    /// (#619).
    ///
    /// Returns the `ContainerFormat` itself rather than its extension: the
    /// decision originates from `config.remux_container`, which is already
    /// typed, and callers need to ask container-level questions of it (does it
    /// support faststart?). Handing back a `&str` forced every caller to
    /// re-derive those answers from string literals, which is how the faststart
    /// list silently drifted out of sync with `supports_faststart()` (#539).
    fn target_container(msg: &PipelineMessage, input_ext: &str) -> Option<ContainerFormat> {
        if let Some(container) = msg.config.remux_container {
            if container.is_canonical_ext(input_ext) {
                return None; // already in target container
            }
            return Some(container);
        }
        // HLS auto-remux: .ts → .mp4
        if msg.is_hls && !ContainerFormat::Mp4.is_canonical_ext(input_ext) {
            return Some(ContainerFormat::Mp4);
        }
        None
    }
}

#[async_trait]
impl PipelineStage for RemuxStage {
    fn name(&self) -> &'static str {
        "RemuxStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        // Skip when audio extraction is active — extract_audio produces a
        // standalone audio file that should not be remuxed into a video container.
        if msg.config.extract_audio {
            return false;
        }
        msg.config.remux_container.is_some() || msg.is_hls
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();
        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let Some(target) = Self::target_container(&msg, input_ext) else {
            debug!("RemuxStage: file already in target container ({input_ext}), skipping");
            return Ok(msg);
        };

        info!(
            "RemuxStage: remuxing {} → {}",
            input_file.display(),
            target.as_ext()
        );

        let output_path = msg.tracker.temp_path(&input_file, target.as_ext());

        let opts = RemuxOptions::for_container(target, msg.encoding_tool.clone());

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
        });

        if let Err(e) = self
            .ffmpeg
            .remux(&input_file, &output_path, &opts, callback)
            .await
        {
            keep_download_on_policy_refusal(&e, &mut msg, "RemuxStage");
            return Err(anyhow::Error::new(e).context("remux stage failed"));
        }

        debug!("RemuxStage: remuxed to {}", output_path.display());

        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_types::PostProcess;
    use rdlp_types::{ContainerFormat, InfoDict};

    use crate::pipeline::{FileTracker, TempRegistry};

    fn make_msg_with_config(
        files: Vec<PathBuf>,
        config: PostProcess,
        is_hls: bool,
    ) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls,
            verbose: false,
            callback_factory: None,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_with_remux_container() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let config = PostProcess {
            remux_container: Some(ContainerFormat::Mp4),
            ..PostProcess::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/video.ts")], config, false);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_run_when_is_hls() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let msg = make_msg_with_config(
            vec![PathBuf::from("/tmp/video.ts")],
            PostProcess::default(),
            true,
        );
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_without_remux_or_hls() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let msg = make_msg_with_config(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcess::default(),
            false,
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_when_extract_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);

        let config = PostProcess {
            remux_container: Some(ContainerFormat::Mkv),
            extract_audio: true,
            ..PostProcess::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/video.ts")], config, false);
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn target_container_already_in_target_returns_none() {
        let reg = Arc::new(TempRegistry::new());
        let msg = PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "T".to_string(),
                "T".to_string(),
                "https://x.com".to_string(),
            ),
            tracker: FileTracker::new(vec![PathBuf::from("/tmp/v.mp4")], reg),
            config: Arc::new(PostProcess {
                remux_container: Some(ContainerFormat::Mp4),
                ..PostProcess::default()
            }),
            original_stem: "v".into(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        assert!(RemuxStage::target_container(&msg, "mp4").is_none());
    }

    /// #539: the faststart decision must come from the container type.
    ///
    /// `m4v`/`f4v` route to the same mov/mp4 muxer as `mp4`/`mov` and
    /// `+faststart` demonstrably relocates their `moov` atom, but the old
    /// `matches!(target_ext, "mp4" | "mov")` excluded them, so `--remux m4v`
    /// shipped a file with `moov` at the end.
    #[test]
    fn faststart_follows_the_container_type_for_every_remux_target() {
        // The third column is the expected extension as a LITERAL: the stage
        // names its temp output from `target.as_ext()` (see `process`), so a
        // wrong `as_ext()` (e.g. #538's `Wmv => "asf"`) would propagate the
        // wrong extension toward the output filename. Pinned here, one layer
        // closer to the user than the `rdlp-types` unit tests reach.
        for (container, want, want_ext) in [
            (ContainerFormat::Mp4, true, "mp4"),
            (ContainerFormat::Mov, true, "mov"),
            (ContainerFormat::M4v, true, "m4v"),
            (ContainerFormat::F4v, true, "f4v"),
            (ContainerFormat::Mkv, false, "mkv"),
            (ContainerFormat::WebM, false, "webm"),
            (ContainerFormat::Avi, false, "avi"),
            // #538: the ASF family, pinned one layer closer to the user than
            // the `rdlp-types` unit tests reach.
            (ContainerFormat::Wmv, false, "wmv"),
            (ContainerFormat::Wma, false, "wma"),
            (ContainerFormat::Asf, false, "asf"),
        ] {
            let config = PostProcess {
                remux_container: Some(container),
                ..PostProcess::default()
            };
            let msg = make_msg_with_config(vec![PathBuf::from("/tmp/v.ts")], config, false);
            let target = RemuxStage::target_container(&msg, "ts")
                .expect("a different container must produce a remux target");
            assert_eq!(target, container);
            assert_eq!(
                target.as_ext(),
                want_ext,
                "the output filename for {container:?} derives from as_ext()"
            );
            // Assert on the options the stage actually builds — asserting
            // `target.supports_faststart()` here would only re-test the
            // predicate with a value this test supplied, and would stay green
            // if the stage hardcoded `faststart: false`.
            let opts = RemuxOptions::for_container(target, None);
            assert_eq!(
                opts.faststart, want,
                "faststart for {container:?} must follow supports_faststart()"
            );
        }
    }

    /// #538 flipped the already-in-target short-circuit for the ASF family,
    /// in both directions, and neither direction was pinned.
    ///
    /// `target_container` skips the remux when the input extension already
    /// equals `container.as_ext()`. While `wmv`/`wma`/`asf` shared one variant
    /// whose `as_ext()` was `"asf"`, a `.wmv` input compared against `"asf"`:
    /// `--remux=wmv` re-muxed a file that was already in the requested
    /// container, and `--remux=asf` on a `.wmv` input wrongly skipped. Splitting
    /// the variants corrects both, so both are asserted here.
    #[test]
    fn asf_family_short_circuits_per_spelling_not_per_muxer() {
        // Same spelling in and out → nothing to do.
        for (input_ext, container) in [
            ("wmv", ContainerFormat::Wmv),
            ("wma", ContainerFormat::Wma),
            ("asf", ContainerFormat::Asf),
        ] {
            let config = PostProcess {
                remux_container: Some(container),
                ..PostProcess::default()
            };
            let msg = make_msg_with_config(
                vec![PathBuf::from(format!("/tmp/v.{input_ext}"))],
                config,
                false,
            );
            assert!(
                RemuxStage::target_container(&msg, input_ext).is_none(),
                "{input_ext} input with --remux={input_ext} must skip the remux"
            );
        }

        // Different spelling within the same muxer family → still a real remux,
        // because the extension the user asked for is not the one on disk.
        for (input_ext, container) in [
            ("wmv", ContainerFormat::Asf),
            ("asf", ContainerFormat::Wmv),
            ("wmv", ContainerFormat::Wma),
        ] {
            let config = PostProcess {
                remux_container: Some(container),
                ..PostProcess::default()
            };
            let msg = make_msg_with_config(
                vec![PathBuf::from(format!("/tmp/v.{input_ext}"))],
                config,
                false,
            );
            assert_eq!(
                RemuxStage::target_container(&msg, input_ext),
                Some(container),
                "{input_ext} input with --remux={} must still remux",
                container.as_ext()
            );
        }
    }

    /// #619: `--remux=<c>` promises the container AND its canonical
    /// extension. An input spelled with a parse-only alias (`.matroska`,
    /// `.quicktime`, `.mpegts`) resolves to the same `ContainerFormat`, but
    /// no muxer declares those as extensions, so it is NOT already in the
    /// target: the remux runs, and its output is named `as_ext()` — that
    /// remux is the canonicalisation. Both halves are pinned here: the
    /// decision to do work, and the extension the work is named with.
    #[test]
    fn alias_spelled_input_is_remuxed_to_the_canonical_extension() {
        for (input_ext, container, want_ext) in [
            ("matroska", ContainerFormat::Mkv, "mkv"),
            ("quicktime", ContainerFormat::Mov, "mov"),
            ("mpegts", ContainerFormat::Ts, "ts"),
        ] {
            // The alias really does parse to the target — the whole point is
            // that parsing equality is NOT the "already in target" test.
            assert_eq!(
                ContainerFormat::from_path(std::path::Path::new(&format!("/tmp/v.{input_ext}"))),
                Some(container)
            );
            let config = PostProcess {
                remux_container: Some(container),
                ..PostProcess::default()
            };
            let msg = make_msg_with_config(
                vec![PathBuf::from(format!("/tmp/v.{input_ext}"))],
                config,
                false,
            );
            let target = RemuxStage::target_container(&msg, input_ext)
                .unwrap_or_else(|| panic!(".{input_ext} + --remux={want_ext} must remux"));
            assert_eq!(target.as_ext(), want_ext, "output is named canonically");
            // And the canonical spelling is left alone.
            assert_eq!(RemuxStage::target_container(&msg, want_ext), None);
        }

        // HLS auto-remux: `.mpegts` is not `.mp4`, so it is remuxed too.
        let msg = make_msg_with_config(
            vec![PathBuf::from("/tmp/v.mpegts")],
            PostProcess::default(),
            true,
        );
        assert_eq!(
            RemuxStage::target_container(&msg, "mpegts"),
            Some(ContainerFormat::Mp4)
        );
    }

    /// An explicit target is honoured verbatim, aliases included.
    #[test]
    fn target_container_returns_the_configured_container() {
        let config = PostProcess {
            remux_container: Some(ContainerFormat::M4v),
            ..PostProcess::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/v.mp4")], config, false);
        assert_eq!(
            RemuxStage::target_container(&msg, "mp4"),
            Some(ContainerFormat::M4v)
        );
    }

    /// Already-in-target detection stays case-insensitive.
    #[test]
    fn target_container_none_when_already_in_target_any_case() {
        let config = PostProcess {
            remux_container: Some(ContainerFormat::M4v),
            ..PostProcess::default()
        };
        let msg = make_msg_with_config(vec![PathBuf::from("/tmp/v.M4V")], config, false);
        assert!(RemuxStage::target_container(&msg, "M4V").is_none());
    }

    #[test]
    fn target_container_hls_ts_returns_mp4() {
        let reg = Arc::new(TempRegistry::new());
        let msg = PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "T".to_string(),
                "T".to_string(),
                "https://x.com".to_string(),
            ),
            tracker: FileTracker::new(vec![PathBuf::from("/tmp/v.ts")], reg),
            config: Arc::new(PostProcess::default()),
            original_stem: "v".into(),
            is_hls: true,
            verbose: false,
            callback_factory: None,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        assert_eq!(
            RemuxStage::target_container(&msg, "ts"),
            Some(ContainerFormat::Mp4)
        );
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RemuxStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }
}
