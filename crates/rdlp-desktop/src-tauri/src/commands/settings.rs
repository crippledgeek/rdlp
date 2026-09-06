//! Application settings IPC commands.
//!
//! Provides commands to read, update, and interact with persistent
//! application settings such as the download directory. The
//! [`pick_directory`] command uses the native OS folder picker via
//! `tauri-plugin-dialog`, and [`reveal_in_folder`] uses
//! `tauri-plugin-opener` to show a file in the system file manager.

// `Duration::from_mins` (lint's suggested replacement) is stable since Rust
// 1.91 (`duration_constructors_lite`); the workspace MSRV is 1.88.
#![allow(clippy::duration_suboptimal_units)]

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::state::{AppSettings, AppState, SettingsValidationError};
use rdlp_types::boundary::{Action, Subject};

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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::reveal_off_runtime;
    use crate::error::AppError;

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
