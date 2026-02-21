//! Download request types for the rdlp public API.
//!
//! These structs define the full set of options a frontend can pass when
//! initiating a download. All types use typed enums from `rdlp-types`
//! (re-exported through `rdlp-core`) instead of raw strings.

use rdlp_core::{AudioFormat, BrowserType, ContainerFormat, SubtitleFormat};
use std::path::PathBuf;

/// Top-level request for initiating a download.
///
/// # Examples
///
/// ```
/// use rdlp_api::request::DownloadRequest;
///
/// let req = DownloadRequest::new("https://example.com/video");
/// assert_eq!(req.url, "https://example.com/video");
/// assert!(!req.plan_only);
/// ```
#[derive(Debug, Clone, Default)]
pub struct DownloadRequest {
    /// URL to download.
    pub url: String,
    /// Output file/directory options.
    pub output: OutputOptions,
    /// Format selection options.
    pub format: FormatOptions,
    /// Subtitle download/embed options.
    pub subtitles: SubtitleOptions,
    /// Post-processing options (remux, extract audio, metadata, etc.).
    pub postprocess: PostProcessOptions,
    /// Network and retry options.
    pub network: NetworkOptions,
    /// If `true`, only extract metadata without downloading.
    pub plan_only: bool,
    /// Enable verbose logging. `None` preserves base config.
    pub verbose: Option<bool>,
}

impl DownloadRequest {
    /// Create a new request for the given URL with default options.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL to download from.
    ///
    /// # Returns
    ///
    /// A `DownloadRequest` with all options set to their defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use rdlp_api::request::DownloadRequest;
    ///
    /// let req = DownloadRequest::new("https://example.com/video");
    /// assert_eq!(req.url, "https://example.com/video");
    /// ```
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}

/// Output file and directory options.
///
/// # Examples
///
/// ```
/// use rdlp_api::request::OutputOptions;
/// use std::path::PathBuf;
///
/// // File output with template
/// let opts = OutputOptions {
///     output_dir: Some(PathBuf::from("/tmp/downloads")),
///     template: Some("%(title)s.%(ext)s".into()),
///     ..OutputOptions::default()
/// };
///
/// // Stdout streaming (-o -)
/// let stdout_opts = OutputOptions {
///     stdout: Some(true),
///     ..OutputOptions::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    /// Directory to write output files to.
    pub output_dir: Option<PathBuf>,
    /// Output filename template (yt-dlp `%(field)s` syntax).
    pub template: Option<String>,
    /// Prefix prepended to all output paths.
    pub paths_prefix: Option<PathBuf>,
    /// Stream output to stdout (`-o -`). `None` preserves base config.
    pub stdout: Option<bool>,
}

/// Format selection options.
///
/// # Examples
///
/// ```
/// use rdlp_api::request::FormatOptions;
///
/// let opts = FormatOptions {
///     selector: Some("bestvideo+bestaudio".into()),
///     ..FormatOptions::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct FormatOptions {
    /// Format selector string (e.g. `"bestvideo+bestaudio"`).
    pub selector: Option<String>,
    /// Prefer free/open-source codecs (VP9, Opus, etc.).
    pub prefer_free_formats: bool,
    /// Show interactive format selection menu.
    pub interactive: bool,
    /// Require strict video-only + audio-only streams for merge.
    /// `None` preserves base config.
    pub audio_multistreams: Option<bool>,
}

/// Subtitle download and embedding options.
///
/// # Examples
///
/// ```
/// use rdlp_api::request::SubtitleOptions;
/// use rdlp_core::SubtitleFormat;
///
/// let opts = SubtitleOptions {
///     write_subs: Some(true),
///     sub_langs: vec!["en".into(), "ja".into()],
///     sub_format: Some(SubtitleFormat::Srt),
///     ..SubtitleOptions::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct SubtitleOptions {
    /// Download subtitles. `None` preserves base config.
    pub write_subs: Option<bool>,
    /// Download auto-generated subtitles. `None` preserves base config.
    pub write_auto_subs: Option<bool>,
    /// Subtitle language codes to download (e.g. `["en", "ja"]`).
    pub sub_langs: Vec<String>,
    /// Preferred subtitle format.
    pub sub_format: Option<SubtitleFormat>,
    /// Embed subtitles into the output container. `None` preserves base config.
    pub embed_subs: Option<bool>,
    /// Fail if requested subtitles are not available. `None` preserves base config.
    pub strict_subs: Option<bool>,
}

