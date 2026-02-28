//! FFmpeg video conversion/remuxing post-processor.
//!
//! Converts video files to different formats or remuxes them to different
//! containers using `ffmpeg-the-third` library bindings (no CLI process spawning).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessCallback, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use rdlp_ffmpeg::PostProcessError;
use rdlp_ffmpeg::VideoConvertOptions;

use rdlp_core::ContainerFormat;

/// Supported video codecs for transcoding.
const VIDEO_CODECS: &[(&str, &str)] = &[
    ("h264", "libx264"),
    ("h265", "libx265"),
    ("hevc", "libx265"),
    ("vp9", "libvpx-vp9"),
    ("vp8", "libvpx"),
    ("av1", "libaom-av1"),
    ("vvc", "libvvenc"),
    ("h266", "libvvenc"),
    ("mpeg1", "mpeg1video"),
    ("mpeg2", "mpeg2video"),
    ("mpeg4", "mpeg4"),
    ("theora", "libtheora"),
    ("prores", "prores_ks"),
    ("dnxhd", "dnxhd"),
    ("wmv2", "wmv2"),
    ("ffv1", "ffv1"),
    ("xvid", "libxvid"),
];

ffmpeg_processor!(
    FFmpegVideoConvertor,
    "FFmpegVideoConvertor",
    40,
    "Post-processor that converts or remuxes video files.\n\n\
     # Priority\n\
     This processor has priority 40 (runs after merging, before metadata).\n\n\
     # When it runs\n\
     - When `recode_video` is specified in config (full transcode)\n\
     - When container change is needed"
);

impl FFmpegVideoConvertor {
    /// Check if the format is a supported container.
    fn is_supported_container(format: &str) -> bool {
        format.parse::<ContainerFormat>().is_ok()
    }

    /// Get the FFmpeg encoder for a codec.
    fn get_encoder(codec: &str) -> Option<&'static str> {
        VIDEO_CODECS
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(codec))
            .map(|(_, e)| *e)
    }

    /// Determine if we can remux (copy) or need to transcode.
    fn can_remux(input_ext: &str, output_ext: &str, video_codec: Option<&str>) -> bool {
        /// Check if `codec` case-insensitively matches any of the given names.
        fn codec_is(codec: &str, names: &[&str]) -> bool {
            names.iter().any(|n| n.eq_ignore_ascii_case(codec))
        }

        /// Check if `ext` case-insensitively matches any of the given extensions.
        fn ext_is(ext: &str, exts: &[&str]) -> bool {
            exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
        }

        // Check if the codec is compatible with the target container
        if ext_is(output_ext, &["mp4", "f4v"]) {
            // MP4/F4V supports H.264, H.265, MPEG-4, AV1
            video_codec
                .is_some_and(|c| codec_is(c, &["h264", "avc", "h265", "hevc", "mpeg4", "av1"]))
        } else if ext_is(output_ext, &["mkv", "mka", "nut", "mxf"]) {
            // MKV, MKA, NUT, MXF accept almost everything
            true
        } else if output_ext.eq_ignore_ascii_case("webm") {
            // WebM supports VP8, VP9, AV1
            video_codec.is_some_and(|c| codec_is(c, &["vp8", "vp9", "av1"]))
        } else if output_ext.eq_ignore_ascii_case("ivf") {
            // IVF supports VP8, VP9, AV1
            video_codec.is_some_and(|c| codec_is(c, &["vp8", "vp9", "av1"]))
        } else if output_ext.eq_ignore_ascii_case("3gp") {
            // 3GP supports H.264, H.263, MPEG-4
            video_codec.is_some_and(|c| codec_is(c, &["h264", "avc", "h263", "mpeg4"]))
        } else if output_ext.eq_ignore_ascii_case("asf") {
            // ASF/WMV supports WMV, H.264, MPEG-4
            video_codec.is_some_and(|c| codec_is(c, &["wmv1", "wmv2", "h264", "avc", "mpeg4"]))
        } else if ext_is(output_ext, &["mpg", "vob"]) {
            // MPEG/VOB supports MPEG-1, MPEG-2, MPEG-4
            video_codec.is_some_and(|c| {
                codec_is(c, &["mpeg1", "mpeg1video", "mpeg2", "mpeg2video", "mpeg4"])
            })
        } else if output_ext.eq_ignore_ascii_case("avi") {
            // AVI supports most codecs
            true
        } else {
            // Same container - can copy
            input_ext.eq_ignore_ascii_case(output_ext)
        }
    }

    /// Build `VideoConvertOptions` from the target format and remux decision.
    fn build_convert_options(target_format: &str, can_remux: bool) -> VideoConvertOptions {
        if can_remux {
            return VideoConvertOptions {
                remux_only: true,
                audio_copy: true,
                ..Default::default()
            };
        }

        // Determine target codec from container
        let target_codec = match target_format {
            "webm" => "vp9",
            "ivf" => "vp9",
            "ogg" => "theora",
            "mpg" | "vob" => "mpeg2",
            _ => "h264",
        };

        let encoder = Self::get_encoder(target_codec);
        let (preset, crf) = match target_codec {
            "h264" | "h265" | "hevc" | "vvc" | "h266" => (Some("medium".to_string()), Some(23)),
            "vp9" | "vp8" => (None, Some(30)),
            "av1" => (None, Some(28)),
            _ => (None, None),
        };

        VideoConvertOptions {
            remux_only: false,
            video_codec: encoder.map(String::from),
            preset,
            crf,
            audio_copy: true,
        }
    }
}

