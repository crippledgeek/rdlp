//! Download lifecycle IPC commands.
//!
//! Provides Tauri commands to start, cancel, query, and remove
//! downloads from the managed download queue. Each download runs as
//! a background Tokio task, forwarding rdlp-api events to the
//! frontend via [`crate::events::emit_event`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::events;
use crate::state::{AppState, DownloadJob, JobStatus, SavedDownloadOptions};
use rdlp_api::Event;
use rdlp_api::request::{
    DownloadRequest, FormatOptions, NetworkOptions, OutputOptions, PostProcessOptions,
    SubtitleOptions,
};
use rdlp_types::{AudioFormat, BrowserType, ContainerFormat, RecodeAudioMode, VideoEncoderName};

/// Frontend-supplied download options.
///
/// Deserialized from JSON sent by the React frontend. Typed enums
/// (`ContainerFormat`, `AudioFormat`) deserialize from lowercase
/// JSON strings via serde. `Serialize` is derived so a snapshot can be
/// stored on the job for retry.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOptions {
    /// Format selector string (e.g. `"bestvideo+bestaudio"`).
    pub format: Option<String>,
    /// Directory to save the downloaded file.
    pub output_dir: Option<String>,
    /// Whether to download subtitles.
    pub subtitles: bool,
    /// Subtitle language codes (e.g. `["en", "sv"]`).
    pub subtitle_langs: Vec<String>,
    /// Container format to remux into (e.g. `Mp4`, `Mkv`).
    pub remux: Option<ContainerFormat>,
    /// Audio format to extract (e.g. `Mp3`, `Opus`).
    pub extract_audio: Option<AudioFormat>,
    /// Whether to embed a thumbnail in the output file.
    pub embed_thumbnail: bool,
    /// Allow downloading multiple audio streams for merge.
    pub audio_multistreams: bool,
    /// Container format to recode video into (transcodes, not just remux).
    pub recode_video: Option<ContainerFormat>,
    /// Enable audio normalization for this download. `None` = use settings default.
    pub normalize_audio: Option<bool>,
    /// Use EBU R128 loudnorm mode. `None` = use settings default.
    pub loudnorm: Option<bool>,
    /// Loudnorm preset override. `None` = use settings default.
    pub loudnorm_preset: Option<String>,
    /// Custom LUFS target. `None` = use settings default.
    pub loudnorm_target_i: Option<f64>,
    /// Custom dBTP target. `None` = use settings default.
    pub loudnorm_target_tp: Option<f64>,
    /// Custom LU target. `None` = use settings default.
    pub loudnorm_target_lra: Option<f64>,
    /// Force dynamic mode. `None` = use settings default.
    pub loudnorm_dynamic: Option<bool>,
    /// Precompress before loudnorm. `None` = use settings default.
    pub loudnorm_precompress: Option<bool>,
    /// Enable boost fallback. `None` = use settings default.
    pub normalize_boost: Option<bool>,
    /// Boost gain in dB. `None` = use settings default.
    pub normalize_boost_db: Option<f64>,
    /// Write (keep) thumbnail as a separate file. `None` = use settings default.
    pub write_thumbnail: Option<bool>,
    /// Additional gain in dB applied on top of normalization. `None` = use settings default.
    pub audio_gain_target: Option<f64>,
    /// Browser to extract cookies from. `None` = use settings default.
    pub cookies_from_browser: Option<BrowserType>,
    /// Path to a Netscape cookies file. `None` = use settings default.
    pub cookies_file: Option<String>,
    /// HTTP/SOCKS proxy URL. `None` = use settings default.
    pub proxy: Option<String>,
    /// Download rate limit string (e.g. `"500K"`). `None` = use settings default.
    pub rate_limit: Option<String>,
    /// Output filename template. `None` = use settings default.
    pub output_template: Option<String>,
    /// Embed subtitles into the output container. `None` = use settings default.
    pub embed_subtitles: Option<bool>,
    /// Explicit video encoder override (e.g., "libsvtav1", "libx264").
    /// `None` = auto-select best available encoder for the target codec.
    /// An empty string fails to deserialize with a clear error (#642) rather
    /// than silently reaching the recode stage as a would-be empty override.
    #[serde(default)]
    pub video_encoder: Option<VideoEncoderName>,
    /// Target container for recode (e.g. `Mp4`, `Mkv`, `WebM`).
    /// Takes precedence over `recode_video` when both are set.
    /// `None` = use `recode_video` value.
    #[serde(default)]
    pub recode_container: Option<ContainerFormat>,
    /// Audio handling during video recode.
    /// `None` = use default (Copy).
    #[serde(default)]
    pub recode_audio: Option<RecodeAudioMode>,
    /// Encoder threads for video recode (1-64). `None` = auto.
    #[serde(default)]
    pub recode_threads: Option<u32>,
    /// Encoder preset override for video recode. `None` = per-codec default.
    #[serde(default)]
    pub recode_preset: Option<String>,
    /// `VPx` deadline mode (`good`, `best`, `realtime`). `None` = encoder default.
    #[serde(default)]
    pub recode_deadline: Option<rdlp_types::VpxDeadline>,
    /// libvpx `-cpu-used` for VP8/VP9 recode. `None` = encoder default.
    /// Range is encoder-specific (validated by `validate_speed_controls`).
    #[serde(default)]
    pub recode_cpu_used: Option<i32>,
    /// HEVC / VVC speed-level knob (encoder-specific range). `None` = encoder default.
    #[serde(default)]
    pub recode_speed_level: Option<u32>,
    /// Enable verbose `FFmpeg` log capture for this download.
    /// `None` = use settings default.
    #[serde(default)]
    pub verbose: Option<bool>,
}

/// Playlist membership context — sent from frontend when downloading
/// episodes selected from a playlist. All fields are required when present.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistContext {
    /// Shared UUID identifying this playlist batch (generated by frontend).
    pub playlist_id: String,
    /// Display title of the playlist/series.
    pub playlist_title: String,
    /// 1-based position of this episode in the playlist.
    pub playlist_index: usize,
    /// Total number of episodes in the playlist.
    pub playlist_count: usize,
}

/// Max byte length for an operator-supplied recode preset name. Generous —
/// the longest legitimate value (`wavefrontsynchro=1:tiles=2x2`, set internally)
/// is ~30 bytes; this only guards against a pathological IPC payload.
const MAX_RECODE_PRESET_LEN: usize = 64;