/// Post-processing options (remux, audio extraction, metadata, thumbnails).
///
/// # Examples
///
/// ```no_run
/// use rdlp_api::request::PostProcessOptions;
/// use rdlp_core::ContainerFormat;
///
/// let opts = PostProcessOptions {
///     remux: Some(ContainerFormat::Mp4),
///     embed_metadata: Some(true),
///     ..PostProcessOptions::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct PostProcessOptions {
    /// Remux to a different container format.
    pub remux: Option<ContainerFormat>,
    /// Extract audio to the specified format.
    pub extract_audio: Option<AudioFormat>,
    /// Embed metadata (title, uploader, etc.) into the output file. `None` preserves base config.
    pub embed_metadata: Option<bool>,
    /// Embed thumbnail into the output file. `None` preserves base config.
    pub embed_thumbnail: Option<bool>,
    /// Disable thumbnail downloading entirely. `None` preserves base config.
    pub no_thumbnail: Option<bool>,
    /// Keep the downloaded thumbnail as a separate file. `None` preserves base config.
    pub write_thumbnail: Option<bool>,
    /// Apply peak audio normalization. `None` preserves base config.
    pub normalize_audio: Option<bool>,
    /// Apply EBU R128 loudness normalization (two-pass). `None` preserves base config.
    pub loudnorm: Option<bool>,
    /// Loudness normalization preset name (e.g. `"streaming"`).
    pub loudnorm_preset: Option<String>,
    /// Recode (transcode) video into a different container format.
    pub recode_video: Option<ContainerFormat>,
}

