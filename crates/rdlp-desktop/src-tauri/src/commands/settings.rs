//! Application settings IPC commands.
//!
//! Provides commands to read, update, and interact with persistent
//! application settings such as the download directory. The
//! [`pick_directory`] command uses the native OS folder picker via
//! `tauri-plugin-dialog`, and [`reveal_in_folder`] uses
//! `tauri-plugin-opener` to show a file in the system file manager.

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::state::{AppSettings, AppState, SettingsValidationError};
use rdlp_types::boundary::{Action, Subject};
use rdlp_types::{EffectiveNetwork, EffectiveNormalize, LoudnormPreset, PostProcess};

/// Retrieve the current application settings.
///
/// Locks the shared settings mutex, clones the current
/// [`AppSettings`], and returns them to the frontend.
///
/// # Arguments
///
/// * `state` - Managed application state containing the settings.
///
/// # Returns
///
/// A clone of the current [`AppSettings`].
///
/// # Errors
///
/// This function does not currently return errors but returns
/// `Result` for forward-compatible IPC signatures.
#[tauri::command]
pub async fn settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    Ok(settings)
}

/// The network/download values the engine runs with when an
/// [`AppSettings`] field is `None` (inherit).
///
/// Resolves the client's base [`Config`](rdlp_types::Config) — `config.toml`
/// or the built-in defaults — through the single resolver
/// `Config::effective_network()`, so the GUI's "inherit" placeholders show
/// the value a download will actually use instead of carrying their own copy
/// of the defaults (#611). The base config is loaded once per process, so
/// the frontend caches this indefinitely.
///
/// # Errors
///
/// This function does not currently return errors but returns
/// `Result` for forward-compatible IPC signatures.
#[tauri::command]
pub async fn effective_network(state: State<'_, AppState>) -> Result<EffectiveNetwork, AppError> {
    Ok(state.client.config().effective_network())
}

/// The built-in network/download defaults, `EffectiveNetwork::DEFAULT`,
/// before any `config.toml` layering.
///
/// Distinct from [`effective_network`]: that is what an empty field
/// *inherits*; this is what the GUI *seeds* when the inherited value cannot
/// express the user's intent — e.g. re-enabling idle-connection eviction
/// when the base configuration has disabled it with the `0` sentinel. Serving
/// it over IPC keeps the desktop free of its own copy of the number (#611).
///
/// # Errors
///
/// This function does not currently return errors but returns
/// `Result` for forward-compatible IPC signatures.
#[tauri::command]
pub async fn builtin_network_defaults() -> Result<EffectiveNetwork, AppError> {
    Ok(EffectiveNetwork::DEFAULT)
}

/// The normalization values the engine runs with for a given preset when the
/// [`AppSettings`] target fields are `None` (inherit).
///
/// The I/TP/LRA defaults are PRESET-DEPENDENT, so the GUI cannot fetch this
/// once: it passes the draft's preset (`None` = inherit the base config's),
/// and the payload carries the resolved preset plus the six values for it.
/// Resolves through the single resolver `PostProcess::effective_normalize`
/// on the client's base [`PostProcess`] with the preset overlaid, so the
/// Settings placeholders show the value a download will actually use — the
/// previous hand-copied placeholders were Streaming-only and wrong under
/// `Loud`/`Broadcast` (#611).
///
/// # Errors
///
/// This function does not currently return errors but returns
/// `Result` for forward-compatible IPC signatures.
#[tauri::command]
pub async fn effective_normalize(
    preset: Option<LoudnormPreset>,
    state: State<'_, AppState>,
) -> Result<EffectiveNormalize, AppError> {
    Ok(resolve_effective_normalize(
        &state.client.config().postprocess,
        preset,
    ))
}

/// Overlay `preset` on the base post-process config and resolve it.
///
/// Pure so the command's one decision — "the draft's preset wins over the
/// base's, and `None` inherits" — is testable without managed `State`.
fn resolve_effective_normalize(
    base: &PostProcess,
    preset: Option<LoudnormPreset>,
) -> EffectiveNormalize {
    PostProcess {
        loudnorm_preset: preset.or(base.loudnorm_preset),
        ..base.clone()
    }
    .effective_normalize()
}

