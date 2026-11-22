// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony dialog backend built directly on `openharmony-ability-plugin-files`
//! (system file/folder pickers) and `openharmony-ability-plugin-window` (ArkUI
//! modal prompts).

use std::path::{Path, PathBuf};

use openharmony_ability_plugin_files::{
    FileDialogFilter, FileDialogOptions, FilesBridgePlugin, FilesExt, dialog_type,
};
use openharmony_ability_plugin_window::{
    WindowBridgePlugin, WindowClient, WindowPromptRequest,
};
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{
    CANCEL, FileDialogBuilder, FilePath, Filter, MessageDialogBuilder, NO, OK, YES, models::*,
};

fn with_ohos_app<R>(
    f: impl FnOnce(&tauri::ohos::openharmony_ability::OpenHarmonyApp) -> R,
) -> Option<R> {
    let guard = tauri::ohos::APP.lock().ok()?;
    guard.as_ref().map(f)
}

fn clone_ohos_app() -> Option<tauri::ohos::openharmony_ability::OpenHarmonyApp> {
    with_ohos_app(Clone::clone)
}

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Dialog<R>> {
    let _ = with_ohos_app(|ohos_app| {
        match ohos_app.registered_plugin::<FilesBridgePlugin>() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(e) = ohos_app.register_plugin(FilesBridgePlugin) {
                    log::error!("[dialog] failed to register FilesBridgePlugin: {e}");
                }
            }
            Err(e) => {
                log::error!("[dialog] failed to check FilesBridgePlugin registration: {e}");
            }
        }

        match ohos_app.registered_plugin::<WindowBridgePlugin>() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(e) = ohos_app.register_plugin(WindowBridgePlugin) {
                    log::error!("[dialog] failed to register WindowBridgePlugin: {e}");
                }
            }
            Err(e) => {
                log::error!("[dialog] failed to check WindowBridgePlugin registration: {e}");
            }
        }
    });

    Ok(Dialog(app.clone()))
}

/// Access to the dialog APIs on OpenHarmony.
#[derive(Debug)]
pub struct Dialog<R: Runtime>(AppHandle<R>);

impl<R: Runtime> Clone for Dialog<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R: Runtime> Dialog<R> {
    pub(crate) fn app_handle(&self) -> &AppHandle<R> {
        &self.0
    }
}

fn uri_to_file_path(raw: String) -> FilePath {
    if let Ok(url) = url::Url::parse(&raw) {
        FilePath::Url(url)
    } else {
        FilePath::Path(PathBuf::from(raw))
    }
}

fn default_location_uri(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://") {
        return Some(trimmed.to_string());
    }
    url::Url::from_file_path(path).ok().map(|u| u.to_string())
}

fn map_filters(filters: &[Filter]) -> Vec<FileDialogFilter> {
    let mut out = Vec::new();
    for filter in filters {
        let mut patterns = Vec::new();
        for ext in &filter.extensions {
            let trimmed = ext.trim();
            if trimmed.is_empty() || trimmed.contains('/') {
                continue;
            }
            if trimmed == "*" || trimmed == "*.*" {
                patterns.push("*".to_string());
            } else {
                patterns.push(trimmed.trim_start_matches('.').to_string());
            }
        }
        if patterns.is_empty() {
            continue;
        }
        let pattern = if patterns.iter().any(|p| p == "*") {
            "*".to_string()
        } else {
            patterns.join(";")
        };
        let mut entry = FileDialogFilter::new().pattern(pattern);
        let name = filter.name.trim();
        if !name.is_empty() {
            entry = entry.name(name);
        }
        out.push(entry);
    }
    out
}