/// Reject an out-of-range explicit recode thread count at the IPC boundary.
///
/// `None` (auto) and `1..=MAX_RECODE_THREADS` are accepted. The desktop path
/// does not run `Config::validate()`, so this is the boundary guard.
fn validate_recode_threads(threads: Option<u32>) -> Result<(), AppError> {
    match threads {
        None => Ok(()),
        Some(t) if (1..=rdlp_types::config::MAX_RECODE_THREADS).contains(&t) => Ok(()),
        Some(t) => Err(AppError::InvalidInput {
            field: "recode_threads".to_owned(),
            message: format!(
                "recode_threads must be 1..={} (got {t})",
                rdlp_types::config::MAX_RECODE_THREADS
            ),
        }),
    }
}

fn validate_recode_preset(preset: Option<&str>) -> Result<(), AppError> {
    match preset {
        Some(p) if p.len() > MAX_RECODE_PRESET_LEN => Err(AppError::InvalidInput {
            field: "recode_preset".to_owned(),
            message: format!("preset name too long (max {MAX_RECODE_PRESET_LEN} bytes)"),
        }),
        _ => Ok(()),
    }
}

/// Merge per-download subtitle options with the app settings defaults.
///
/// `write_subs` reads the global `settings.write_subtitles` only —
/// `options.subtitles` has no per-download UI control yet (tracked in
/// follow-up #606), so it is intentionally not consulted here.
///
/// # Arguments
///
/// * `options` - Frontend-supplied download options.
/// * `settings` - Persisted application settings used as defaults.
/// * `embed_subtitles` - Already-resolved embed-subtitles flag (per-download
///   override or settings default).
///
/// # Returns
///
/// A [`SubtitleOptions`] with every field resolved.
fn build_subtitle_options(
    options: &DownloadOptions,
    settings: &crate::state::AppSettings,
    embed_subtitles: bool,
) -> SubtitleOptions {
    SubtitleOptions {
        write_subs: Some(settings.write_subtitles),
        write_auto_subs: Some(settings.write_auto_subtitles),
        sub_langs: options.subtitle_langs.clone(),
        sub_format: settings.default_subtitle_format,
        embed_subs: embed_subtitles.then_some(true),
        strict_subs: Some(settings.strict_subs),
        verify_sub_urls: Some(settings.verify_sub_urls),
        retry_subs: Some(settings.retry_subs),
    }
}

/// Build the [`NetworkOptions`] DTO from per-download options and persisted settings.
///
/// Every field is `Option`: `None` means "preserve whatever the base `Config` (loaded
/// from the user's config file) already holds". Emitting `Some(default_value)` here
/// would silently clobber a config-file setting, which is the defect class #540/#583
/// tracked.
///
/// # Arguments
///
/// * `options` - Frontend-supplied download options.
/// * `settings` - Persisted application settings used as defaults.
fn build_network_options(
    options: &DownloadOptions,
    settings: &crate::state::AppSettings,
) -> NetworkOptions {
    // The settings half is the same overlay every other command applies
    // (`settings_network_options`); this function adds only the per-download
    // overrides on top of it, so no field can be honoured here and forgotten
    // there — search honoured none of them (#691).
    let from_settings = crate::commands::network::settings_network_options(settings);

    // Rate limit: choose between the two *strings* before parsing. Parsing
    // first and then falling back would let an unparseable per-download value
    // silently inherit the settings rate instead of overriding it.
    let rate_limit = match options.rate_limit.as_deref() {
        Some(per_download) => parse_rate_limit(per_download),
        None => from_settings.rate_limit,
    };

    NetworkOptions {
        cookies_from_browser: options
            .cookies_from_browser
            .or(from_settings.cookies_from_browser),
        cookies_file: options
            .cookies_file
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| from_settings.cookies_file.clone()),
        proxy: options
            .proxy
            .clone()
            .or_else(|| from_settings.proxy.clone()),
        rate_limit,
        ..from_settings
    }
}