/// Update application settings with new values.
///
/// Validates security-sensitive fields (cookies path traversal, proxy URL)
/// before persisting. Locks the shared settings mutex and replaces the
/// current [`AppSettings`] with the provided values.
///
/// # Arguments
///
/// * `settings` - New settings from the frontend.
/// * `state` - Managed application state containing the settings.
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if security validation fails, or
/// [`AppError::Internal`] if saving settings to disk fails.
#[tauri::command]
pub async fn update_settings(
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    settings.validate_security().map_err(|e| {
        let field = match &e {
            SettingsValidationError::CookiesFileTraversal => "cookies_file",
            SettingsValidationError::InvalidProxy(_) => "proxy",
            SettingsValidationError::OutOfRange { field, .. } => field,
        };
        AppError::invalid_input(Action::new("update_settings"), field, e)
    })?;

    let mut current = state
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *current = settings;
    let result = current.save();
    drop(current);
    result.map_err(|e| AppError::internal(Action::new("save_settings"), e))?;

    Ok(())
}

/// Open a native directory picker dialog and return the selected path.
///
/// Uses `tauri-plugin-dialog` to present the OS-native folder selection
/// dialog. Returns the chosen directory path as a [`String`], or `None`
/// if the user cancelled the dialog.
///
/// # Arguments
///
/// * `app` - Tauri application handle for accessing the dialog plugin.
///
/// # Returns
///
/// `Some(path)` if a directory was selected, `None` if the user
/// cancelled.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if the selected path cannot be
/// converted to a UTF-8 string.
#[tauri::command]
pub async fn pick_directory(app: AppHandle) -> Result<Option<String>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    app.dialog().file().pick_folder(move |folder| {
        // Ignore send error — receiver dropped means command was cancelled.
        let _ = tx.send(folder);
    });

    let folder = tokio::time::timeout(Duration::from_secs(300), rx)
        .await
        .map_err(|_| {
            AppError::environment(
                Action::new("pick_directory"),
                "folder picker timed out after 5 minutes",
            )
        })?
        .map_err(|_| {
            AppError::environment(
                Action::new("pick_directory"),
                "folder picker channel closed unexpectedly",
            )
        })?;

    match folder {
        Some(file_path) => {
            let path = file_path
                .into_path()
                .map_err(|e| AppError::environment(Action::new("pick_directory"), e))?;

            let path_str = path
                .to_str()
                .ok_or_else(|| {
                    AppError::environment(
                        Action::new("pick_directory"),
                        "selected path contains invalid UTF-8",
                    )
                })?
                .to_owned();

            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

/// Run the blocking reveal without occupying an async-runtime worker thread.
///
/// Generic over the closure so the test can drive the same mechanism with a
/// synthetic blocking call, but named — and its error message phrased — for
/// the one operation it serves. A second caller would inherit "Reveal task
/// failed", so give this a parameterised message before adding one.
///
/// `tauri-plugin-opener`'s Linux reveal is blocking: it opens a zbus session
/// connection, which calls `block_on` internally. Invoked directly from an
/// `async` command it runs on a Tokio worker and panics with "Cannot start a
/// runtime from within a runtime" — and a panicking command never sends an IPC
/// response, so the caller's promise hangs forever instead of rejecting. That
/// is why the button appeared to do nothing at all (#693).
///
/// Upstream has the same defect in the plugin's own command
/// (tauri-apps/plugins-workspace#3552); its fix PR #3565 was still unmerged as
/// of 2026-09-05. We pin 2.5.3, and the newest release then published (2.5.5)
/// also predates the fix — so bumping the dependency does not remove the need
/// for this wrapper.
async fn reveal_off_runtime<F, R>(f: F) -> Result<R, AppError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    // `internal`, not `environment`: a `JoinError` here means the blocking
    // closure PANICKED, which is a bug in this process rather than a fact
    // about the machine it runs on. ERROR is the right triage signal for it.
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::internal(Action::new("reveal"), format!("reveal task failed: {e}")))
}

/// Reveal a file or directory in the system file manager.
///
/// Uses `tauri-plugin-opener` to invoke the OS-native "reveal in folder"
/// action for the given path. No `opener:` capability is involved: Tauri
/// capabilities gate JS-to-command IPC, and this calls the plugin's Rust
/// function directly.
///
/// # Arguments
///
/// * `path` - Absolute path to the file or directory to reveal.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if the path is invalid or the OS
/// reveal action fails.
#[tauri::command]
pub async fn reveal_in_folder(path: String) -> Result<(), AppError> {
    if path.is_empty() {
        return Err(AppError::invalid_input(
            Action::new("reveal"),
            "path",
            "output file path is empty",
        ));
    }

    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        return Err(AppError::environment(
            Action::with_subject("reveal", Subject::Path(&path)),
            "file not found",
        ));
    }

    // No "attempting" record. It was `info!` before; demoting it to `debug!`
    // would have been a deletion in disguise, since `LOG_LEVEL` (lib.rs) is
    // Info in every build — so the honest version is to remove it. Nothing is
    // lost: the failure record below carries the same `path=`, and an
    // always-on attempt line is not an attested convention (#695). Raise the
    // module with `.level_for(...)` when tracing a specific reveal.

    reveal_off_runtime(move || tauri_plugin_opener::reveal_item_in_dir(&path_buf))
        .await?
        .map_err(|e| AppError::environment(Action::with_subject("reveal", Subject::Path(&path)), e))
}

