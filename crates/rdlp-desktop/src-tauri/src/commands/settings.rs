//! Application settings IPC commands.
//!
//! Provides commands to read, update, and interact with persistent
//! application settings such as the download directory. The
//! [`pick_directory`] command uses the native OS folder picker via
//! `tauri-plugin-dialog`, and [`reveal_in_folder`] uses
//! `tauri-plugin-opener` to show a file in the system file manager.

use std::path::PathBuf;
use std::time::Duration;

use log::{info, warn};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;
use crate::state::{AppSettings, AppState, SettingsValidationError};

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
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
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
    settings
        .validate_security()
        .map_err(|e| AppError::InvalidInput {
            field: match &e {
                SettingsValidationError::CookiesFileTraversal => "cookies_file",
                SettingsValidationError::InvalidProxy(_) => "proxy",
            }
            .to_owned(),
            message: e.to_string(),
        })?;

    let mut current = state.settings.lock().unwrap_or_else(|e| e.into_inner());

    *current = settings;
    current.save().map_err(|e| AppError::Internal {
        message: format!("Failed to save settings: {e}"),
    })?;

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
        .map_err(|_| AppError::Internal {
            message: "Folder picker timed out after 5 minutes".to_owned(),
        })?
        .map_err(|_| AppError::Internal {
            message: "Folder picker channel closed unexpectedly".to_owned(),
        })?;

    match folder {
        Some(file_path) => {
            let path = file_path.into_path().map_err(|e| AppError::Internal {
                message: format!("Failed to convert folder path: {e}"),
            })?;

            let path_str = path
                .to_str()
                .ok_or_else(|| AppError::Internal {
                    message: "Selected path contains invalid UTF-8".to_owned(),
                })?
                .to_owned();

            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

/// Reveal a file or directory in the system file manager.
///
/// Uses `tauri-plugin-opener` to invoke the OS-native "reveal in folder"
/// action for the given path. Requires the `opener:allow-reveal-item-in-dir`
/// capability.
///
/// # Arguments
///
/// * `path` - Absolute path to the file or directory to reveal.
/// * `app` - Tauri application handle for accessing the opener plugin.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if the path is invalid or the OS
/// reveal action fails.
#[tauri::command]
pub async fn reveal_in_folder(path: String, app: AppHandle) -> Result<(), AppError> {
    if path.is_empty() {
        return Err(AppError::InvalidInput {
            field: "path".to_owned(),
            message: "Output file path is empty".to_owned(),
        });
    }

    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        warn!("reveal_in_folder: file does not exist: {}", path);
        return Err(AppError::Internal {
            message: format!("File not found: {path}"),
        });
    }

    info!("reveal_in_folder: revealing {}", path);

    app.opener()
        .reveal_item_in_dir(&path_buf)
        .map_err(|e| AppError::Internal {
            message: format!("Failed to reveal path in folder: {e}"),
        })?;

    Ok(())
}
