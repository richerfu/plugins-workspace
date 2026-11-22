// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! OpenHarmony-specific IPC commands, delegating to the shared mobile
//! implementation in [`crate::mobile`].

use tauri::{AppHandle, Runtime, State, command};

use crate::{Notification, Result};

/// Cancel pending notifications by ID, or all if none specified.
#[command]
pub(crate) async fn cancel<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    notifications: Option<Vec<i32>>,
) -> Result<()> {
    match notifications {
        Some(ids) if !ids.is_empty() => notification.cancel(ids),
        _ => notification.cancel_all(),
    }
}

/// Get pending notifications.
#[command]
pub(crate) async fn get_pending<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
) -> Result<Vec<crate::PendingNotification>> {
    notification.pending()
}

#[command]
pub(crate) async fn remove_active<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    notifications: Option<Vec<RemoveActiveId>>,
) -> Result<()> {
    match notifications {
        Some(items) if !items.is_empty() => {
            notification.remove_active(items.into_iter().map(|i| i.id).collect())
        }
        _ => notification.remove_all_active(),
    }
}

#[command]
pub(crate) async fn get_active<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
) -> Result<Vec<crate::ActiveNotification>> {
    notification.active()
}

#[command]
pub(crate) async fn register_action_types<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    types: Vec<crate::ActionType>,
) -> Result<()> {
    notification.register_action_types(types)
}

pub(crate) struct ChannelArg(crate::Channel);

impl<'de, R: Runtime> tauri::ipc::CommandArg<'de, R> for ChannelArg {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> std::result::Result<Self, tauri::ipc::InvokeError> {
        match command.message.payload() {
            tauri::ipc::InvokeBody::Json(v) => {
                let ch: crate::Channel = serde_json::from_value(v.clone()).map_err(|e| {
                    tauri::ipc::InvokeError::from_error(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        e.to_string(),
                    ))
                })?;
                Ok(ChannelArg(ch))
            }
            tauri::ipc::InvokeBody::Raw(_) => Err(tauri::ipc::InvokeError::from_error(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "create_channel expects JSON payload",
                ),
            )),
        }
    }
}

#[command]
pub(crate) async fn create_channel<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    _channel: ChannelArg,
) -> Result<()> {
    notification.create_channel(_channel.0)
}

#[command]
pub(crate) async fn delete_channel<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
    id: String,
) -> Result<()> {
    notification.delete_channel(id)
}

#[command]
pub(crate) async fn list_channels<R: Runtime>(
    _app: AppHandle<R>,
    notification: State<'_, Notification<R>>,
) -> Result<serde_json::Value> {
    notification.list_channels_raw()
}

#[derive(serde::Deserialize)]
pub(crate) struct RemoveActiveId {
    pub id: i32,
}