fn build_file_dialog_options<R: Runtime>(
    dialog: &FileDialogBuilder<R>,
    kind: &'static str,
    allow_many: bool,
    include_filters: bool,
    include_suggested_name: bool,
) -> FileDialogOptions {
    let mut options = FileDialogOptions::new(kind).allow_many(allow_many);

    if let Some(ref dir) = dialog.starting_directory {
        if let Some(uri) = default_location_uri(dir) {
            options = options.default_location(uri);
        }
    }

    if include_suggested_name {
        if let Some(ref name) = dialog.file_name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                options = options.suggested_name(trimmed);
            }
        }
    }

    if include_filters {
        let filters = map_filters(&dialog.filters);
        if !filters.is_empty() {
            options = options.filters(filters);
        }
    }

    // Window-owned DocumentViewPicker requires API 13+ on OpenHarmony.
    if let Some(window_id) = dialog.parent_window_id {
        if window_id >= 0 && tauri::ohos::openharmony_ability::version::sdk_api_version() >= 13 {
            options = options.parent_window_id(window_id);
        }
    }

    options
}

async fn run_file_dialog(options: FileDialogOptions) -> Vec<FilePath> {
    let Some(ohos_app) = clone_ohos_app() else {
        log::error!("[dialog] OHOS APP not initialized");
        return Vec::new();
    };
    match ohos_app.show_file_dialog(options).await {
        Ok(response) => response.files.into_iter().map(uri_to_file_path).collect(),
        Err(e) => {
            log::warn!("[dialog] file dialog failed or cancelled: {e}");
            Vec::new()
        }
    }
}

pub fn pick_file<R: Runtime, F: FnOnce(Option<FilePath>) + Send + 'static>(
    dialog: FileDialogBuilder<R>,
    f: F,
) {
    let options = build_file_dialog_options(&dialog, dialog_type::OPEN_FILE, false, true, false);
    tauri::async_runtime::spawn(async move {
        let mut files = run_file_dialog(options).await;
        f(files.drain(..).next());
    });
}

pub fn pick_files<R: Runtime, F: FnOnce(Option<Vec<FilePath>>) + Send + 'static>(
    dialog: FileDialogBuilder<R>,
    f: F,
) {
    let options = build_file_dialog_options(&dialog, dialog_type::OPEN_FILE, true, true, false);
    tauri::async_runtime::spawn(async move {
        let files = run_file_dialog(options).await;
        if files.is_empty() {
            f(None);
        } else {
            f(Some(files));
        }
    });
}

pub fn pick_folder<R: Runtime, F: FnOnce(Option<FilePath>) + Send + 'static>(
    dialog: FileDialogBuilder<R>,
    f: F,
) {
    let options = build_file_dialog_options(&dialog, dialog_type::OPEN_FOLDER, false, false, false);
    tauri::async_runtime::spawn(async move {
        let mut files = run_file_dialog(options).await;
        f(files.drain(..).next());
    });
}

pub fn pick_folders<R: Runtime, F: FnOnce(Option<Vec<FilePath>>) + Send + 'static>(
    dialog: FileDialogBuilder<R>,
    f: F,
) {
    let options = build_file_dialog_options(&dialog, dialog_type::OPEN_FOLDER, true, false, false);
    tauri::async_runtime::spawn(async move {
        let files = run_file_dialog(options).await;
        if files.is_empty() {
            f(None);
        } else {
            f(Some(files));
        }
    });
}

pub fn save_file<R: Runtime, F: FnOnce(Option<FilePath>) + Send + 'static>(
    dialog: FileDialogBuilder<R>,
    f: F,
) {
    let options = build_file_dialog_options(&dialog, dialog_type::SAVE_FILE, false, true, true);
    tauri::async_runtime::spawn(async move {
        let mut files = run_file_dialog(options).await;
        f(files.drain(..).next());
    });
}

struct MappedPrompt {
    request: WindowPromptRequest,
    results: Vec<MessageDialogResult>,
    fallback: MessageDialogResult,
}