#[async_trait]
impl PostProcessor for FFmpegVideoConvertor {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        // Run if recode_video is specified
        config.recode_video.is_some()
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
        callback: Option<Arc<dyn PostProcessCallback>>,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let input_file = &files[0];

        let target_format = match config.recode_video {
            Some(c) => c.as_ext(),
            None => {
                debug!("No recode target configured; defaulting to MP4");
                "mp4"
            }
        };

        // Validate target format
        if !Self::is_supported_container(target_format) {
            return Err(PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "video conversion".to_string(),
            }
            .into());
        }

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Skip if already in target format
        if input_ext.eq_ignore_ascii_case(target_format) {
            debug!(format:? = target_format; "File already in target format, skipping conversion");
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!(
            file:? = input_file.display(),
            from:? = input_ext,
            to:? = target_format;
            "Converting format"
        );

        // Probe input to determine codecs
        let media_info = self.ffmpeg.probe(input_file).await?;

        // Determine if we can remux or need to transcode
        let can_remux =
            Self::can_remux(input_ext, target_format, media_info.video_codec.as_deref());

        if can_remux {
            debug!("Remuxing (stream copy)");
        } else {
            debug!("Transcoding video");
        }

        // Build output path
        let output_path = input_file.with_extension(target_format);

        // Build conversion options
        let opts = Self::build_convert_options(target_format, can_remux);

        // Convert via library bindings
        let progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>> =
            callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                Arc::new(move |frac| cb.on_progress(frac))
            });
        self.ffmpeg
            .convert_video(input_file, &output_path, &opts, progress_fn)
            .await?;

        info!(output:? = output_path.display(); "Converted");

        Ok(PostProcessResult {
            info: info.clone(),
            files: vec![output_path],
            temp_files: if config.keep_video { Vec::new() } else { files },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_supported_container() {
        assert!(FFmpegVideoConvertor::is_supported_container("mp4"));
        assert!(FFmpegVideoConvertor::is_supported_container("MKV"));
        assert!(FFmpegVideoConvertor::is_supported_container("webm"));
        assert!(!FFmpegVideoConvertor::is_supported_container("xyz"));
    }

    #[test]
    fn test_get_encoder() {
        assert_eq!(FFmpegVideoConvertor::get_encoder("h264"), Some("libx264"));
        assert_eq!(FFmpegVideoConvertor::get_encoder("vp9"), Some("libvpx-vp9"));
        assert_eq!(FFmpegVideoConvertor::get_encoder("unknown"), None);
    }

    #[test]
    fn test_can_remux() {
        // H.264 to MP4 - can remux
        assert!(FFmpegVideoConvertor::can_remux("mkv", "mp4", Some("h264")));

        // VP9 to MP4 - cannot remux (needs transcode)
        assert!(!FFmpegVideoConvertor::can_remux("webm", "mp4", Some("vp9")));

        // VP9 to WebM - can remux
        assert!(FFmpegVideoConvertor::can_remux("mkv", "webm", Some("vp9")));

        // Anything to MKV - can remux
        assert!(FFmpegVideoConvertor::can_remux("mp4", "mkv", Some("h264")));
        assert!(FFmpegVideoConvertor::can_remux("webm", "mkv", Some("vp9")));
    }

    #[test]
    fn test_build_convert_options_remux() {
        let opts = FFmpegVideoConvertor::build_convert_options("mp4", true);
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
        assert!(opts.video_codec.is_none());
    }

    #[test]
    fn test_build_convert_options_transcode_mp4() {
        let opts = FFmpegVideoConvertor::build_convert_options("mp4", false);
        assert!(!opts.remux_only);
        assert_eq!(opts.video_codec, Some("libx264".to_string()));
        assert_eq!(opts.preset, Some("medium".to_string()));
        assert_eq!(opts.crf, Some(23));
        assert!(opts.audio_copy);
    }

    #[test]
    fn test_build_convert_options_transcode_webm() {
        let opts = FFmpegVideoConvertor::build_convert_options("webm", false);
        assert!(!opts.remux_only);
        assert_eq!(opts.video_codec, Some("libvpx-vp9".to_string()));
        assert_eq!(opts.preset, None); // VP9 has no preset
        assert_eq!(opts.crf, Some(30));
        assert!(opts.audio_copy);
    }
}
