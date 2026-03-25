use std::path::PathBuf;
use std::sync::Arc;

use rdlp_types::{AudioFormat, ContainerFormat, RecodeAudioMode};

/// Callback for reporting post-processing progress.
///
/// Implementations receive progress updates (0.0–1.0) during FFmpeg
/// post-processing stages. Used to bridge synchronous FFmpeg loops
/// to the async event system.
///
/// # Arguments
/// * `progress` - Progress fraction in \[0.0, 1.0\]
///
/// # Example
///
/// ```rust
/// use rdlp_core::PostProcessCallback;
/// use std::sync::Arc;
///
/// struct PrintProgress;
/// impl PostProcessCallback for PrintProgress {
///     fn on_progress(&self, progress: f64) {
///         println!("Progress: {:.0}%", progress * 100.0);
///     }
/// }
/// ```
pub trait PostProcessCallback: Send + Sync {
    /// Report progress as a fraction in \[0.0, 1.0\].
    fn on_progress(&self, progress: f64);

    /// Forward a log message from the post-processing stage.
    ///
    /// Used to surface FFmpeg C-level log output (e.g. matroska muxer trace)
    /// to the frontend when verbose/trace mode is enabled. Default is a no-op.
    fn on_log(&self, _message: &str) {}
}

/// Factory for creating per-stage post-processing callbacks.
///
/// Receives the processor/stage name and returns a fresh callback that
/// will receive progress updates (0.0–1.0) for that specific stage.
pub type PostProcessCallbackFactory =
    Arc<dyn Fn(&str) -> Arc<dyn PostProcessCallback> + Send + Sync>;

/// Post-processing configuration
#[derive(Debug, Clone, Default)]
pub struct PostProcessConfig {
    /// Extract audio only
    pub extract_audio: bool,

    /// Audio format to convert to
    pub audio_format: Option<AudioFormat>,

    /// Audio quality (VBR level or bitrate)
    pub audio_quality: Option<String>,

    /// Video format to recode to
    pub recode_video: Option<ContainerFormat>,

    /// Remux to container format (stream copy, no re-encoding)
    pub remux_container: Option<ContainerFormat>,

    /// Merge output format (when combining video+audio)
    pub merge_output_format: Option<ContainerFormat>,

    /// Embed thumbnail in video file
    pub embed_thumbnail: bool,

    /// Write thumbnail image to disk (keep after embedding)
    pub write_thumbnail: bool,

    /// Embed metadata (title, artist, etc.)
    pub embed_metadata: bool,

    /// Embed subtitles in video file
    pub embed_subtitles: bool,

    /// Keep subtitle files after embedding
    pub write_subtitles: bool,

    /// Keep original files after processing
    pub keep_video: bool,

    /// FFmpeg location (if not in PATH)
    pub ffmpeg_location: Option<PathBuf>,

    /// Additional FFmpeg arguments
    pub ffmpeg_args: Vec<String>,

    /// Normalize audio levels (peak mode unless loudnorm is set)
    pub normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (implies normalize_audio)
    pub loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default -1.0)
    pub audio_gain_target: Option<f64>,

    /// Loudnorm preset name (broadcast, streaming, loud)
    pub loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm (default -14.0)
    pub loudnorm_target_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm (default -1.5)
    pub loudnorm_target_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm (default 11.0)
    pub loudnorm_target_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    pub loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm in pass 2
    pub loudnorm_precompress: bool,

    /// Enable limiter-boost fallback for over-compressed content
    pub normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default 12.0 when None)
    pub normalize_boost_db: Option<f64>,

    /// Enable verbose FFmpeg logging (forward muxer trace to callback).
    pub verbose: bool,

    /// Explicit video encoder override (e.g., "libsvtav1", "libx264").
    /// When `None`, the best available encoder for the target codec is
    /// auto-selected via the encoder registry. When `Some`, the named
    /// encoder is verified for availability before use; an unavailable
    /// encoder causes the conversion step to return an error.
    pub video_encoder: Option<String>,

    /// How to handle audio during video recode.
    /// Default is Copy (stream copy audio unchanged).
    pub recode_audio: RecodeAudioMode,

    /// Output container override for video recode.
    /// When `None`, the container is derived from the video codec.
    pub recode_container: Option<ContainerFormat>,
}