/// Start a new download for the given URL.
///
/// Validates the URL, adds a job to the queue, builds a
/// [`DownloadRequest`] from the provided options merged with app
/// settings defaults, and spawns a background task to drive the
/// download and forward events to the frontend.
///
/// # Arguments
///
/// * `url` - Source URL to download.
/// * `options` - Frontend-supplied download options.
/// * `title` - Optional display title (e.g. from search results) to show immediately.
/// * `playlist_context` - Optional playlist membership info for batch downloads.
/// * `state` - Managed application state.
/// * `app` - Tauri application handle for emitting events.
///
/// # Returns
///
/// The UUID string of the newly created download job.
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if URL validation or proxy URL
/// validation fails.
#[tauri::command]
#[allow(clippy::too_many_lines)]
pub async fn start_download(
    url: String,
    options: DownloadOptions,
    title: Option<String>,
    playlist_context: Option<PlaylistContext>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, AppError> {
    // Validate URL at the boundary
    rdlp_security::validate_url_security(&url).map_err(|e| AppError::InvalidInput {
        field: "url".to_owned(),
        message: e.to_string(),
    })?;

    // Generate a unique job ID
    let job_id = uuid::Uuid::new_v4().to_string();

    // Read settings defaults
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    // Validate proxy URL at the boundary if provided per-download.
    if let Some(proxy) = &options.proxy {
        rdlp_security::validate_proxy_url(proxy).map_err(|e| AppError::InvalidInput {
            field: "proxy".to_owned(),
            message: e.to_string(),
        })?;
    }
    // Also validate the settings proxy (may have been loaded from disk before
    // validate_security was added).
    if let Some(proxy) = &settings.proxy {
        rdlp_security::validate_proxy_url(proxy).map_err(|e| AppError::InvalidInput {
            field: "proxy".to_owned(),
            message: e.to_string(),
        })?;
    }

    // Validate recode thread count at the IPC boundary (Config::validate is not
    // called on the desktop path, so this guard is the only enforcement point).
    validate_recode_threads(options.recode_threads)?;
    validate_recode_preset(options.recode_preset.as_deref())?;
    rdlp_ffmpeg::validate_speed_controls(
        rdlp_ffmpeg::resolve_recode_encoder(
            options
                .video_encoder
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            options.recode_video.map(|c| c.as_ext()),
            options.recode_container.map(|c| c.as_ext()),
        ),
        options.recode_preset.as_deref(),
        options.recode_deadline.map(rdlp_types::VpxDeadline::as_str),
        options.recode_cpu_used,
        options.recode_speed_level,
    )
    .map_err(|e| AppError::InvalidInput {
        field: "recode_speed_control".to_owned(),
        message: e.to_string(),
    })?;

    // Save a serializable snapshot of the options for retry (before any partial moves).
    let saved_options = serde_json::to_value(&options)
        .ok()
        .map(|json| SavedDownloadOptions { json });

    // Embed subtitles.
    let embed_subtitles = options.embed_subtitles.unwrap_or(settings.embed_subtitles);

    // Subtitle options (borrows `options`; must run before any `options`
    // field with a non-`Copy` type is moved out below).
    let subtitle_options = build_subtitle_options(&options, &settings, embed_subtitles);

    // Network options (borrows `options`; must run before any `options`
    // field with a non-`Copy` type is moved out below).
    let network_options = build_network_options(&options, &settings);

    // Build the download request from options + settings
    let output_dir = options
        .output_dir
        .map_or_else(|| settings.output_dir.clone(), PathBuf::from);

    let remux = options.remux.or(settings.default_remux);
    let extract_audio = options.extract_audio.or(settings.default_extract_audio);

    // Output template.
    let output_template = options
        .output_template
        .or_else(|| settings.output_template.clone());

    // Write thumbnail.
    let write_thumbnail_resolved = options.write_thumbnail.unwrap_or(settings.write_thumbnail);

    // Add job to queue as Pending and immediately attach the options snapshot.
    {
        let mut queue = state
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.add_job(&job_id, &url, title, playlist_context);
        if let Some(job) = queue.get_job_mut(&job_id) {
            job.options = saved_options;
        }
    }

    let request = DownloadRequest {
        url,
        output: OutputOptions {
            output_dir: Some(output_dir),
            template: output_template,
            ..OutputOptions::default()
        },
        format: FormatOptions {
            selector: options.format,
            audio_multistreams: if options.audio_multistreams {
                Some(true)
            } else {
                None
            },
            ..FormatOptions::default()
        },
        subtitles: subtitle_options,
        postprocess: PostProcessOptions {
            remux,
            extract_audio,
            recode_video: options.recode_video,
            video_encoder: options.video_encoder,
            recode_container: options.recode_container,
            recode_audio: options.recode_audio,
            recode_threads: options.recode_threads,
            recode_preset: options.recode_preset,
            recode_deadline: options.recode_deadline,
            recode_cpu_used: options.recode_cpu_used,
            recode_speed_level: options.recode_speed_level,
            embed_thumbnail: Some(options.embed_thumbnail),
            embed_metadata: Some(settings.embed_metadata),
            write_thumbnail: write_thumbnail_resolved.then_some(true),
            normalize_audio: Some(options.normalize_audio.unwrap_or(settings.normalize_audio)),
            loudnorm: Some(options.loudnorm.unwrap_or(settings.loudnorm)),
            loudnorm_preset: options
                .loudnorm_preset
                .or_else(|| settings.loudnorm_preset.clone()),
            loudnorm_target_i: options.loudnorm_target_i.or(settings.loudnorm_target_i),
            loudnorm_target_tp: options.loudnorm_target_tp.or(settings.loudnorm_target_tp),
            loudnorm_target_lra: options.loudnorm_target_lra.or(settings.loudnorm_target_lra),
            loudnorm_dynamic: Some(
                options
                    .loudnorm_dynamic
                    .unwrap_or(settings.loudnorm_dynamic),
            ),
            loudnorm_precompress: Some(
                options
                    .loudnorm_precompress
                    .unwrap_or(settings.loudnorm_precompress),
            ),
            normalize_boost: Some(options.normalize_boost.unwrap_or(settings.normalize_boost)),
            normalize_boost_db: options
                .normalize_boost_db
                .or(settings.normalize_boost_db)
                .or_else(|| options.audio_gain_target.or(settings.audio_gain_target)),
            ..PostProcessOptions::default()
        },
        network: network_options,
        verbose: if options.verbose.unwrap_or(settings.verbose) {
            Some(true)
        } else {
            None
        },
        ..DownloadRequest::default()
    };

    // Start the download via rdlp-api (must be outside any lock scope)
    let mut handle = state.client.download(request);

    // Atomically store the cancel token and transition to Running in a
    // single lock acquisition. This prevents concurrent commands from
    // observing the job in a Pending-with-cancel or Running-without-cancel
    // intermediate state.
    let cancel_token = handle.cancel_token();
    let now = chrono::Utc::now().timestamp();
    {
        let mut queue = state
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.start_job(&job_id, Box::new(move || cancel_token.cancel()), now);
    }

    let queue = state.queue.clone();
    let id = job_id.clone();

    // Spawn background event loop.
    //
    // Lock safety: The `MutexGuard` acquired below is dropped at the
    // end of each loop iteration before `.recv().await` yields. This
    // ensures the lock is never held across an await point, avoiding
    // both deadlocks and prolonged contention. The pattern is sound
    // because `MutexGuard` (from `std::sync::Mutex`) is `!Send`, and
    // Rust prevents it from being held across `.await` at compile time.
    tokio::spawn(async move {
        while let Some(event) = handle.events().recv().await {
            // Forward to frontend
            events::emit_event(&app, &id, &event);

            // Update queue state based on event
            let mut q = queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(job) = q.get_job_mut(&id) {
                match &event {
                    Event::Progress { progress, .. } => {
                        job.progress = progress.progress.map(|p| f64::from(p.fraction()));
                        job.speed = Some(progress.speed_string());
                        job.eta = progress.eta.as_ref().map(crate::events::format_eta);
                    }
                    Event::MetadataReady { info, .. } => {
                        job.title = Some(info.title.clone());
                    }
                    Event::Completed { output_files, .. } => {
                        job.status = JobStatus::Completed;
                        job.progress = Some(1.0);
                        job.stage = None;
                        job.clear_current_unit();
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                        job.output_path = output_files.first().map(|p| p.display().to_string());
                    }
                    Event::Failed { error, .. } => {
                        // stage + current_unit intentionally left as-is: only read while in-flight / processing.
                        job.status = JobStatus::Failed;
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                        job.error = Some(error.user_message().into_owned());
                        job.retryable = error.is_retryable();
                    }
                    Event::Cancelled { .. } => {
                        job.status = JobStatus::Cancelled;
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                    }
                    Event::PostProcessing { stage, .. } => {
                        job.begin_postprocessing(stage.clone());
                    }
                    Event::PostProcessProgress {
                        stage, progress, ..
                    } => {
                        // PostProcessProgress.progress is a `Progress` directly (NOT the
                        // Option<Progress> wrapped in Event::Progress.progress).
                        job.set_postprocess_progress(stage.clone(), f64::from(progress.fraction()));
                    }
                    Event::PlaylistItemStarted {
                        index,
                        total,
                        title,
                        ..
                    } => {
                        job.set_current_unit(*index, *total, title.clone());
                    }
                    Event::UnitCompleted { .. } => {
                        job.clear_current_unit();
                    }
                    _ => {}
                }
            }
        }
    });

    Ok(job_id)
}

#[cfg(test)]
mod tests {
    //! Tests for the normalization option merge logic in [`start_download`].
    //!
    //! Because `start_download` is an async Tauri command that requires
    //! managed `State` and `AppHandle`, it cannot be called directly in
    //! unit tests.  Instead, the merge pattern is replicated here in
    //! [`merge_normalize_options`] so the semantics can be verified in
    //! isolation.

    use super::*;
    use crate::state::AppSettings;

    /// Replicates the normalization-field merge from `start_download`
    /// (the `postprocess` block) for isolated testing.
    ///
    /// # Arguments
    ///
    /// * `options` - Frontend-supplied download options.
    /// * `settings` - Persisted application settings used as defaults.
    ///
    /// # Returns
    ///
    /// A [`PostProcessOptions`] with every normalization field resolved.
    fn merge_normalize_options(
        options: &DownloadOptions,
        settings: &AppSettings,
    ) -> PostProcessOptions {
        PostProcessOptions {
            normalize_audio: Some(options.normalize_audio.unwrap_or(settings.normalize_audio)),
            loudnorm: Some(options.loudnorm.unwrap_or(settings.loudnorm)),
            loudnorm_preset: options
                .loudnorm_preset
                .clone()
                .or_else(|| settings.loudnorm_preset.clone()),
            loudnorm_target_i: options.loudnorm_target_i.or(settings.loudnorm_target_i),
            loudnorm_target_tp: options.loudnorm_target_tp.or(settings.loudnorm_target_tp),
            loudnorm_target_lra: options.loudnorm_target_lra.or(settings.loudnorm_target_lra),
            loudnorm_dynamic: Some(
                options
                    .loudnorm_dynamic
                    .unwrap_or(settings.loudnorm_dynamic),
            ),
            loudnorm_precompress: Some(
                options
                    .loudnorm_precompress
                    .unwrap_or(settings.loudnorm_precompress),
            ),
            normalize_boost: Some(options.normalize_boost.unwrap_or(settings.normalize_boost)),
            normalize_boost_db: options.normalize_boost_db.or(settings.normalize_boost_db),
            ..PostProcessOptions::default()
        }
    }

    /// Build a [`DownloadOptions`] with all normalization fields set to `None`.
    fn default_download_options() -> DownloadOptions {
        DownloadOptions {
            format: None,
            output_dir: None,
            subtitles: false,
            subtitle_langs: Vec::new(),
            remux: None,
            extract_audio: None,
            embed_thumbnail: true,
            audio_multistreams: false,
            recode_video: None,
            normalize_audio: None,
            loudnorm: None,
            loudnorm_preset: None,
            loudnorm_target_i: None,
            loudnorm_target_tp: None,
            loudnorm_target_lra: None,
            loudnorm_dynamic: None,
            loudnorm_precompress: None,
            normalize_boost: None,
            normalize_boost_db: None,
            write_thumbnail: None,
            audio_gain_target: None,
            cookies_from_browser: None,
            cookies_file: None,
            proxy: None,
            rate_limit: None,
            output_template: None,
            embed_subtitles: None,
            video_encoder: None,
            recode_container: None,
            recode_audio: None,
            recode_threads: None,
            recode_preset: None,
            recode_deadline: None,
            recode_cpu_used: None,
            recode_speed_level: None,
            verbose: None,
        }
    }

    // ------------------------------------------------------------------ //
    // Recode thread validation
    // ------------------------------------------------------------------ //

    #[test]
    fn rejects_out_of_range_recode_threads() {
        assert!(validate_recode_threads(Some(0)).is_err());
        assert!(validate_recode_threads(Some(rdlp_types::config::MAX_RECODE_THREADS + 1)).is_err());
        assert!(validate_recode_threads(Some(8)).is_ok());
        assert!(validate_recode_threads(None).is_ok());
    }

    #[test]
    fn rejects_overlong_recode_preset() {
        assert!(validate_recode_preset(Some(&"x".repeat(65))).is_err());
        assert!(validate_recode_preset(Some("faster")).is_ok());
        assert!(validate_recode_preset(None).is_ok());
    }

    #[test]
    fn rejects_inapplicable_speed_control() {
        // deadline on an H.264 encoder must be rejected by the boundary validator.
        let err =
            rdlp_ffmpeg::validate_speed_controls(Some("libx264"), None, Some("good"), None, None);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_out_of_range_cpu_used() {
        // VP9 cpu-used range is -8..=8; 16 must be rejected at the boundary.
        let err =
            rdlp_ffmpeg::validate_speed_controls(Some("libvpx-vp9"), None, None, Some(16), None);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_out_of_range_speed_level() {
        // xavs2 speed_level range is 0..=9; 10 must be rejected at the boundary.
        let err =
            rdlp_ffmpeg::validate_speed_controls(Some("libxavs2"), None, None, None, Some(10));
        assert!(err.is_err());
    }

    // ------------------------------------------------------------------ //
    // Rate limit parsing
    // ------------------------------------------------------------------ //

    #[test]
    fn test_parse_rate_limit_bytes() {
        assert_eq!(parse_rate_limit("1000"), Some(1000));
    }

    #[test]
    fn test_parse_rate_limit_kilobytes() {
        assert_eq!(parse_rate_limit("500K"), Some(512_000));
        assert_eq!(parse_rate_limit("500k"), Some(512_000));
    }

    #[test]
    fn test_parse_rate_limit_megabytes() {
        assert_eq!(parse_rate_limit("2M"), Some(2 * 1024 * 1024));
    }

    #[test]
    fn test_parse_rate_limit_empty() {
        assert_eq!(parse_rate_limit(""), None);
    }

    #[test]
    fn test_parse_rate_limit_invalid() {
        assert_eq!(parse_rate_limit("notanumber"), None);
    }

    // ------------------------------------------------------------------ //
    // A. All-None options fall back to settings defaults
    // ------------------------------------------------------------------ //

    /// When every normalization field in `DownloadOptions` is `None`, the
    /// result MUST equal the corresponding settings value.
    #[test]
    fn test_merge_all_none_uses_settings_defaults() {
        let settings = AppSettings {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_owned()),
            loudnorm_target_i: Some(-23.0),
            loudnorm_target_tp: Some(-2.0),
            loudnorm_target_lra: Some(7.0),
            loudnorm_dynamic: true,
            loudnorm_precompress: true,
            normalize_boost: true,
            normalize_boost_db: Some(8.0),
            ..AppSettings::default()
        };
        let options = default_download_options();
        let result = merge_normalize_options(&options, &settings);

        assert_eq!(result.normalize_audio, Some(true));
        assert_eq!(result.loudnorm, Some(true));
        assert_eq!(result.loudnorm_preset.as_deref(), Some("broadcast"));
        assert_eq!(result.loudnorm_target_i, Some(-23.0));
        assert_eq!(result.loudnorm_target_tp, Some(-2.0));
        assert_eq!(result.loudnorm_target_lra, Some(7.0));
        assert_eq!(result.loudnorm_dynamic, Some(true));
        assert_eq!(result.loudnorm_precompress, Some(true));
        assert_eq!(result.normalize_boost, Some(true));
        assert_eq!(result.normalize_boost_db, Some(8.0));
    }

    // ------------------------------------------------------------------ //
    // B. Per-download overrides take precedence over settings
    // ------------------------------------------------------------------ //

    /// Explicit values in `DownloadOptions` MUST shadow settings defaults.
    #[test]
    fn test_merge_overrides_take_precedence() {
        let settings = AppSettings {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_owned()),
            loudnorm_target_i: Some(-23.0),
            loudnorm_target_tp: Some(-2.0),
            loudnorm_target_lra: Some(7.0),
            loudnorm_dynamic: true,
            loudnorm_precompress: true,
            normalize_boost: true,
            normalize_boost_db: Some(12.0),
            ..AppSettings::default()
        };
        let options = DownloadOptions {
            normalize_audio: Some(false),
            loudnorm: Some(false),
            loudnorm_preset: Some("streaming".to_owned()),
            loudnorm_target_i: Some(-14.0),
            loudnorm_target_tp: Some(-1.0),
            loudnorm_target_lra: Some(11.0),
            loudnorm_dynamic: Some(false),
            loudnorm_precompress: Some(false),
            normalize_boost: Some(false),
            normalize_boost_db: Some(6.0),
            ..default_download_options()
        };
        let result = merge_normalize_options(&options, &settings);

        assert_eq!(result.normalize_audio, Some(false));
        assert_eq!(result.loudnorm, Some(false));
        assert_eq!(result.loudnorm_preset.as_deref(), Some("streaming"));
        assert_eq!(result.loudnorm_target_i, Some(-14.0));
        assert_eq!(result.loudnorm_target_tp, Some(-1.0));
        assert_eq!(result.loudnorm_target_lra, Some(11.0));
        assert_eq!(result.loudnorm_dynamic, Some(false));
        assert_eq!(result.loudnorm_precompress, Some(false));
        assert_eq!(result.normalize_boost, Some(false));
        assert_eq!(result.normalize_boost_db, Some(6.0));
    }

    // ------------------------------------------------------------------ //
    // C. Mixed: some overridden, some defaulted
    // ------------------------------------------------------------------ //

    /// Overriding only a subset of fields MUST leave the rest falling
    /// through to the settings defaults.
    #[test]
    fn test_merge_partial_overrides() {
        let settings = AppSettings {
            normalize_audio: false,
            loudnorm: true,
            loudnorm_preset: Some("broadcast".to_owned()),
            loudnorm_target_i: Some(-23.0),
            loudnorm_dynamic: false,
            ..AppSettings::default()
        };
        // Override normalize_audio and loudnorm_target_i only; leave
        // loudnorm_preset as None so it falls back to the settings value.
        let options = DownloadOptions {
            normalize_audio: Some(true),
            loudnorm_target_i: Some(-16.0),
            ..default_download_options()
        };
        let result = merge_normalize_options(&options, &settings);

        // Overridden fields
        assert_eq!(result.normalize_audio, Some(true));
        assert_eq!(result.loudnorm_target_i, Some(-16.0));

        // Fields that fall through to settings
        assert_eq!(result.loudnorm, Some(true));
        assert_eq!(result.loudnorm_preset.as_deref(), Some("broadcast"));
        assert_eq!(result.loudnorm_dynamic, Some(false));
    }

    // ------------------------------------------------------------------ //
    // D. Default settings + default options = all disabled
    // ------------------------------------------------------------------ //

    /// When both options and settings are at their default values every
    /// normalization flag MUST be disabled and every target MUST be `None`.
    #[test]
    fn test_merge_all_defaults_all_disabled() {
        let settings = AppSettings::default();
        let options = default_download_options();
        let result = merge_normalize_options(&options, &settings);

        assert_eq!(result.normalize_audio, Some(false));
        assert_eq!(result.loudnorm, Some(false));
        assert!(result.loudnorm_preset.is_none());
        assert!(result.loudnorm_target_i.is_none());
        assert!(result.loudnorm_target_tp.is_none());
        assert!(result.loudnorm_target_lra.is_none());
        assert_eq!(result.loudnorm_dynamic, Some(false));
        assert_eq!(result.loudnorm_precompress, Some(false));
        assert_eq!(result.normalize_boost, Some(false));
        assert!(result.normalize_boost_db.is_none());
    }

    // ------------------------------------------------------------------ //
    // E. Option<f64> fields: None option + None settings → None result
    // ------------------------------------------------------------------ //

    /// When both the download option and the settings value for an
    /// `Option<f64>` field are `None`, the result MUST also be `None`.
    #[test]
    fn test_merge_option_f64_both_none_stays_none() {
        let settings = AppSettings {
            loudnorm_target_i: None,
            loudnorm_target_tp: None,
            loudnorm_target_lra: None,
            normalize_boost_db: None,
            ..AppSettings::default()
        };
        let options = DownloadOptions {
            loudnorm_target_i: None,
            loudnorm_target_tp: None,
            loudnorm_target_lra: None,
            normalize_boost_db: None,
            ..default_download_options()
        };
        let result = merge_normalize_options(&options, &settings);

        assert!(result.loudnorm_target_i.is_none());
        assert!(result.loudnorm_target_tp.is_none());
        assert!(result.loudnorm_target_lra.is_none());
        assert!(result.normalize_boost_db.is_none());
    }

    // ------------------------------------------------------------------ //
    // F. unwrap_or vs or semantics
    // ------------------------------------------------------------------ //

    /// For `bool` fields (merged via `unwrap_or`): `Some(false)` in the
    /// download options MUST override a `true` settings default.
    #[test]
    fn test_merge_bool_some_false_overrides_true_settings() {
        let settings = AppSettings {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_dynamic: true,
            loudnorm_precompress: true,
            normalize_boost: true,
            ..AppSettings::default()
        };
        let options = DownloadOptions {
            normalize_audio: Some(false),
            loudnorm: Some(false),
            loudnorm_dynamic: Some(false),
            loudnorm_precompress: Some(false),
            normalize_boost: Some(false),
            ..default_download_options()
        };
        let result = merge_normalize_options(&options, &settings);

        assert_eq!(result.normalize_audio, Some(false));
        assert_eq!(result.loudnorm, Some(false));
        assert_eq!(result.loudnorm_dynamic, Some(false));
        assert_eq!(result.loudnorm_precompress, Some(false));
        assert_eq!(result.normalize_boost, Some(false));
    }

    /// For `Option<f64>` fields (merged via `or`): `None` in the download
    /// options MUST pass through to the settings value; `Some(x)` MUST
    /// override the settings value.
    #[test]
    fn test_merge_option_f64_or_semantics() {
        let settings = AppSettings {
            loudnorm_target_i: Some(-23.0),
            loudnorm_target_tp: Some(-2.0),
            normalize_boost_db: Some(8.0),
            ..AppSettings::default()
        };

        // None option → falls through to settings
        let none_options = default_download_options();
        let result = merge_normalize_options(&none_options, &settings);
        assert_eq!(result.loudnorm_target_i, Some(-23.0));
        assert_eq!(result.loudnorm_target_tp, Some(-2.0));
        assert_eq!(result.normalize_boost_db, Some(8.0));

        // Some(x) option → overrides settings
        let some_options = DownloadOptions {
            loudnorm_target_i: Some(-14.0),
            loudnorm_target_tp: Some(-1.0),
            normalize_boost_db: Some(4.0),
            ..default_download_options()
        };
        let result = merge_normalize_options(&some_options, &settings);
        assert_eq!(result.loudnorm_target_i, Some(-14.0));
        assert_eq!(result.loudnorm_target_tp, Some(-1.0));
        assert_eq!(result.normalize_boost_db, Some(4.0));
    }

    // ------------------------------------------------------------------ //
    // G. Network timeouts: AppSettings → NetworkOptions plumbing
    //    (folded into `build_network_options` coverage — the former
    //    `merge_network_options` test-local fork is gone; see section I)
    // ------------------------------------------------------------------ //

    /// When `AppSettings` carries the timeout fields, they MUST propagate
    /// into the `NetworkOptions` produced by `build_network_options`.
    #[test]
    fn test_settings_timeouts_propagate_to_network_options() {
        let settings = AppSettings {
            socket_timeout: Some(45),
            read_timeout: Some(120),
            pool_idle_timeout: Some(0),
            download_timeout: Some(7200),
            merge_timeout: Some(600),
            ..AppSettings::default()
        };
        let options = default_download_options();
        let net = build_network_options(&options, &settings);
        assert_eq!(net.timeout_secs, Some(45));
        assert_eq!(net.read_timeout_secs, Some(120));
        assert_eq!(net.pool_idle_timeout_secs, Some(0));
        assert_eq!(net.download_timeout_secs, Some(7200));
        assert_eq!(net.merge_timeout_secs, Some(600));
    }

    /// Default `AppSettings` MUST leave all timeout fields unset in the
    /// resulting `NetworkOptions` so the HTTP client falls back to its
    /// baked-in defaults.
    #[test]
    fn test_default_settings_leave_timeouts_unset_in_network_options() {
        let settings = AppSettings::default();
        let options = default_download_options();
        let net = build_network_options(&options, &settings);
        assert!(net.timeout_secs.is_none());
        assert!(net.read_timeout_secs.is_none());
        assert!(net.pool_idle_timeout_secs.is_none());
        assert!(net.download_timeout_secs.is_none());
        assert!(net.merge_timeout_secs.is_none());
    }

    // ------------------------------------------------------------------ //
    // H. Subtitle options: settings -> SubtitleOptions bridge
    // ------------------------------------------------------------------ //

    /// `write_subs` MUST read the global `settings.write_subtitles` value
    /// only. This is the regression guard for the misleading
    /// `options.subtitles || settings.write_subtitles` OR that was removed:
    /// `options.subtitles` has no per-download UI (hardcoded `false` in the
    /// frontend; tracked in #606), so it must not influence the result.
    #[test]
    fn test_write_subs_follows_settings_only() {
        let options = default_download_options();

        let settings_on = AppSettings {
            write_subtitles: true,
            ..AppSettings::default()
        };
        let result_on = build_subtitle_options(&options, &settings_on, false);
        assert_eq!(result_on.write_subs, Some(true));

        let settings_off = AppSettings {
            write_subtitles: false,
            ..AppSettings::default()
        };
        let result_off = build_subtitle_options(&options, &settings_off, false);
        assert_eq!(result_off.write_subs, Some(false));

        // Regression guard for the removed `options.subtitles || settings.write_subtitles`
        // OR: an explicit per-download `subtitles: true` combined with the setting OFF
        // yields `Some(true)` under the old code (`true || false`) but MUST be
        // `Some(false)` now — `options.subtitles` no longer influences the result
        // (per-download control deferred to #606). This case discriminates the fix.
        let options_subs_true = DownloadOptions {
            subtitles: true,
            ..default_download_options()
        };
        let result_override = build_subtitle_options(&options_subs_true, &settings_off, false);
        assert_eq!(
            result_override.write_subs,
            Some(false),
            "options.subtitles must not influence write_subs; only the setting drives it",
        );
    }

    /// Each of the 4 remaining settings-only subtitle bools MUST reach the
    /// corresponding `SubtitleOptions` field verbatim.
    #[test]
    fn test_subtitle_settings_bools_propagate_to_dto() {
        let options = default_download_options();
        let settings = AppSettings {
            write_auto_subtitles: true,
            strict_subs: true,
            verify_sub_urls: true,
            retry_subs: true,
            ..AppSettings::default()
        };
        let result = build_subtitle_options(&options, &settings, false);

        assert_eq!(result.write_auto_subs, Some(true));
        assert_eq!(result.strict_subs, Some(true));
        assert_eq!(result.verify_sub_urls, Some(true));
        assert_eq!(result.retry_subs, Some(true));
    }

    /// Default (all-false) settings MUST leave every subtitle bool field
    /// `Some(false)` — negative counterpart of the previous test, proving
    /// each field is independently wired rather than defaulting to `true`.
    #[test]
    fn test_subtitle_settings_bools_default_to_false() {
        let options = default_download_options();
        let settings = AppSettings::default();
        let result = build_subtitle_options(&options, &settings, false);

        assert_eq!(result.write_subs, Some(false));
        assert_eq!(result.write_auto_subs, Some(false));
        assert_eq!(result.strict_subs, Some(false));
        assert_eq!(result.verify_sub_urls, Some(false));
        assert_eq!(result.retry_subs, Some(false));
    }

    // ------------------------------------------------------------------ //
    // I. Network options: build_network_options throughput settings bridge
    // ------------------------------------------------------------------ //

    #[test]
    fn build_network_options_passes_throughput_settings_through() {
        let settings = crate::state::AppSettings {
            concurrent_fragments: Some(16),
            buffer_size: Some(8 * 1024 * 1024),
            parallel_threshold: Some(20 * 1024 * 1024),
            hls_head_probe_timeout: Some(30),
            ..crate::state::AppSettings::default()
        };
        let net = build_network_options(&default_download_options(), &settings);
        assert_eq!(net.concurrent_fragments, Some(16));
        assert_eq!(net.buffer_size, Some(8 * 1024 * 1024));
        assert_eq!(net.parallel_threshold, Some(20 * 1024 * 1024));
        assert_eq!(net.hls_head_probe_timeout, Some(30));
    }

    /// `None` must stay `None` all the way to the DTO — the desktop loads a real
    /// config.toml as its base Config, so emitting `Some(default)` here would clobber
    /// the user's config file.
    #[test]
    fn build_network_options_leaves_unset_throughput_settings_none() {
        let net = build_network_options(
            &default_download_options(),
            &crate::state::AppSettings::default(),
        );
        assert!(net.concurrent_fragments.is_none());
        assert!(net.buffer_size.is_none());
        assert!(net.parallel_threshold.is_none());
        assert!(net.hls_head_probe_timeout.is_none());
    }

    #[test]
    fn build_network_options_still_carries_existing_timeouts() {
        let settings = crate::state::AppSettings {
            socket_timeout: Some(45),
            download_timeout: Some(600),
            ..crate::state::AppSettings::default()
        };
        let net = build_network_options(&default_download_options(), &settings);
        assert_eq!(net.timeout_secs, Some(45));
        assert_eq!(net.download_timeout_secs, Some(600));
    }

    // ------------------------------------------------------------------ //
    // I.1 Network options: per-download-overrides-settings precedence
    //     (cookies, proxy, rate limit)
    // ------------------------------------------------------------------ //

    /// `cookies_file`: a per-download value MUST win over a settings value.
    #[test]
    fn build_network_options_cookies_file_per_download_wins() {
        let options = DownloadOptions {
            cookies_file: Some("/per-download/cookies.txt".to_owned()),
            ..default_download_options()
        };
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/settings/cookies.txt")),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(
            net.cookies_file,
            Some(PathBuf::from("/per-download/cookies.txt"))
        );
    }

    /// `cookies_file`: absent per-download MUST fall back to the settings value.
    #[test]
    fn build_network_options_cookies_file_falls_back_to_settings() {
        let options = default_download_options();
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/settings/cookies.txt")),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(
            net.cookies_file,
            Some(PathBuf::from("/settings/cookies.txt"))
        );
    }

    /// `cookies_from_browser`: a per-download value MUST win over a settings value.
    #[test]
    fn build_network_options_cookies_from_browser_per_download_wins() {
        let options = DownloadOptions {
            cookies_from_browser: Some(BrowserType::Chrome),
            ..default_download_options()
        };
        let settings = AppSettings {
            cookies_from_browser: Some(BrowserType::Firefox),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.cookies_from_browser, Some(BrowserType::Chrome));
    }

    /// `cookies_from_browser`: absent per-download MUST fall back to settings.
    #[test]
    fn build_network_options_cookies_from_browser_falls_back_to_settings() {
        let options = default_download_options();
        let settings = AppSettings {
            cookies_from_browser: Some(BrowserType::Firefox),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.cookies_from_browser, Some(BrowserType::Firefox));
    }

    /// `proxy`: a per-download value MUST win over a settings value.
    #[test]
    fn build_network_options_proxy_per_download_wins() {
        let options = DownloadOptions {
            proxy: Some("http://per-download:3128".to_owned()),
            ..default_download_options()
        };
        let settings = AppSettings {
            proxy: Some("http://settings:3128".to_owned()),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.proxy.as_deref(), Some("http://per-download:3128"));
    }

    /// `proxy`: absent per-download MUST fall back to the settings value.
    #[test]
    fn build_network_options_proxy_falls_back_to_settings() {
        let options = default_download_options();
        let settings = AppSettings {
            proxy: Some("http://settings:3128".to_owned()),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.proxy.as_deref(), Some("http://settings:3128"));
    }

    /// `rate_limit`: a per-download value MUST win over a settings value, and
    /// the winning string MUST be parsed to bytes/sec (not passed through raw).
    #[test]
    fn build_network_options_rate_limit_per_download_wins_and_parses() {
        let options = DownloadOptions {
            rate_limit: Some("500K".to_owned()),
            ..default_download_options()
        };
        let settings = AppSettings {
            rate_limit: Some("2M".to_owned()),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.rate_limit, Some(512_000));
    }

    /// `rate_limit`: absent per-download MUST fall back to the settings
    /// value, which MUST also be parsed to bytes/sec.
    #[test]
    fn build_network_options_rate_limit_falls_back_to_settings_and_parses() {
        let options = default_download_options();
        let settings = AppSettings {
            rate_limit: Some("2M".to_owned()),
            ..AppSettings::default()
        };
        let net = build_network_options(&options, &settings);
        assert_eq!(net.rate_limit, Some(2 * 1024 * 1024));
    }

    /// `sub_langs` MUST pass through from `options` and `sub_format` from
    /// `settings`; `embed_subs` reflects the caller-resolved flag.
    #[test]
    fn test_subtitle_langs_and_format_pass_through() {
        let options = DownloadOptions {
            subtitle_langs: vec!["en".to_owned(), "sv".to_owned()],
            ..default_download_options()
        };
        let settings = AppSettings {
            default_subtitle_format: Some(rdlp_types::SubtitleFormat::Srt),
            ..AppSettings::default()
        };

        let result_no_embed = build_subtitle_options(&options, &settings, false);
        assert_eq!(
            result_no_embed.sub_langs,
            vec!["en".to_owned(), "sv".to_owned()]
        );
        assert_eq!(
            result_no_embed.sub_format,
            Some(rdlp_types::SubtitleFormat::Srt)
        );
        assert!(result_no_embed.embed_subs.is_none());

        let result_embed = build_subtitle_options(&options, &settings, true);
        assert_eq!(result_embed.embed_subs, Some(true));
    }
}

