// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Platform-dispatched operations for the window-state plugin.
//!
//! Each function comes in two fn-level-`cfg` variants — OHOS and non-OHOS —
//! so the call sites in `lib.rs` stay free of `cfg` attributes. The OHOS
//! variants carry the OHOS lock-discipline workarounds; the non-OHOS variants
//! keep the upstream desktop behavior (including the macOS `restorable_size`
//! overlay title bar fix) verbatim.

use std::fs::create_dir_all;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, Runtime, Window};

use crate::{MonitorExt, PluginState, Result, StateFlags, WindowState, WindowStateCache};

#[cfg(target_env = "ohos")]
use std::collections::HashMap;

#[cfg(target_env = "ohos")]
use crate::WindowExt;

#[cfg(not(target_env = "ohos"))]
use crate::{AppHandleExt, WindowExtInternal, restorable_size};

#[cfg(target_env = "ohos")]
pub(crate) fn save_window_state<R: Runtime>(app: &AppHandle<R>, flags: StateFlags) -> Result<()> {
    let _ = &flags;
    let app_dir = app.path().app_config_dir()?;
    let plugin_state = app.state::<PluginState>();
    let state_path = app_dir.join(&plugin_state.filename);
    let windows = app.webview_windows();
    let cache = app.state::<WindowStateCache>();

    // OHOS: collect fresh geometry BEFORE taking the cache lock to avoid
    // deadlock between worker thread (cache lock -> main-thread dispatcher recv)
    // and main thread (Resized/Moved -> cache lock).
    let fresh_geometry: Vec<(String, Option<(i32, i32)>, Option<(u32, u32)>)> = {
        let tracked: Vec<String> = cache.0.lock().unwrap().keys().cloned().collect();
        let mut fresh = Vec::with_capacity(tracked.len());
        for (window_label, window) in &windows {
            let label: String = plugin_state
                .map_label
                .as_ref()
                .map(|map| map(window_label).to_string())
                .unwrap_or_else(|| window_label.clone());
            if !tracked.contains(&label) {
                continue;
            }
            let position = window.outer_position().ok().map(|pos| (pos.x, pos.y));
            let size = window
                .inner_size()
                .ok()
                .and_then(|size| {
                    (size.width > 0 && size.height > 0).then_some((size.width, size.height))
                });
            fresh.push((label, position, size));
        }
        fresh
    };

    let mut state = cache.0.lock().unwrap();
    for (label, s) in state.iter_mut() {
        if let Some((_, position, size)) = fresh_geometry.iter().find(|(l, _, _)| l == label) {
            if let Some((x, y)) = position {
                s.x = *x;
                s.y = *y;
            }
            if let Some((width, height)) = size {
                s.width = *width;
                s.height = *height;
            }
        }
    }

    // Serialize under the lock, then drop the lock before disk I/O.
    {
        let serialized = serde_json::to_vec_pretty(&*state)?;
        drop(state);
        create_dir_all(app_dir)?;
        std::fs::write(state_path, serialized)?;
    }

    Ok(())
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn save_window_state<R: Runtime>(app: &AppHandle<R>, flags: StateFlags) -> Result<()> {
    let app_dir = app.path().app_config_dir()?;
    let plugin_state = app.state::<PluginState>();
    let state_path = app_dir.join(&plugin_state.filename);
    let windows = app.webview_windows();
    let cache = app.state::<WindowStateCache>();
    let mut state = cache.0.lock().unwrap();

    for (label, s) in state.iter_mut() {
        let window = if let Some(map) = &plugin_state.map_label {
            windows
                .iter()
                .find_map(|(l, window)| (map(l) == label).then_some(window))
        } else {
            windows.get(label)
        };

        if let Some(window) = window {
            window.update_state(s, flags)?;
        }
    }

    create_dir_all(app_dir)?;
    std::fs::write(state_path, serde_json::to_vec_pretty(&*state)?)?;

    Ok(())
}

#[cfg(target_env = "ohos")]
pub(crate) fn restore_state<R: Runtime>(
    window: &Window<R>,
    plugin_state: &PluginState,
    label: &str,
    flags: StateFlags,
) -> tauri::Result<()> {
    let mut file_cache: Option<HashMap<String, WindowState>> = None;
    if let Ok(app_dir) = window.app_handle().path().app_config_dir() {
        let state_path = app_dir.join(&plugin_state.filename);
        if state_path.exists() {
            if let Ok(data) = std::fs::read(&state_path) {
                file_cache =
                    serde_json::from_slice::<HashMap<String, WindowState>>(&data).ok();
            }
        }
    }

    let cache = window.state::<WindowStateCache>();
    let saved = {
        let mut c = cache.0.lock().unwrap();
        if let Some(saved) = file_cache.as_ref().and_then(|fc| fc.get(label)) {
            c.insert(label.to_string(), saved.clone());
        }
        let saved = c
            .get(label)
            .filter(|state| state != &&WindowState::default())
            .cloned();
        if saved.is_none() {
            c.insert(label.to_string(), WindowState::default());
        }
        saved
    };

    let mut should_show = true;

    if let Some(state) = saved {
        if flags.contains(StateFlags::DECORATIONS) {
            #[cfg(desktop)]
            window.set_decorations(state.decorated)?;
        }

        if flags.contains(StateFlags::POSITION) {
            let position = if state.maximized {
                PhysicalPosition::new(state.prev_x, state.prev_y)
            } else {
                PhysicalPosition::new(state.x, state.y)
            };
            let size = (state.width, state.height).into();
            for m in window.available_monitors()? {
                if m.intersects(position, size) {
                    window.set_position(position)?;
                    break;
                }
            }
        }

        if flags.contains(StateFlags::SIZE) {
            window.set_size(PhysicalSize {
                width: state.width,
                height: state.height,
            })?;
        }

        if flags.contains(StateFlags::MAXIMIZED) && state.maximized {
            #[cfg(desktop)]
            window.maximize()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            #[cfg(desktop)]
            window.set_fullscreen(state.fullscreen)?;
        }

        should_show = state.visible;
    }

    if flags.contains(StateFlags::VISIBLE) && should_show {
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn restore_state<R: Runtime>(
    window: &Window<R>,
    _plugin_state: &PluginState,
    label: &str,
    flags: StateFlags,
) -> tauri::Result<()> {
    let cache = window.state::<WindowStateCache>();
    let mut c = cache.0.lock().unwrap();

    let mut should_show = true;

    if let Some(state) = c
        .get(label)
        .filter(|state| state != &&WindowState::default())
    {
        if flags.contains(StateFlags::DECORATIONS) {
            window.set_decorations(state.decorated)?;
        }

        if flags.contains(StateFlags::POSITION) {
            // a maximized window's x and y are the monitor's corner, so it is
            // placed where it was before it was maximized
            let position = if state.maximized {
                PhysicalPosition::new(state.prev_x, state.prev_y)
            } else {
                PhysicalPosition::new(state.x, state.y)
            };
            let size = (state.width, state.height).into();
            // restore position to saved value if saved monitor exists
            // otherwise, let the OS decide where to place the window
            for m in window.available_monitors()? {
                if m.intersects(position, size) {
                    window.set_position(position)?;
                    break;
                }
            }
        }

        if flags.contains(StateFlags::SIZE) {
            window.set_size(PhysicalSize {
                width: state.width,
                height: state.height,
            })?;
        }

        if flags.contains(StateFlags::MAXIMIZED) && state.maximized {
            window.maximize()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            window.set_fullscreen(state.fullscreen)?;
        }

        should_show = state.visible;
    } else {
        let mut metadata = WindowState::default();

        if flags.contains(StateFlags::SIZE) {
            let size = restorable_size(window)?;
            metadata.width = size.width;
            metadata.height = size.height;
        }

        if flags.contains(StateFlags::POSITION) {
            let pos = window.outer_position()?;
            metadata.x = pos.x;
            metadata.y = pos.y;
        }

        if flags.contains(StateFlags::MAXIMIZED) {
            metadata.maximized = window.is_maximized()?;
        }

        if flags.contains(StateFlags::VISIBLE) {
            metadata.visible = window.is_visible()?;
        }

        if flags.contains(StateFlags::DECORATIONS) {
            metadata.decorated = window.is_decorated()?;
        }

        if flags.contains(StateFlags::FULLSCREEN) {
            metadata.fullscreen = window.is_fullscreen()?;
        }

        c.insert(label.into(), metadata);
    }

    if flags.contains(StateFlags::VISIBLE) && should_show {
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

#[cfg(target_env = "ohos")]
pub(crate) fn on_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, state_flags: StateFlags) {
    if let RunEvent::Ready = &event {
        let windows_to_restore: Vec<_> = {
            let plugin_state = app.state::<PluginState>();
            app.webview_windows()
                .into_iter()
                .filter(|(_, window)| {
                    let label = plugin_state
                        .map_label
                        .as_ref()
                        .map(|map| map(window.label()))
                        .unwrap_or_else(|| window.label());
                    if plugin_state.denylist.contains(label) {
                        return false;
                    }
                    if let Some(filter_callback) = &plugin_state.filter_callback {
                        if !filter_callback(label) {
                            return false;
                        }
                    }
                    !plugin_state.skip_initial_state.contains(label)
                })
                .map(|(_, w)| w)
                .collect()
        };
        for window in windows_to_restore {
            let _ = window.restore_state(state_flags);
        }
    }
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn on_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent, state_flags: StateFlags) {
    if let RunEvent::Exit = event {
        let _ = app.save_window_state(state_flags);
    }
}

#[cfg(target_env = "ohos")]
pub(crate) fn update_state_is_maximized<R: Runtime>(
    _window: &Window<R>,
    _flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(false)
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn update_state_is_maximized<R: Runtime>(
    window: &Window<R>,
    flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(
        flags.intersects(StateFlags::MAXIMIZED | StateFlags::POSITION | StateFlags::SIZE)
            && window.is_maximized()?,
    )
}

#[cfg(target_env = "ohos")]
pub(crate) fn update_state_is_minimized<R: Runtime>(
    _window: &Window<R>,
    _flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(false)
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn update_state_is_minimized<R: Runtime>(
    window: &Window<R>,
    flags: StateFlags,
) -> tauri::Result<bool> {
    Ok(flags.intersects(StateFlags::POSITION | StateFlags::SIZE) && window.is_minimized()?)
}

#[cfg(target_env = "ohos")]
pub(crate) fn moved_is_minimized<R: Runtime>(_window: &Window<R>) -> bool {
    false
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn moved_is_minimized<R: Runtime>(window: &Window<R>) -> bool {
    window.is_minimized().unwrap_or_default()
}

#[cfg(target_env = "ohos")]
pub(crate) fn resized_should_save<R: Runtime>(_window: &Window<R>) -> bool {
    true
}

#[cfg(not(target_env = "ohos"))]
pub(crate) fn resized_should_save<R: Runtime>(window: &Window<R>) -> bool {
    // TODO: Remove once https://github.com/tauri-apps/tauri/issues/5812 is resolved.
    let is_maximized = if cfg!(target_os = "macos")
        && (!window.is_decorated().unwrap_or_default()
            || !window.is_resizable().unwrap_or_default())
    {
        false
    } else {
        window.is_maximized().unwrap_or_default()
    };
    !window.is_minimized().unwrap_or_default() && !is_maximized
}
