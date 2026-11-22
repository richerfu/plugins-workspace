// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony-specific implementation for the updater plugin.
//!
//! On OHOS, updates are hosted by AppGallery — the desktop updater API
//! (endpoints/pubkey from `tauri.conf.json`) does not apply. Instead of
//! faking the desktop commands, this module exposes two OHOS-only commands:
//! - `check_app_gallery_update` queries `updateManager.checkAppUpdate()`
//!   (pure metadata, no dialog)
//! - `show_app_gallery_update_dialog` triggers `updateManager.showUpdateDialog()`
//!   (system dialog driving the full download + install flow)

use crate::{Error, Result};
use openharmony_ability_plugin_updater::{Updater, UpdaterExt};
use serde::Serialize;
use tauri::{Manager, Runtime, Webview};

/// Result of an AppGallery update check, serialized as
/// `{ available, currentVersion, version }`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppGalleryUpdateInfo {
    available: bool,
    current_version: String,
    version: Option<String>,
}

/// Get the AppGallery updater bridge handle.
///
/// The `tauri::ohos::APP` guard is acquired and dropped inside this helper,
/// so the lock is never held across an await.
fn updater_bridge() -> Result<Updater> {
    let guard = tauri::ohos::APP
        .lock()
        .map_err(|_| Error::Network("OHOS APP mutex poisoned".into()))?;
    let app = guard
        .as_ref()
        .ok_or_else(|| Error::Network("OHOS APP not initialized".into()))?;
    app.updater()
        .map_err(|e| Error::Network(e.reason.to_string()))
}

/// Check for updates via AppGallery (OHOS only).
///
/// This is a pure query — no dialog is shown.
#[tauri::command]
pub(crate) async fn check_app_gallery_update<R: Runtime>(
    webview: Webview<R>,
) -> Result<AppGalleryUpdateInfo> {
    let updater = updater_bridge()?;

    match updater.check().await {
        Ok(None) => Ok(AppGalleryUpdateInfo {
            available: false,
            current_version: webview.package_info().version.to_string(),
            version: None,
        }),
        Ok(Some(r)) => Ok(AppGalleryUpdateInfo {
            available: true,
            current_version: r.current_version,
            version: r.version.filter(|v| !v.is_empty() && v != "unknown"),
        }),
        Err(e) => Err(Error::Network(e.reason.to_string())),
    }
}

/// Show the system AppGallery update dialog (OHOS only).
///
/// The dialog is user-driven: resolving `Ok(())` does NOT mean the update was installed.
#[tauri::command]
pub(crate) async fn show_app_gallery_update_dialog<R: Runtime>(
    _webview: Webview<R>,
) -> Result<()> {
    updater_bridge()?
        .download_and_install()
        .await
        .map_err(|e| Error::Network(e.reason.to_string()))?;

    Ok(())
}