/// Cancel an in-progress download by its job ID.
///
/// Takes the cancel closure from the queue and invokes it. The
/// background event loop handles the resulting `Event::Cancelled` and
/// updates the job status.
///
/// # Arguments
///
/// * `job_id` - UUID of the download job to cancel.
/// * `state` - Managed application state.
///
/// # Errors
///
/// Returns [`AppError::DownloadFailed`] if the job is not found or
/// has no active cancel function.
#[tauri::command]
pub async fn cancel_download(job_id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let cancel_fn = {
        let mut queue = state
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue
            .take_cancel(&job_id)
            .ok_or_else(|| AppError::DownloadFailed {
                job_id: job_id.clone(),
                message: "No active cancel function for this job".to_owned(),
                retryable: false,
            })?
    };

    // Only invoke the cancel token. The background event loop handles
    // the Event::Cancelled and sets status/completed_at as the single
    // source of truth — no direct mutation here.
    cancel_fn();

    Ok(())
}

/// Retrieve the current download queue.
///
/// Returns all queued, active, and completed download jobs in
/// insertion order.
///
/// # Arguments
///
/// * `state` - Managed application state.
///
/// # Errors
///
/// This function currently does not return errors but uses `Result` for
/// a forward-compatible IPC signature.
///
/// # Returns
///
/// A vector of all [`DownloadJob`]s, cloned from the queue.
#[tauri::command]
pub async fn queue(state: State<'_, AppState>) -> Result<Vec<DownloadJob>, AppError> {
    let jobs = state
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .all_jobs()
        .into_iter()
        .cloned()
        .collect();
    Ok(jobs)
}