#[cfg(test)]
// float_cmp: the resolver propagates the owner's constants unchanged, so exact
// equality is the oracle; an epsilon would accept a drifted value.
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::{resolve_effective_normalize, reveal_off_runtime};
    use crate::error::AppError;
    use rdlp_types::{LoudnormPreset, PostProcess};

    /// `None` inherits the base config's preset; `Some` overlays it. The
    /// payload's targets follow whichever preset won, which is what makes
    /// the GUI's I/TP/LRA placeholders correct under a non-Streaming preset.
    #[test]
    fn effective_normalize_overlays_the_draft_preset_on_the_base() {
        let base = PostProcess {
            loudnorm_preset: Some(LoudnormPreset::Broadcast),
            ..PostProcess::default()
        };

        let inherited = resolve_effective_normalize(&base, None);
        assert_eq!(inherited.preset, LoudnormPreset::Broadcast);
        assert_eq!(
            inherited.target_lra,
            LoudnormPreset::Broadcast.targets().range_lu
        );

        let overlaid = resolve_effective_normalize(&base, Some(LoudnormPreset::Loud));
        assert_eq!(overlaid.preset, LoudnormPreset::Loud);
        assert_eq!(
            overlaid.target_i,
            LoudnormPreset::Loud.targets().integrated_lufs
        );

        // An unset base falls to the type's default, like the engine does.
        let unset = resolve_effective_normalize(&PostProcess::default(), None);
        assert_eq!(unset.preset, LoudnormPreset::default());
    }

    /// A blocking closure that itself starts a runtime must survive.
    ///
    /// This is the shape of `zbus::blocking::Connection::session()`, which is
    /// what the opener plugin calls on Linux: `Runtime::block_on` panics when
    /// it runs on a thread that is already driving async tasks. Running the
    /// closure on the blocking pool is what makes it legal — so this test
    /// fails (by panic) against a helper that simply calls `f()` inline, which
    /// is exactly what the command used to do.
    #[tokio::test]
    async fn runs_a_blocking_call_that_starts_its_own_runtime() {
        let out = reveal_off_runtime(|| {
            tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(async { 7 })
        })
        .await;
        assert_eq!(out.expect("must not panic"), 7);
    }

    /// An empty path is rejected at the boundary, before any OS call.
    #[tokio::test]
    async fn empty_path_is_rejected() {
        let err = super::reveal_in_folder(String::new())
            .await
            .expect_err("empty path must be rejected");
        assert!(
            matches!(err, AppError::InvalidInput { ref field, .. } if field == "path"),
            "got: {err:?}"
        );
    }

    /// A path that does not exist fails with a message naming it, rather than
    /// reaching the file manager.
    ///
    /// Both guards return before the reveal, which is what keeps this module's
    /// tests hermetic: a positive case would pop a real file-manager window, so
    /// the OS call itself is verified manually (and by `reveal_off_runtime`'s test for
    /// the part that actually broke).
    #[tokio::test]
    async fn missing_path_reports_the_path() {
        testing_logger::setup();
        let err = super::reveal_in_folder("/nonexistent/rdlp-test-reveal.mkv".to_owned())
            .await
            .expect_err("missing path must fail");
        let msg = format!("{err}");
        assert!(msg.contains("file not found"), "got: {msg}");

        // End-to-end: this branch must ROUTE THROUGH `AppError::environment`,
        // not just return a similar message. Without this the test passes
        // against a hand-rolled `AppError` that logs nothing. WARN, not
        // ERROR: a path that is gone is the environment, not a bug in the
        // app, and ERROR is a triage signal. Nothing lands at ERROR at all.
        // The path lives in the structured log record, not in the Display.
        testing_logger::validate(|captured| {
            assert_eq!(
                captured
                    .iter()
                    .filter(|l| l.level == log::Level::Error)
                    .count(),
                0,
                "an environmental failure must not reach ERROR"
            );
            let errs: Vec<_> = captured
                .iter()
                .filter(|l| l.level == log::Level::Warn)
                .collect();
            assert_eq!(errs.len(), 1, "the branch logs exactly once");
            let body = errs.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("outcome=failed"), "got: {body}");
            assert!(
                body.contains("path=/nonexistent/rdlp-test-reveal.mkv"),
                "names the target: {body}"
            );
        });
    }
}
