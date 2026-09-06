//! Format listing IPC commands.
//!
//! Exposes available download formats for a given URL so the
//! frontend can present an interactive format selector. The
//! [`formats()`] command validates the URL via SSRF protection,
//! extracts metadata through the rdlp-api client, and returns a
//! serialisable `FormatListResponse` containing format details,
//! subtitles, thumbnail URL, and duration.

mod models;

pub(crate) use models::*;

use rdlp_types::protocol::DownloadProtocol;
use rdlp_types::{Format, FormatSelector};
use tauri::State;

use crate::error::AppError;
use crate::state::AppState;
use rdlp_types::boundary::Action;

/// Retrieve available formats for a URL.
///
/// Validates the URL for SSRF safety, extracts metadata via the
/// rdlp-api client, and maps the result into a frontend-friendly
/// `FormatListResponse`.
///
/// # Arguments
///
/// * `url` - The webpage URL to extract formats from.
/// * `state` - Application state containing the [`rdlp_api::RdlpClient`].
///
/// # Returns
///
/// A `FormatListResponse` on success, or an [`AppError`] on failure.
///
/// # Errors
///
/// * [`AppError::InvalidInput`] if the URL fails SSRF validation.
/// * [`AppError::ExtractionFailed`] if no metadata is returned.
/// * Other [`AppError`] variants mapped from [`rdlp_api::RdlpApiError`].
#[tauri::command]
pub async fn formats(
    url: String,
    state: State<'_, AppState>,
) -> Result<FormatListResponse, AppError> {
    // 1. SSRF / URL validation
    rdlp_security::validate_url_security(&url)
        .map_err(|e| AppError::invalid_input(Action::new("analyze"), "url", e))?;

    // 2. Extract metadata. The shared client was built once from
    // `config.toml`; the GUI's network settings live in `AppSettings`, so
    // they can only reach this path as a per-call override (#691). Clone out
    // of the guard: it must not be held across the `.await`.
    let network = {
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::commands::network::settings_network_options(&settings)?
    };

    let info_dicts = state
        .client
        .extract_info(&url, &network)
        .await
        .map_err(|e| AppError::from_api(Action::new("analyze"), &e))?;

    // 3. Detect playlist vs single video
    let is_playlist = info_dicts.len() > 1;

    let (first_info, playlist) = if is_playlist {
        // Build playlist metadata from all entries
        let playlist_title = info_dicts
            .first()
            .and_then(|i| i.playlist_title.clone())
            .unwrap_or_else(|| "Untitled Playlist".to_string());
        let playlist_count = info_dicts.len();

        let entries: Vec<PlaylistEntry> = info_dicts
            .iter()
            .enumerate()
            .map(|(i, info)| PlaylistEntry {
                index: info.playlist_index.unwrap_or(i + 1),
                title: info.title.clone(),
                url: info.webpage_url.clone(),
                thumbnail_url: info.thumbnail.clone(),
                duration: info.duration,
                has_sub: true,  // TODO: detect from info if available
                has_dub: false, // TODO: detect from info if available
            })
            .collect();

        let playlist_info = PlaylistInfo {
            title: playlist_title,
            count: playlist_count,
            entries,
        };

        let first = info_dicts.into_iter().next().ok_or_else(|| {
            AppError::extraction_failed(Action::new("analyze"), "no metadata returned")
        })?;

        (first, Some(playlist_info))
    } else {
        let first = info_dicts.into_iter().next().ok_or_else(|| {
            AppError::extraction_failed(Action::new("analyze"), "no metadata returned")
        })?;
        (first, None)
    };

    let info = first_info;

    // 4. Map formats
    let formats = info
        .formats
        .iter()
        .map(|f| FormatInfo {
            format_id: f.format_id.clone(),
            ext: f.ext.clone(),
            format_note: f.format_note.clone(),
            width: f.width,
            height: f.height,
            fps: f.fps,
            tbr: f.tbr,
            vcodec: f.vcodec.as_str().map(str::to_string),
            acodec: f.acodec.as_str().map(str::to_string),
            filesize: f.resolved_filesize(),
            vbr: f.vbr,
            abr: f.abr,
            asr: f.asr,
            protocol: f.protocol.to_string(),
            has_video: f.has_video(),
            has_audio: f.has_audio(),
            audio_group_id: f.audio_group_id.clone(),
        })
        .collect();

    // 5. Map subtitles
    let subtitles = info
        .subtitles
        .as_ref()
        .map(|subs| {
            subs.iter()
                .map(|(lang, entries)| SubtitleInfo {
                    lang: lang.clone(),
                    formats: entries.iter().map(|s| s.ext.clone()).collect(),
                })
                .collect()
        })
        .unwrap_or_default();

    // 6. Build response
    Ok(FormatListResponse {
        title: info.title,
        formats,
        subtitles,
        thumbnail_url: info.thumbnail,
        duration: info.duration,
        is_playlist,
        playlist,
    })
}

