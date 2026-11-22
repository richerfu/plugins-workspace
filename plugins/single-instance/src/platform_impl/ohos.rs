// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::Mutex;

use crate::SingleInstanceCallback;
use openharmony_ability_plugin_deep_link::{DeepLinkBridgePlugin, DeepLinkExt};
use tauri::{
    Manager, RunEvent, Runtime,
    plugin::{self, TauriPlugin},
};

pub fn init<R: Runtime>(cb: Box<SingleInstanceCallback<R>>) -> TauriPlugin<R> {
    let cb = Mutex::new(cb);
    plugin::Builder::new("single-instance")
        .setup(|_app, _api| {
            // OHOS enforces singleton at the OS level via launchType: "singleton".
            // Ensure DeepLinkBridgePlugin is registered so take_want_parameters() works
            // even when tauri-plugin-deep-link is not installed.
            if let Ok(guard) = tauri::ohos::APP.lock() {
                if let Some(ohos_app) = guard.as_ref() {
                    match ohos_app.registered_plugin::<DeepLinkBridgePlugin>() {
                        Ok(Some(_)) => {}
                        Ok(None) => {
                            if let Err(e) = ohos_app.register_plugin(DeepLinkBridgePlugin) {
                                tracing::warn!(
                                    "[single-instance] failed to register DeepLinkBridgePlugin: {e}"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[single-instance] failed to inspect DeepLinkBridgePlugin registration: {e}"
                            );
                        }
                    }
                }
            }
            Ok(())
        })
        .on_event(move |app, event| {
            // First launch does NOT trigger this handler:
            // tao emits Event::Opened only on onNewWant (re-launch),
            // not on initial onCreate.
            if let RunEvent::Opened { urls } = event {
                let params_json = if let Ok(guard) = tauri::ohos::APP.lock() {
                    if let Some(ohos_app) = guard.as_ref() {
                        match ohos_app.deep_link() {
                            Ok(client) => client.take_want_parameters(),
                            Err(_) => String::new(),
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let uri = urls.first().map(|u| u.to_string()).unwrap_or_default();
                let args: Vec<String> = [uri, params_json]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                match cb.lock() {
                    Ok(mut cb) => cb(app.app_handle(), args, String::new()),
                    Err(e) => tracing::error!("single-instance callback mutex poisoned: {e}"),
                }
            }
        })
        .build()
}