/// Network, retry, and cookie options.
///
/// # Examples
///
/// ```
/// use rdlp_api::request::NetworkOptions;
/// use rdlp_core::BrowserType;
///
/// let opts = NetworkOptions {
///     retries: Some(5),
///     cookies_from_browser: Some(BrowserType::Chrome),
///     ..NetworkOptions::default()
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct NetworkOptions {
    /// Maximum number of retries for failed requests. `None` preserves base config.
    pub retries: Option<u32>,
    /// Per-read idle timeout in seconds. `None` preserves base config.
    pub timeout_secs: Option<u64>,
    /// Number of concurrent download fragments/chunks. `None` preserves base config.
    pub concurrent_fragments: Option<u32>,
    /// Download rate limit in bytes per second.
    pub rate_limit: Option<u64>,
    /// Browser to extract cookies from.
    pub cookies_from_browser: Option<BrowserType>,
    /// Path to a Netscape-format cookies file.
    pub cookies_file: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_download_request() {
        let req = DownloadRequest::default();

        // Top-level defaults
        assert!(req.url.is_empty());
        assert!(!req.plan_only);
        assert!(req.verbose.is_none());

        // OutputOptions defaults
        assert!(req.output.output_dir.is_none());
        assert!(req.output.template.is_none());
        assert!(req.output.paths_prefix.is_none());
        assert!(req.output.stdout.is_none());

        // FormatOptions defaults
        assert!(req.format.selector.is_none());
        assert!(!req.format.prefer_free_formats);
        assert!(!req.format.interactive);
        assert!(req.format.audio_multistreams.is_none());

        // SubtitleOptions defaults
        assert!(req.subtitles.write_subs.is_none());
        assert!(req.subtitles.write_auto_subs.is_none());
        assert!(req.subtitles.sub_langs.is_empty());
        assert!(req.subtitles.sub_format.is_none());
        assert!(req.subtitles.embed_subs.is_none());
        assert!(req.subtitles.strict_subs.is_none());

        // PostProcessOptions defaults
        assert!(req.postprocess.remux.is_none());
        assert!(req.postprocess.extract_audio.is_none());
        assert!(req.postprocess.embed_metadata.is_none());
        assert!(req.postprocess.embed_thumbnail.is_none());
        assert!(req.postprocess.no_thumbnail.is_none());
        assert!(req.postprocess.write_thumbnail.is_none());
        assert!(req.postprocess.normalize_audio.is_none());
        assert!(req.postprocess.loudnorm.is_none());
        assert!(req.postprocess.loudnorm_preset.is_none());
        assert!(req.postprocess.recode_video.is_none());

        // NetworkOptions defaults
        assert!(req.network.retries.is_none());
        assert!(req.network.timeout_secs.is_none());
        assert!(req.network.concurrent_fragments.is_none());
        assert!(req.network.rate_limit.is_none());
        assert!(req.network.cookies_from_browser.is_none());
        assert!(req.network.cookies_file.is_none());
    }

    #[test]
    fn test_request_with_overrides() {
        let req = DownloadRequest {
            url: "https://example.com/video".into(),
            output: OutputOptions {
                output_dir: Some(PathBuf::from("/tmp/downloads")),
                template: Some("%(title)s.%(ext)s".into()),
                ..OutputOptions::default()
            },
            format: FormatOptions {
                selector: Some("bestvideo+bestaudio".into()),
                interactive: true,
                ..FormatOptions::default()
            },
            subtitles: SubtitleOptions {
                write_subs: Some(true),
                sub_langs: vec!["en".into(), "ja".into()],
                sub_format: Some(SubtitleFormat::Srt),
                embed_subs: Some(true),
                ..SubtitleOptions::default()
            },
            postprocess: PostProcessOptions {
                remux: Some(ContainerFormat::Mp4),
                embed_metadata: Some(true),
                loudnorm: Some(true),
                loudnorm_preset: Some("streaming".into()),
                ..PostProcessOptions::default()
            },
            network: NetworkOptions {
                retries: Some(5),
                concurrent_fragments: Some(8),
                cookies_from_browser: Some(BrowserType::Chrome),
                ..NetworkOptions::default()
            },
            plan_only: true,
            verbose: Some(true),
        };

        assert_eq!(req.url, "https://example.com/video");
        assert!(req.plan_only);
        assert_eq!(req.verbose, Some(true));

        // Output overrides
        assert_eq!(
            req.output.output_dir.as_deref(),
            Some(std::path::Path::new("/tmp/downloads"))
        );
        assert_eq!(req.output.template.as_deref(), Some("%(title)s.%(ext)s"));
        assert!(req.output.paths_prefix.is_none());

        // Format overrides
        assert_eq!(req.format.selector.as_deref(), Some("bestvideo+bestaudio"));
        assert!(!req.format.prefer_free_formats);
        assert!(req.format.interactive);

        // Subtitle overrides
        assert_eq!(req.subtitles.write_subs, Some(true));
        assert!(req.subtitles.write_auto_subs.is_none());
        assert_eq!(req.subtitles.sub_langs, vec!["en", "ja"]);
        assert_eq!(req.subtitles.sub_format, Some(SubtitleFormat::Srt));
        assert_eq!(req.subtitles.embed_subs, Some(true));
        assert!(req.subtitles.strict_subs.is_none());

        // PostProcess overrides
        assert_eq!(req.postprocess.remux, Some(ContainerFormat::Mp4));
        assert!(req.postprocess.extract_audio.is_none());
        assert_eq!(req.postprocess.embed_metadata, Some(true));
        assert!(req.postprocess.embed_thumbnail.is_none());
        assert!(req.postprocess.no_thumbnail.is_none());
        assert!(req.postprocess.write_thumbnail.is_none());
        assert!(req.postprocess.normalize_audio.is_none());
        assert_eq!(req.postprocess.loudnorm, Some(true));
        assert_eq!(
            req.postprocess.loudnorm_preset.as_deref(),
            Some("streaming")
        );

        // Network overrides
        assert_eq!(req.network.retries, Some(5));
        assert!(req.network.timeout_secs.is_none());
        assert_eq!(req.network.concurrent_fragments, Some(8));
        assert!(req.network.rate_limit.is_none());
        assert_eq!(req.network.cookies_from_browser, Some(BrowserType::Chrome));
        assert!(req.network.cookies_file.is_none());
    }

    #[test]
    fn test_new_constructor() {
        let req = DownloadRequest::new("https://example.com/video");
        assert_eq!(req.url, "https://example.com/video");
        assert!(!req.plan_only);
        assert!(req.postprocess.embed_thumbnail.is_none());
        assert!(req.network.retries.is_none());
    }

    #[test]
    fn test_new_accepts_string() {
        let url = String::from("https://example.com/video");
        let req = DownloadRequest::new(url);
        assert_eq!(req.url, "https://example.com/video");
    }
}