/// Validate a yt-dlp format expression and return the matching format IDs.
///
/// Parses the expression using [`rdlp_types::FormatSelector`] and, when
/// `formats` is non-empty, builds [`rdlp_types::Format`] values with
/// full metadata so filter predicates (e.g. `[height<=1080]`) can
/// match correctly.
///
/// # Arguments
///
/// * `expression` - A yt-dlp format selector expression (e.g. `bv[height<=1080]+ba`).
/// * `formats` - Format metadata from the frontend for matching.
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if the expression cannot be parsed
/// by [`rdlp_types::FormatSelector`].
///
/// # Returns
///
/// A list of matching format IDs on success, or an [`AppError`] on
/// parse failure.
#[tauri::command]
pub async fn validate_format_expression(
    expression: String,
    formats: Vec<FormatData>,
) -> Result<Vec<String>, AppError> {
    // Pure in-memory selector parse + map (~<100 µs per call). Below the
    // `spawn_blocking` threshold per the TLS Phase 1 async audit
    // (Finding 5.R1); panic isolation for this site is now provided by
    // the global `std::panic::set_hook` installed in `run()`.
    let selector = FormatSelector::parse(&expression)
        .map_err(|e| AppError::invalid_input(Action::new("analyze"), "expression", e))?;

    if formats.is_empty() {
        return Ok(Vec::new());
    }

    let format_list: Vec<Format> = formats
        .iter()
        .map(|fd| {
            let protocol = fd
                .protocol
                .parse::<DownloadProtocol>()
                .unwrap_or(DownloadProtocol::Https);
            let mut f = Format::new(
                &fd.format_id,
                format!("stub://{}", fd.format_id),
                &fd.ext,
                protocol,
            );
            f.width = fd.width;
            f.height = fd.height;
            f.fps = fd.fps;
            f.tbr = fd.tbr;
            f.vbr = fd.vbr;
            f.abr = fd.abr;
            f.asr = fd.asr;
            f.filesize = fd.filesize;
            f.vcodec = rdlp_api::Codec::from(fd.vcodec.clone());
            f.acodec = rdlp_api::Codec::from(fd.acodec.clone());
            f
        })
        .collect();

    let selected = selector.select(&format_list);
    let matched: Vec<String> = selected.iter().map(|f| f.format_id.clone()).collect();
    Ok(matched)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Analyze returning no metadata is a recorded outcome, not a silent one.
    #[test]
    fn an_empty_analyze_result_is_recorded() {
        testing_logger::setup();
        let _ = AppError::extraction_failed(Action::new("analyze"), "no metadata returned");

        testing_logger::validate(|captured| {
            let body = captured.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("action=analyze"), "got: {body}");
            assert!(body.contains("outcome=failed"), "got: {body}");
            assert!(body.contains("no metadata returned"), "got: {body}");
        });
    }

    #[test]
    fn test_format_info_serialization() {
        let info = FormatInfo {
            format_id: "137".to_owned(),
            ext: "mp4".to_owned(),
            format_note: Some("1080p".to_owned()),
            width: Some(1920),
            height: Some(1080),
            fps: Some(30.0),
            tbr: Some(4500.0),
            vcodec: Some("h264".to_owned()),
            acodec: Some("aac".to_owned()),
            filesize: Some(524_288_000),
            vbr: Some(4000.0),
            abr: Some(128.0),
            asr: Some(44100),
            protocol: "https".to_owned(),
            has_video: true,
            has_audio: true,
            audio_group_id: None,
        };

        let json = serde_json::to_value(&info).expect("serialization");
        assert_eq!(json["format_id"], "137");
        assert_eq!(json["ext"], "mp4");
        assert_eq!(json["width"], 1920);
        assert_eq!(json["height"], 1080);
        assert_eq!(json["has_video"], true);
        assert_eq!(json["has_audio"], true);
        assert_eq!(json["filesize"], 524_288_000u64);
        assert_eq!(json["vbr"], 4000.0);
        assert_eq!(json["abr"], 128.0);
        assert_eq!(json["asr"], 44100);
        assert_eq!(json["protocol"], "https");
    }

    #[test]
    fn test_format_info_optional_fields_null() {
        let info = FormatInfo {
            format_id: "hls-auto".to_owned(),
            ext: "mp4".to_owned(),
            format_note: None,
            width: None,
            height: None,
            fps: None,
            tbr: None,
            vcodec: None,
            acodec: None,
            filesize: None,
            vbr: None,
            abr: None,
            asr: None,
            protocol: "m3u8_native".to_owned(),
            has_video: false,
            has_audio: true,
            audio_group_id: None,
        };

        let json = serde_json::to_value(&info).expect("serialization");
        assert!(json["format_note"].is_null());
        assert!(json["width"].is_null());
        assert!(json["filesize"].is_null());
        assert!(json["vbr"].is_null());
        assert!(json["abr"].is_null());
        assert!(json["asr"].is_null());
        assert_eq!(json["protocol"], "m3u8_native");
        assert!(!json["has_video"].as_bool().unwrap());
        assert!(json["has_audio"].as_bool().unwrap());
    }

    #[test]
    fn test_subtitle_info_serialization() {
        let sub = SubtitleInfo {
            lang: "en".to_owned(),
            formats: vec!["srt".to_owned(), "vtt".to_owned()],
        };

        let json = serde_json::to_value(&sub).expect("serialization");
        assert_eq!(json["lang"], "en");
        let fmts = json["formats"].as_array().unwrap();
        assert_eq!(fmts.len(), 2);
        assert_eq!(fmts[0], "srt");
        assert_eq!(fmts[1], "vtt");
    }

    #[test]
    fn test_format_list_response_serialization() {
        let resp = FormatListResponse {
            title: "Test Video".to_owned(),
            formats: vec![FormatInfo {
                format_id: "22".to_owned(),
                ext: "mp4".to_owned(),
                format_note: Some("720p".to_owned()),
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                tbr: Some(2500.0),
                vcodec: Some("h264".to_owned()),
                acodec: Some("aac".to_owned()),
                filesize: Some(100_000_000),
                vbr: Some(2000.0),
                abr: Some(128.0),
                asr: Some(44100),
                protocol: "https".to_owned(),
                has_video: true,
                has_audio: true,
                audio_group_id: None,
            }],
            subtitles: vec![SubtitleInfo {
                lang: "en".to_owned(),
                formats: vec!["vtt".to_owned()],
            }],
            thumbnail_url: Some("https://example.com/thumb.jpg".to_owned()),
            duration: Some(180.5),
            is_playlist: false,
            playlist: None,
        };

        let json = serde_json::to_value(&resp).expect("serialization");
        assert_eq!(json["title"], "Test Video");
        assert_eq!(json["formats"].as_array().unwrap().len(), 1);
        assert_eq!(json["subtitles"].as_array().unwrap().len(), 1);
        assert_eq!(json["thumbnail_url"], "https://example.com/thumb.jpg");
        assert_eq!(json["duration"], 180.5);
        assert_eq!(json["is_playlist"], false);
        assert!(json["playlist"].is_null());
    }

    #[test]
    fn test_format_list_response_empty() {
        let resp = FormatListResponse {
            title: "Empty Video".to_owned(),
            formats: vec![],
            subtitles: vec![],
            thumbnail_url: None,
            duration: None,
            is_playlist: false,
            playlist: None,
        };

        let json = serde_json::to_value(&resp).expect("serialization");
        assert_eq!(json["title"], "Empty Video");
        assert!(json["formats"].as_array().unwrap().is_empty());
        assert!(json["subtitles"].as_array().unwrap().is_empty());
        assert!(json["thumbnail_url"].is_null());
        assert!(json["duration"].is_null());
        assert_eq!(json["is_playlist"], false);
        assert!(json["playlist"].is_null());
    }

    fn make_format_data(
        format_id: &str,
        ext: &str,
        vcodec: Option<&str>,
        acodec: Option<&str>,
        height: Option<u32>,
    ) -> FormatData {
        FormatData {
            format_id: format_id.to_owned(),
            ext: ext.to_owned(),
            width: height.map(|h| h * 16 / 9),
            height,
            fps: None,
            tbr: None,
            vcodec: vcodec.map(std::borrow::ToOwned::to_owned),
            acodec: acodec.map(std::borrow::ToOwned::to_owned),
            filesize: None,
            vbr: None,
            abr: None,
            asr: None,
            protocol: "https".to_owned(),
        }
    }

    #[tokio::test]
    async fn test_validate_format_expression_valid() {
        let formats = vec![
            make_format_data("137", "mp4", Some("h264"), Some("aac"), Some(1080)),
            make_format_data("140", "m4a", Some("none"), Some("aac"), None),
        ];
        let result = super::validate_format_expression("best".to_owned(), formats).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_format_expression_invalid() {
        let result = super::validate_format_expression(String::new(), vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_validate_format_expression_empty_formats() {
        let result = super::validate_format_expression("bv+ba".to_owned(), vec![]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_validate_format_expression_format_id_match() {
        let formats = vec![
            make_format_data("137", "mp4", Some("h264"), Some("aac"), Some(1080)),
            make_format_data("140", "m4a", Some("none"), Some("aac"), None),
        ];
        let result = super::validate_format_expression("137".to_owned(), formats).await;
        assert!(result.is_ok());
        let matches = result.unwrap();
        assert_eq!(matches, vec!["137"]);
    }

    #[tokio::test]
    async fn test_validate_format_expression_height_filter() {
        let formats = vec![
            make_format_data("v720", "mp4", Some("h264"), Some("none"), Some(720)),
            make_format_data("v1080", "mp4", Some("h264"), Some("none"), Some(1080)),
            make_format_data("a128", "m4a", Some("none"), Some("aac"), None),
        ];
        let result =
            super::validate_format_expression("bv[height<=720]+ba".to_owned(), formats).await;
        assert!(result.is_ok());
        let matches = result.unwrap();
        assert_eq!(matches, vec!["v720", "a128"]);
    }

    /// Regression guard for Finding 5.R1 (TLS Phase 1 async audit).
    ///
    /// On unpatched code the command wrapped its pure-sync body in
    /// `tokio::task::spawn_blocking`. A current-thread runtime built
    /// with `max_blocking_threads(1)` combined with a long-running
    /// occupant on the blocking pool starves the validator. On patched
    /// code the body is inline and runs on the caller's task, so the
    /// blocking pool is irrelevant and the call completes promptly.
    #[test]
    fn test_validate_format_expression_runs_without_spawn_blocking() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .expect("build runtime");

        runtime.block_on(async {
            // Pin the single blocking worker with a long-running task so
            // any `spawn_blocking` inside `validate_format_expression`
            // would queue behind it and miss the timeout.
            let hold = Arc::new(AtomicBool::new(true));
            let hold_clone = Arc::clone(&hold);
            let occupant = tokio::task::spawn_blocking(move || {
                while hold_clone.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            });

            let formats = vec![make_format_data(
                "137",
                "mp4",
                Some("h264"),
                Some("aac"),
                Some(1080),
            )];

            let call = super::validate_format_expression("bestvideo".to_owned(), formats);
            let result = tokio::time::timeout(Duration::from_millis(500), call).await;

            // Release the occupant before asserting so the runtime can
            // shut down cleanly regardless of outcome.
            hold.store(false, Ordering::SeqCst);
            let _ = occupant.await;

            let inner = result.expect("validate_format_expression timed out — blocking pool starvation indicates spawn_blocking is still in use");
            assert!(inner.is_ok(), "validate_format_expression should accept 'bestvideo'");
        });
    }

    #[tokio::test]
    async fn test_validate_format_expression_ext_filter() {
        let formats = vec![
            make_format_data("v1", "mp4", Some("h264"), Some("none"), Some(720)),
            make_format_data("v2", "webm", Some("vp9"), Some("none"), Some(720)),
        ];
        let result = super::validate_format_expression("bv[ext=webm]".to_owned(), formats).await;
        assert!(result.is_ok());
        let matches = result.unwrap();
        assert_eq!(matches, vec!["v2"]);
    }
}