/// Remove a completed, failed, or cancelled download from the queue.
///
/// Only jobs in a terminal state may be removed. Active or pending
/// jobs cannot be removed (cancel them first).
///
/// # Arguments
///
/// * `job_id` - UUID of the download job to remove.
/// * `state` - Managed application state.
///
/// # Errors
///
/// Returns [`AppError::DownloadFailed`] if the job cannot be removed
/// (not found or still active).
#[tauri::command]
pub async fn remove_job(job_id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let removed = state
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove_job(&job_id);

    if !removed {
        return Err(AppError::DownloadFailed {
            job_id,
            message: "Job not found or still active".to_owned(),
            retryable: false,
        });
    }

    Ok(())
}

/// Remove all completed, failed, and cancelled downloads from the queue.
///
/// Active or pending jobs are left in place.
///
/// # Arguments
///
/// * `state` - Managed application state.
///
/// # Errors
///
/// This function currently does not return errors but uses `Result` for
/// a forward-compatible IPC signature.
///
/// # Returns
///
/// The number of jobs removed.
#[tauri::command]
pub async fn clear_completed_jobs(state: State<'_, AppState>) -> Result<usize, AppError> {
    let count = state
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear_completed();
    Ok(count)
}

/// Retrieve the options used when a job was originally started.
///
/// Returns the serialized [`DownloadOptions`] snapshot stored on the job,
/// which the frontend uses to populate retry dialogs.
///
/// # Arguments
///
/// * `job_id` - UUID of the download job.
/// * `state` - Managed application state.
///
/// # Returns
///
/// The JSON value of the original [`DownloadOptions`], or `null` if not recorded.
///
/// # Errors
///
/// Returns [`AppError::DownloadFailed`] if the job is not found.
#[tauri::command]
pub async fn job_options(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, AppError> {
    let options = state
        .queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_job(&job_id)
        .ok_or_else(|| AppError::DownloadFailed {
            job_id: job_id.clone(),
            message: "Job not found".to_owned(),
            retryable: false,
        })?
        .options
        .as_ref()
        .map(|o| o.json.clone());
    Ok(options)
}

// ============================================================================
// Helpers
// ============================================================================

/// Parse a human-readable rate limit string into bytes per second.
///
/// Accepts suffixes: `K`/`k` (kilobytes), `M`/`m` (megabytes), `G`/`g` (gigabytes).
/// Bare integers are treated as bytes per second.
///
/// Returns `None` on parse error.
pub(super) fn parse_rate_limit(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if s.ends_with('K') || s.ends_with('k') {
        (&s[..s.len() - 1], 1_024u64)
    } else if s.ends_with('M') || s.ends_with('m') {
        (&s[..s.len() - 1], 1_024 * 1_024)
    } else if s.ends_with('G') || s.ends_with('g') {
        (&s[..s.len() - 1], 1_024 * 1_024 * 1_024)
    } else {
        (s, 1u64)
    };

    num_str.trim().parse::<u64>().ok().map(|n| n * multiplier)
}