fn non_empty_label(label: &str, fallback: &str) -> String {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn build_prompt_request(
    window_id: i64,
    title: &str,
    message: &str,
    kind: MessageDialogKind,
    buttons: &MessageDialogButtons,
) -> MappedPrompt {
    let prefix = match kind {
        MessageDialogKind::Info => "",
        MessageDialogKind::Warning => "⚠️ ",
        MessageDialogKind::Error => "❌ ",
    };

    let (prompt_message, prompt_detail) = if !title.trim().is_empty() {
        (
            format!("{prefix}{title}"),
            if message.is_empty() {
                None
            } else {
                Some(message.to_string())
            },
        )
    } else {
        (format!("{prefix}{message}"), None)
    };

    let (button_labels, results, cancel_index, fallback) = match buttons {
        MessageDialogButtons::Ok => (
            vec![OK.to_string()],
            vec![MessageDialogResult::Ok],
            Some(0),
            MessageDialogResult::Ok,
        ),
        MessageDialogButtons::OkCancel => (
            vec![OK.to_string(), CANCEL.to_string()],
            vec![MessageDialogResult::Ok, MessageDialogResult::Cancel],
            Some(1),
            MessageDialogResult::Cancel,
        ),
        MessageDialogButtons::YesNo => (
            vec![YES.to_string(), NO.to_string()],
            vec![MessageDialogResult::Yes, MessageDialogResult::No],
            Some(1),
            MessageDialogResult::No,
        ),
        MessageDialogButtons::YesNoCancel => (
            vec![YES.to_string(), NO.to_string(), CANCEL.to_string()],
            vec![
                MessageDialogResult::Yes,
                MessageDialogResult::No,
                MessageDialogResult::Cancel,
            ],
            Some(2),
            MessageDialogResult::Cancel,
        ),
        MessageDialogButtons::OkCustom(ok) => (
            vec![non_empty_label(ok, OK)],
            vec![MessageDialogResult::Custom(ok.clone())],
            Some(0),
            MessageDialogResult::Custom(ok.clone()),
        ),
        MessageDialogButtons::OkCancelCustom(ok, cancel) => (
            vec![non_empty_label(ok, OK), non_empty_label(cancel, CANCEL)],
            vec![
                MessageDialogResult::Custom(ok.clone()),
                MessageDialogResult::Custom(cancel.clone()),
            ],
            Some(1),
            MessageDialogResult::Custom(cancel.clone()),
        ),
        MessageDialogButtons::YesNoCancelCustom(yes, no, cancel) => (
            vec![
                non_empty_label(yes, YES),
                non_empty_label(no, NO),
                non_empty_label(cancel, CANCEL),
            ],
            vec![
                MessageDialogResult::Custom(yes.clone()),
                MessageDialogResult::Custom(no.clone()),
                MessageDialogResult::Custom(cancel.clone()),
            ],
            Some(2),
            MessageDialogResult::Custom(cancel.clone()),
        ),
    };

    MappedPrompt {
        request: WindowPromptRequest {
            window_id: window_id.max(0),
            message: prompt_message,
            detail: prompt_detail,
            buttons: button_labels,
            cancel_index,
            cancel_on_touch_outside: false,
        },
        results,
        fallback,
    }
}

/// Shows a native modal message dialog via `openharmony-ability-plugin-window`.
pub fn show_message_dialog<R: Runtime, F: FnOnce(MessageDialogResult) + Send + 'static>(
    dialog: MessageDialogBuilder<R>,
    f: F,
) {
    let window_id = dialog.parent_window_id.unwrap_or(0);
    let mapped = build_prompt_request(
        window_id,
        &dialog.title,
        &dialog.message,
        dialog.kind,
        &dialog.buttons,
    );

    tauri::async_runtime::spawn(async move {
        let result = match clone_ohos_app() {
            Some(ohos_app) => match WindowClient::new(&ohos_app) {
                Ok(client) => match client.show_prompt(mapped.request).await {
                    Ok(index) => mapped
                        .results
                        .get(index as usize)
                        .cloned()
                        .unwrap_or(mapped.fallback),
                    Err(e) => {
                        log::warn!("[dialog] message prompt failed or cancelled: {e}");
                        mapped.fallback
                    }
                },
                Err(e) => {
                    log::error!("[dialog] failed to create WindowClient: {e}");
                    mapped.fallback
                }
            },
            None => {
                log::error!("[dialog] OHOS APP not initialized");
                mapped.fallback
            }
        };
        f(result);
    });
}
