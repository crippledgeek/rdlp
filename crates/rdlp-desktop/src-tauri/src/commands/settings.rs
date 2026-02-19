//! Application settings IPC commands.
//!
//! Provides commands to read, update, and interact with persistent
//! application settings such as the download directory. The
//! [`pick_directory`] command uses the native OS folder picker via
//! `tauri-plugin-dialog`.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::state::{AppSettings, AppState};

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
/// Returns [`AppError::Internal`] if the settings mutex is poisoned.
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|e| AppError::Internal {
            message: format!("Settings lock poisoned: {e}"),
        })?
        .clone();

    Ok(settings)
}

/// Update application settings with new values.
///
/// Locks the shared settings mutex and replaces the current
/// [`AppSettings`] with the provided values.
///
/// # Arguments
///
/// * `settings` - New settings from the frontend.
/// * `state` - Managed application state containing the settings.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if the settings mutex is poisoned.
#[tauri::command]
pub async fn update_settings(
    settings: AppSettings,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let mut current = state.settings.lock().map_err(|e| AppError::Internal {
        message: format!("Settings lock poisoned: {e}"),
    })?;

    *current = settings;
    current.save().ok();

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

    let folder = rx.await.map_err(|_| AppError::Internal {
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
