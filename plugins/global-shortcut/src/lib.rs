// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Register global shortcuts.
//!
//! - Supported platforms: Windows, Linux, macOS and OpenHarmony.
//! - On macOS, registering and unregistering shortcuts must happen on the main thread, so every
//!   [`GlobalShortcut`] operation dispatches to it and blocks until it completes.

#![doc(
    html_logo_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png",
    html_favicon_url = "https://github.com/tauri-apps/tauri/raw/dev/app-icon.png"
)]
#![cfg(not(any(target_os = "android", target_os = "ios")))]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tauri::{
    AppHandle, Manager, Runtime, State,
    ipc::Channel,
    plugin::{Builder as PluginBuilder, TauriPlugin},
};

mod error;

pub use error::Error;
type Result<T> = std::result::Result<T, Error>;

type HotKeyId = u32;

// ─── Platform-specific type aliases ─────────────────────────────────────────

#[cfg(not(target_env = "ohos"))]
pub use global_hotkey::{
    GlobalHotKeyEvent as ShortcutEvent, HotKeyState as ShortcutState,
    hotkey::{Code, HotKey as Shortcut, Modifiers},
};

#[cfg(not(target_env = "ohos"))]
use global_hotkey::GlobalHotKeyEvent;

#[cfg(not(target_env = "ohos"))]
use std::str::FromStr;

// ─── OpenHarmony backend ────────────────────────────────────────────────────

#[cfg(target_env = "ohos")]
mod ohos;

#[cfg(target_env = "ohos")]
pub use ohos::{Code, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

#[cfg(target_env = "ohos")]
use ohos::{lock_shortcuts, ohos_setup};

// ─── Common types ───────────────────────────────────────────────────────────

type HandlerFn<R> = Box<dyn Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>;

/// Internal wrapper around a [`Shortcut`] that lets the shortcut-accepting APIs on
/// [`GlobalShortcut`] and [`Builder`] be generic over an already-parsed [`Shortcut`] or a
/// string accelerator such as `"CmdOrControl+Q"`.
pub struct ShortcutWrapper(Shortcut);

impl From<Shortcut> for ShortcutWrapper {
    fn from(value: Shortcut) -> Self {
        Self(value)
    }
}

#[cfg(not(target_env = "ohos"))]
impl TryFrom<&str> for ShortcutWrapper {
    type Error = global_hotkey::hotkey::HotKeyParseError;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Shortcut::from_str(value).map(ShortcutWrapper)
    }
}

#[cfg(target_env = "ohos")]
impl TryFrom<&str> for ShortcutWrapper {
    type Error = ohos::HotKeyParseError;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        value
            .parse::<Shortcut>()
            .map(ShortcutWrapper)
            .map_err(ohos::HotKeyParseError)
    }
}

struct RegisteredShortcut<R: Runtime> {
    shortcut: Shortcut,
    handler: Option<Arc<HandlerFn<R>>>,
    /// OHOS: monotonic registration stamp.
    /// The entry is inserted synchronously and removed from a worker thread
    /// when ArkTS rejects the request; the stamp makes that removal
    /// conditional so a same-id re-registration that already replaced the
    /// entry is not undone by the stale worker of the earlier attempt.
    #[cfg(target_env = "ohos")]
    stamp: u32,
}

/// Runs the handlers registered for the shortcut behind `event`.
///
/// The shortcut and its handler are copied out of the map and the lock is released before any
/// user code runs, so a handler can call back into the plugin (or pump messages that dispatch
/// another shortcut event) without deadlocking on `shortcuts`.
#[cfg(not(target_env = "ohos"))]
fn dispatch_shortcut_event<R: Runtime>(
    shortcuts: &Mutex<HashMap<HotKeyId, RegisteredShortcut<R>>>,
    app: &AppHandle<R>,
    global_handler: Option<&HandlerFn<R>>,
    event: GlobalHotKeyEvent,
) {
    let registered = shortcuts
        .lock()
        .unwrap()
        .get(&event.id)
        .map(|s| (s.shortcut, s.handler.clone()));
    if let Some((shortcut, shortcut_handler)) = registered {
        if let Some(handler) = &shortcut_handler {
            handler(app, &shortcut, event);
        }
        if let Some(handler) = global_handler {
            handler(app, &shortcut, event);
        }
    }
}

#[cfg(not(target_env = "ohos"))]
struct GlobalHotKeyManager(global_hotkey::GlobalHotKeyManager);

/// SAFETY: we ensure it is run on main thread only
#[cfg(not(target_env = "ohos"))]
unsafe impl Send for GlobalHotKeyManager {}
/// SAFETY: we ensure it is run on main thread only
#[cfg(not(target_env = "ohos"))]
unsafe impl Sync for GlobalHotKeyManager {}

/// The global shortcut APIs, accessible through [`GlobalShortcutExt::global_shortcut`].
///
/// Registering, unregistering and checking a shortcut all dispatch the underlying
/// `global_hotkey` call to the app's main thread and block until it finishes, since macOS
/// requires the hotkey manager to run on the main thread.
pub struct GlobalShortcut<R: Runtime> {
    #[allow(dead_code)]
    #[cfg(not(target_env = "ohos"))]
    app: AppHandle<R>,
    #[cfg(not(target_env = "ohos"))]
    manager: Arc<GlobalHotKeyManager>,
    #[cfg(target_env = "ohos")]
    client: Option<openharmony_ability_plugin_global_shortcut::GlobalShortcutClient>,
    shortcuts: Arc<Mutex<HashMap<HotKeyId, RegisteredShortcut<R>>>>,
}

#[cfg(not(target_env = "ohos"))]
macro_rules! run_main_thread {
    ($handle:expr, $manager:expr, |$m:ident| $ex:expr) => {{
        let (tx, rx) = std::sync::mpsc::channel();
        let manager = $manager.clone();
        let task = move || {
            let f = |$m: &GlobalHotKeyManager| $ex;
            let _ = tx.send(f(&*manager));
        };
        $handle.run_on_main_thread(task)?;
        rx.recv()?
    }};
}

#[cfg(not(target_env = "ohos"))]
fn lock_shortcuts<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap()
}

/// Unified version gate for OHOS global shortcuts: `inputConsumer.on('hotkeyChange')`
/// is API 14+. Every register/unregister/query entry point checks this up front.
#[cfg(target_env = "ohos")]
fn ohos_require_hotkey_api(op: &str) -> Result<()> {
    use tauri::ohos::openharmony_ability::version;

    const MIN_HOTKEY_API_VERSION: i32 = 14;
    let current = version::sdk_api_version();
    if current < MIN_HOTKEY_API_VERSION {
        return Err(Error::GlobalHotkey(format!(
            "{op} requires API level {MIN_HOTKEY_API_VERSION}+ on OpenHarmony (current: {current})"
        )));
    }
    Ok(())
}

impl<R: Runtime> GlobalShortcut<R> {
    fn register_internal<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
        &self,
        shortcut: Shortcut,
        handler: Option<F>,
    ) -> Result<()> {
        let id = shortcut.id();
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("register")?;

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut))?;
            lock_shortcuts(&self.shortcuts).insert(id, RegisteredShortcut { shortcut, handler });
        }

        #[cfg(target_env = "ohos")]
        {
            let client = self.client.as_ref().ok_or_else(|| {
                Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
            })?;
            let modifiers = shortcut.modifiers().to_vec();
            let key = shortcut.code().to_ohos_name().to_string();
            let stamp = {
                let mut shortcuts = lock_shortcuts(&self.shortcuts);
                let stamp = ohos::next_registration_stamp();
                shortcuts.insert(
                    id,
                    RegisteredShortcut {
                        shortcut,
                        handler,
                        stamp,
                    },
                );
                stamp
            };
            ohos::spawn_register(
                client.clone(),
                id,
                &modifiers,
                &key,
                stamp,
                self.shortcuts.clone(),
            );
        }

        Ok(())
    }

    fn register_multiple_internal<S, F>(&self, shortcuts: S, handler: Option<F>) -> Result<()>
    where
        S: IntoIterator<Item = Shortcut>,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));

        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("register")?;

        let hotkeys = shortcuts.into_iter().collect::<Vec<_>>();

        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        for shortcut in hotkeys {
            #[cfg(not(target_env = "ohos"))]
            {
                run_main_thread!(self.app, self.manager, |m| m.0.register(shortcut))?;
            }

            #[cfg(target_env = "ohos")]
            let stamp = ohos::next_registration_stamp();

            #[cfg(target_env = "ohos")]
            {
                let client = self.client.as_ref().ok_or_else(|| {
                    Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
                })?;
                let modifiers = shortcut.modifiers().to_vec();
                let key = shortcut.code().to_ohos_name().to_string();
                let sid = shortcut.id();
                ohos::spawn_register(
                    client.clone(),
                    sid,
                    &modifiers,
                    &key,
                    stamp,
                    self.shortcuts.clone(),
                );
            }

            shortcuts.insert(
                shortcut.id(),
                RegisteredShortcut {
                    shortcut,
                    handler: handler.clone(),
                    #[cfg(target_env = "ohos")]
                    stamp,
                },
            );
        }

        Ok(())
    }

    /// Awaited variant of [`register_multiple_internal`] for the JS-facing
    /// `register` command on OHOS: awaits the real ArkTS receipt so a
    /// rejected registration rejects the command's promise.
    #[cfg(target_env = "ohos")]
    async fn register_multiple_internal_awaited<S, F>(
        &self,
        shortcuts: S,
        handler: Option<F>,
    ) -> Result<()>
    where
        S: IntoIterator<Item = Shortcut>,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        ohos_require_hotkey_api("register")?;

        let handler = handler.map(|h| Arc::new(Box::new(h) as HandlerFn<R>));
        let hotkeys = shortcuts.into_iter().collect::<Vec<_>>();

        for shortcut in hotkeys {
            let id = shortcut.id();
            let client = self.client.as_ref().ok_or_else(|| {
                Error::GlobalHotkey("GlobalShortcutClient not initialized".to_string())
            })?;
            let modifiers = shortcut.modifiers().to_vec();
            let key = shortcut.code().to_ohos_name().to_string();
            let modifier_names = ohos::to_ohos_modifier_names(&modifiers);
            let stamp = {
                let mut shortcuts = lock_shortcuts(&self.shortcuts);
                let stamp = ohos::next_registration_stamp();
                shortcuts.insert(
                    id,
                    RegisteredShortcut {
                        shortcut,
                        handler: handler.clone(),
                        stamp,
                    },
                );
                stamp
            };
            ohos::register_and_cleanup(client, id, &modifier_names, &key, stamp, &self.shortcuts)
                .await
                .map_err(Error::GlobalHotkey)?;
        }

        Ok(())
    }
}

impl<R: Runtime> GlobalShortcut<R> {
    /// Register a shortcut. Returns an error if the shortcut is invalid or already registered
    /// by this [`GlobalShortcut`] instance.
    pub fn register<S>(&self, shortcut: S) -> Result<()>
    where
        S: TryInto<ShortcutWrapper>,
        S::Error: std::error::Error,
    {
        self.register_internal(
            try_into_shortcut(shortcut)?,
            None::<fn(&AppHandle<R>, &Shortcut, ShortcutEvent)>,
        )
    }

    /// Register a shortcut with a handler that is called with the app handle, the triggered
    /// [`Shortcut`] and the [`ShortcutEvent`] (whose `state` is [`ShortcutState::Pressed`] or
    /// [`ShortcutState::Released`]) whenever the shortcut's key combination is pressed or
    /// released.
    pub fn on_shortcut<S, F>(&self, shortcut: S, handler: F) -> Result<()>
    where
        S: TryInto<ShortcutWrapper>,
        S::Error: std::error::Error,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        self.register_internal(try_into_shortcut(shortcut)?, Some(handler))
    }

    /// Register multiple shortcuts.
    pub fn register_multiple<S, T>(&self, shortcuts: S) -> Result<()>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        let mut s = Vec::new();
        for shortcut in shortcuts {
            s.push(try_into_shortcut(shortcut)?);
        }
        self.register_multiple_internal(s, None::<fn(&AppHandle<R>, &Shortcut, ShortcutEvent)>)
    }

    /// Register multiple shortcuts with a handler.
    pub fn on_shortcuts<S, T, F>(&self, shortcuts: S, handler: F) -> Result<()>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
        F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static,
    {
        let mut s = Vec::new();
        for shortcut in shortcuts {
            s.push(try_into_shortcut(shortcut)?);
        }
        self.register_multiple_internal(s, Some(handler))
    }

    /// Unregister a shortcut
    pub fn unregister<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> Result<()>
    where
        S::Error: std::error::Error,
    {
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregister")?;

        let shortcut = try_into_shortcut(shortcut)?;

        #[cfg(not(target_env = "ohos"))]
        {
            run_main_thread!(self.app, self.manager, |m| m.0.unregister(shortcut))?;
            lock_shortcuts(&self.shortcuts).remove(&shortcut.id());
        }

        #[cfg(target_env = "ohos")]
        {
            let generation = ohos::next_registration_stamp();
            if let Some(ref client) = self.client {
                ohos::spawn_unregister(client.clone(), shortcut.id());
            }

            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            if shortcuts
                .get(&shortcut.id())
                .is_some_and(|entry| entry.stamp < generation)
            {
                shortcuts.remove(&shortcut.id());
            }
        }

        Ok(())
    }

    /// Unregister multiple shortcuts.
    pub fn unregister_multiple<T: TryInto<ShortcutWrapper>, S: IntoIterator<Item = T>>(
        &self,
        shortcuts: S,
    ) -> Result<()>
    where
        T::Error: std::error::Error,
    {
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregister")?;

        let mut mapped_shortcuts = Vec::new();
        for shortcut in shortcuts {
            mapped_shortcuts.push(try_into_shortcut(shortcut)?);
        }

        #[cfg(not(target_env = "ohos"))]
        {
            let mapped_shortcuts = mapped_shortcuts.clone();
            #[rustfmt::skip]
            run_main_thread!(self.app, self.manager, |m| m.0.unregister_all(&mapped_shortcuts))?;
        }

        #[cfg(target_env = "ohos")]
        {
            let mut generations = HashMap::new();
            for s in &mapped_shortcuts {
                let generation = ohos::next_registration_stamp();
                generations.insert(s.id(), generation);
                if let Some(ref client) = self.client {
                    ohos::spawn_unregister(client.clone(), s.id());
                }
            }

            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            for (id, generation) in &generations {
                if shortcuts
                    .get(id)
                    .is_some_and(|entry| entry.stamp < *generation)
                {
                    shortcuts.remove(id);
                }
            }
        }

        #[cfg(not(target_env = "ohos"))]
        {
            let mut shortcuts = lock_shortcuts(&self.shortcuts);
            for s in mapped_shortcuts {
                shortcuts.remove(&s.id());
            }
        }

        Ok(())
    }

    /// Unregister all registered shortcuts.
    pub fn unregister_all(&self) -> Result<()> {
        #[cfg(target_env = "ohos")]
        ohos_require_hotkey_api("unregisterAll")?;

        let mut shortcuts = lock_shortcuts(&self.shortcuts);
        let hotkeys = std::mem::take(&mut *shortcuts);

        #[cfg(not(target_env = "ohos"))]
        {
            let hotkeys = hotkeys.values().map(|s| s.shortcut).collect::<Vec<_>>();
            #[rustfmt::skip]
            let res = run_main_thread!(self.app, self.manager, |m| m.0.unregister_all(hotkeys.as_slice()));
            res.map_err(Into::into)
        }

        #[cfg(target_env = "ohos")]
        {
            let _ = &hotkeys;
            if let Some(ref client) = self.client {
                ohos::spawn_unregister_all(client.clone());
            }
            Ok(())
        }
    }

    /// Determines whether the given shortcut is registered by this application or not.
    ///
    /// If the shortcut is registered by another application, it will still return `false`.
    pub fn is_registered<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> bool
    where
        S::Error: std::error::Error,
    {
        if let Ok(shortcut) = try_into_shortcut(shortcut) {
            lock_shortcuts(&self.shortcuts).contains_key(&shortcut.id())
        } else {
            false
        }
    }
}

/// Extension trait for [`Manager`] implementors (e.g. [`AppHandle`]) that exposes access to the
/// global shortcut APIs.
///
/// # Examples
///
/// ```rust,no_run
/// use tauri::Runtime;
/// use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
///
/// fn setup<R: Runtime>(app: &tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
///     app.global_shortcut()
///         .on_shortcut("CommandOrControl+Shift+C", |_app, shortcut, event| {
///             if event.state == ShortcutState::Pressed {
///                 println!("{shortcut:?} pressed");
///             }
///         })?;
///
///     Ok(())
/// }
/// ```
pub trait GlobalShortcutExt<R: Runtime> {
    /// Returns the [`GlobalShortcut`] instance managed by this plugin.
    fn global_shortcut(&self) -> &GlobalShortcut<R>;
}

impl<R: Runtime, T: Manager<R>> GlobalShortcutExt<R> for T {
    fn global_shortcut(&self) -> &GlobalShortcut<R> {
        self.state::<GlobalShortcut<R>>().inner()
    }
}

fn parse_shortcut<S: AsRef<str>>(shortcut: S) -> Result<Shortcut> {
    #[cfg(not(target_env = "ohos"))]
    {
        shortcut.as_ref().parse().map_err(Into::into)
    }
    #[cfg(target_env = "ohos")]
    {
        shortcut
            .as_ref()
            .parse::<Shortcut>()
            .map_err(Error::GlobalHotkey)
    }
}

fn try_into_shortcut<S: TryInto<ShortcutWrapper>>(shortcut: S) -> Result<Shortcut>
where
    S::Error: std::error::Error,
{
    shortcut
        .try_into()
        .map(|s| s.0)
        .map_err(|e| Error::GlobalHotkey(e.to_string()))
}

#[derive(Clone, Serialize)]
struct ShortcutJsEvent {
    shortcut: String,
    id: u32,
    state: ShortcutState,
}

#[cfg(not(target_env = "ohos"))]
#[tauri::command]
fn register<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcuts: Vec<String>,
    handler: Channel<ShortcutJsEvent>,
) -> Result<()> {
    let mut hotkeys = Vec::new();

    let mut shortcut_map = HashMap::new();
    for shortcut in shortcuts {
        let hotkey = parse_shortcut(&shortcut)?;
        shortcut_map.insert(hotkey.id(), shortcut);
        hotkeys.push(hotkey);
    }

    global_shortcut.register_multiple_internal(
        hotkeys,
        Some(
            move |_app: &AppHandle<R>, shortcut: &Shortcut, e: ShortcutEvent| {
                let js_event = ShortcutJsEvent {
                    id: e.id,
                    state: e.state,
                    shortcut: shortcut.into_string(),
                };
                let _ = handler.send(js_event);
            },
        ),
    )
}

#[cfg(target_env = "ohos")]
#[tauri::command]
async fn register<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcuts: Vec<String>,
    handler: Channel<ShortcutJsEvent>,
) -> Result<()> {
    let mut hotkeys = Vec::new();

    let mut shortcut_map = HashMap::new();
    for shortcut in shortcuts {
        let hotkey = parse_shortcut(&shortcut)?;
        shortcut_map.insert(hotkey.id(), shortcut);
        hotkeys.push(hotkey);
    }

    global_shortcut
        .register_multiple_internal_awaited(
            hotkeys,
            Some(
                move |_app: &AppHandle<R>, shortcut: &Shortcut, e: ShortcutEvent| {
                    let js_event = ShortcutJsEvent {
                        id: e.id,
                        state: e.state,
                        shortcut: shortcut.into_string(),
                    };
                    let _ = handler.send(js_event);
                },
            ),
        )
        .await
}

#[tauri::command]
fn unregister<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcuts: Vec<String>,
) -> Result<()> {
    let mut hotkeys = Vec::new();
    for shortcut in shortcuts {
        hotkeys.push(parse_shortcut(&shortcut)?);
    }
    global_shortcut.unregister_multiple(hotkeys)
}

#[tauri::command]
fn unregister_all<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
) -> Result<()> {
    global_shortcut.unregister_all()
}

#[tauri::command]
fn is_registered<R: Runtime>(
    _app: AppHandle<R>,
    global_shortcut: State<'_, GlobalShortcut<R>>,
    shortcut: String,
) -> Result<bool> {
    #[cfg(target_env = "ohos")]
    ohos_require_hotkey_api("isRegistered")?;

    Ok(global_shortcut.is_registered(parse_shortcut(shortcut)?))
}

/// Builder for the global shortcut plugin, letting you configure shortcuts and a handler that
/// are registered as soon as the plugin is set up, before [`Builder::build`] is called.
///
/// # Examples
///
/// ```rust,no_run
/// use tauri::Runtime;
///
/// fn setup<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
///     builder.plugin(
///         tauri_plugin_global_shortcut::Builder::new()
///             .with_shortcut("CommandOrControl+Shift+C")
///             .unwrap()
///             .with_handler(|_app, shortcut, event| {
///                 println!("{shortcut:?}: {event:?}");
///             })
///             .build(),
///     )
/// }
/// ```
pub struct Builder<R: Runtime> {
    shortcuts: Vec<Shortcut>,
    handler: Option<HandlerFn<R>>,
}

impl<R: Runtime> Default for Builder<R> {
    fn default() -> Self {
        Self {
            shortcuts: Vec::new(),
            handler: Default::default(),
        }
    }
}

impl<R: Runtime> Builder<R> {
    /// Creates a new [`Builder`] with no shortcuts or handler configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a shortcut to be registerd.
    pub fn with_shortcut<T>(mut self, shortcut: T) -> Result<Self>
    where
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        self.shortcuts.push(try_into_shortcut(shortcut)?);
        Ok(self)
    }

    /// Add multiple shortcuts to be registerd.
    pub fn with_shortcuts<S, T>(mut self, shortcuts: S) -> Result<Self>
    where
        S: IntoIterator<Item = T>,
        T: TryInto<ShortcutWrapper>,
        T::Error: std::error::Error,
    {
        for shortcut in shortcuts {
            self.shortcuts.push(try_into_shortcut(shortcut)?);
        }

        Ok(self)
    }

    /// Specify a global shortcut handler that will be triggered for any and all shortcuts.
    pub fn with_handler<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
        mut self,
        handler: F,
    ) -> Self {
        self.handler.replace(Box::new(handler));
        self
    }

    /// Builds the plugin. Shortcuts configured with [`Builder::with_shortcut`] /
    /// [`Builder::with_shortcuts`] are registered when the plugin is set up, and the handler
    /// configured with [`Builder::with_handler`] (if any) is called for every hotkey event, in
    /// addition to any handler passed to [`GlobalShortcut::on_shortcut`] /
    /// [`GlobalShortcut::on_shortcuts`] for that specific shortcut.
    pub fn build(self) -> TauriPlugin<R> {
        let handler = self.handler;
        let shortcuts = self.shortcuts;
        PluginBuilder::new("global-shortcut")
            .invoke_handler(tauri::generate_handler![
                register,
                unregister,
                unregister_all,
                is_registered,
            ])
            .setup(move |app, _api| {
                #[cfg(not(target_env = "ohos"))]
                {
                    let manager = global_hotkey::GlobalHotKeyManager::new()?;
                    let mut store = HashMap::<HotKeyId, RegisteredShortcut<R>>::new();
                    for shortcut in shortcuts {
                        manager.register(shortcut)?;
                        store.insert(
                            shortcut.id(),
                            RegisteredShortcut {
                                shortcut,
                                handler: None,
                            },
                        );
                    }

                    let shortcuts = Arc::new(Mutex::new(store));
                    let shortcuts_ = shortcuts.clone();

                    let app_handle = app.clone();
                    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                        dispatch_shortcut_event(&shortcuts_, &app_handle, handler.as_ref(), e);
                    }));

                    app.manage(GlobalShortcut {
                        app: app.clone(),
                        manager: Arc::new(GlobalHotKeyManager(manager)),
                        shortcuts,
                    });
                }

                #[cfg(target_env = "ohos")]
                {
                    let store = HashMap::<HotKeyId, RegisteredShortcut<R>>::new();
                    ohos_setup(app.clone(), shortcuts, handler, store);
                }

                Ok(())
            })
            .build()
    }
}

#[cfg(all(test, not(target_env = "ohos")))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::test::{MockRuntime, mock_app};

    #[test]
    fn handlers_run_without_holding_the_shortcuts_lock() {
        let app = mock_app();
        let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::KeyW);
        let shortcuts = Arc::new(Mutex::new(HashMap::<
            HotKeyId,
            RegisteredShortcut<MockRuntime>,
        >::new()));
        let calls = Arc::new(AtomicUsize::new(0));

        let shortcut_handler: HandlerFn<MockRuntime> = {
            let shortcuts = shortcuts.clone();
            let calls = calls.clone();
            Box::new(move |_, _, _| {
                assert!(
                    shortcuts.try_lock().is_ok(),
                    "the shortcuts lock must not be held while a handler runs"
                );
                calls.fetch_add(1, Ordering::SeqCst);
            })
        };
        let global_handler: HandlerFn<MockRuntime> = {
            let shortcuts = shortcuts.clone();
            let calls = calls.clone();
            Box::new(move |_, _, _| {
                assert!(
                    shortcuts.try_lock().is_ok(),
                    "the shortcuts lock must not be held while a handler runs"
                );
                calls.fetch_add(1, Ordering::SeqCst);
            })
        };

        shortcuts.lock().unwrap().insert(
            shortcut.id(),
            RegisteredShortcut {
                shortcut,
                handler: Some(Arc::new(shortcut_handler)),
            },
        );

        dispatch_shortcut_event(
            &shortcuts,
            app.handle(),
            Some(&global_handler),
            GlobalHotKeyEvent {
                id: shortcut.id(),
                state: ShortcutState::Pressed,
            },
        );

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[cfg(test)]
mod race_model {
    use std::collections::{BTreeSet, HashMap, HashSet};

    /// A message on the Rust → ArkTS bridge queue.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Msg {
        Register { id: u32, generation: u32 },
        Unregister { id: u32, generation: u32 },
        Sweep { generation: u32 },
    }

    /// Model of both sides of the generation protocol.
    #[derive(Clone, Default)]
    struct World {
        counter: u32,
        local: HashMap<u32, u32>,
        queue: Vec<Msg>,
        floor: u32,
        watermarks: HashMap<u32, u32>,
        physical: HashSet<u32>,
    }

    impl World {
        fn next_stamp(&mut self) -> u32 {
            self.counter += 1;
            self.counter
        }

        fn arkts_register(&mut self, id: u32, generation: u32) {
            if generation < self.floor
                || self.watermarks.get(&id).is_some_and(|&s| generation < s)
            {
                if self.local.get(&id) == Some(&generation) {
                    self.local.remove(&id);
                }
                return;
            }
            self.physical.insert(id);
            self.watermarks.insert(id, generation);
        }

        fn arkts_unregister(&mut self, id: u32, generation: u32) {
            if self.watermarks.get(&id).is_some_and(|&s| generation < s) {
                return;
            }
            self.watermarks.insert(id, generation);
            self.physical.remove(&id);
        }

        fn arkts_sweep(&mut self, generation: u32) {
            if generation > self.floor {
                self.floor = generation;
            }
            self.watermarks.retain(|_, &mut g| g > generation);
            let ids: Vec<u32> = self.physical.iter().copied().collect();
            for id in ids {
                if !self.watermarks.get(&id).is_some_and(|&g| g > generation) {
                    self.physical.remove(&id);
                }
            }
        }

        fn drain(&mut self, order: &[usize]) {
            for &i in order {
                match self.queue[i].clone() {
                    Msg::Register { id, generation } => self.arkts_register(id, generation),
                    Msg::Unregister { id, generation } => self.arkts_unregister(id, generation),
                    Msg::Sweep { generation } => self.arkts_sweep(generation),
                }
            }
        }

        fn local_ids(&self) -> BTreeSet<u32> {
            self.local.keys().copied().collect()
        }

        fn physical_ids(&self) -> BTreeSet<u32> {
            self.physical.iter().copied().collect()
        }
    }

    type Step = Box<dyn Fn(&mut World, &mut u32)>;

    fn op_register(id: u32) -> Vec<Step> {
        vec![
            Box::new(move |w, t| {
                *t = w.next_stamp();
                w.local.insert(id, *t);
            }),
            Box::new(move |w, t| {
                w.queue.push(Msg::Register {
                    id,
                    generation: *t,
                })
            }),
        ]
    }

    fn op_unregister(id: u32) -> Vec<Step> {
        vec![
            Box::new(move |w, t| *t = w.next_stamp()),
            Box::new(move |w, t| {
                w.queue.push(Msg::Unregister {
                    id,
                    generation: *t,
                })
            }),
            Box::new(move |w, t| {
                if w.local.get(&id).is_some_and(|&s| s < *t) {
                    w.local.remove(&id);
                }
            }),
        ]
    }

    fn op_unregister_all() -> Vec<Step> {
        vec![Box::new(move |w, t| {
            w.local.clear();
            *t = w.next_stamp();
            w.queue.push(Msg::Sweep { generation: *t });
        })]
    }

    fn run_op(op: &[Step], w: &mut World) {
        let mut t = 0u32;
        for step in op {
            step(w, &mut t);
        }
    }

    fn for_each_interleaving(a: &[Step], b: &[Step], mut f: impl FnMut(World)) {
        let n = a.len() + b.len();
        for mask in 0u32..(1 << n) {
            if mask.count_ones() != a.len() as u32 {
                continue;
            }
            let mut w = World::default();
            let (mut ai, mut bi, mut ta, mut tb) = (0, 0, 0, 0);
            for i in 0..n {
                if mask & (1 << i) != 0 {
                    a[ai](&mut w, &mut ta);
                    ai += 1;
                } else {
                    b[bi](&mut w, &mut tb);
                    bi += 1;
                }
            }
            f(w);
        }
    }

    fn all_orders(len: usize) -> Vec<Vec<usize>> {
        fn go(prefix: Vec<usize>, rest: Vec<usize>, out: &mut Vec<Vec<usize>>) {
            if rest.is_empty() {
                out.push(prefix);
                return;
            }
            for i in 0..rest.len() {
                let mut rest2 = rest.clone();
                let head = rest2.remove(i);
                let mut prefix2 = prefix.clone();
                prefix2.push(head);
                go(prefix2, rest2, out);
            }
        }
        let mut out = Vec::new();
        go(Vec::new(), (0..len).collect(), &mut out);
        out
    }

    fn assert_pair_consistent(a: &[Step], b: &[Step]) {
        for_each_interleaving(a, b, |w| {
            for order in all_orders(w.queue.len()) {
                let mut drained = w.clone();
                drained.drain(&order);
                assert_eq!(
                    drained.local_ids(),
                    drained.physical_ids(),
                    "local store and physical registrations disagree \
                     (queue {:?} drained as {:?}; floor={}, watermarks={:?})",
                    w.queue,
                    order,
                    drained.floor,
                    drained.watermarks
                );
            }
        });
    }

    #[test]
    fn unregister_racing_re_register_is_bidirectionally_consistent() {
        assert_pair_consistent(&op_unregister(7), &op_register(7));
    }

    #[test]
    fn unregister_all_racing_re_register_is_bidirectionally_consistent() {
        assert_pair_consistent(&op_unregister_all(), &op_register(7));
    }

    #[test]
    fn unregister_all_racing_unregister_is_consistent() {
        assert_pair_consistent(&op_unregister_all(), &op_unregister(7));
    }

    #[test]
    fn re_register_racing_unregister_survives_it() {
        let mut w = World::default();
        let (mut tu, mut tr) = (0, 0);
        let u = op_unregister(7);
        let r = op_register(7);
        u[0](&mut w, &mut tu);
        u[1](&mut w, &mut tu);
        r[0](&mut w, &mut tr);
        u[2](&mut w, &mut tu);
        r[1](&mut w, &mut tr);
        for order in all_orders(w.queue.len()) {
            let mut drained = w.clone();
            drained.drain(&order);
            assert!(
                drained.local_ids().contains(&7) && drained.physical_ids().contains(&7),
                "re-register that stamped after the unregister must survive it \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }

    #[test]
    fn re_register_after_sweep_survives_the_sweep() {
        for order in all_orders(3) {
            let mut w = World::default();
            run_op(&op_register(7), &mut w);
            run_op(&op_unregister_all(), &mut w);
            run_op(&op_register(7), &mut w);
            let mut drained = w.clone();
            drained.drain(&order);
            assert_eq!(
                drained.local_ids(),
                BTreeSet::from([7]),
                "post-sweep re-register must stay in the local store \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
            assert_eq!(
                drained.physical_ids(),
                BTreeSet::from([7]),
                "post-sweep re-register must stay physically registered \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }

    #[test]
    fn sweep_unregister_and_re_register_triple_is_consistent() {
        let mut w = World::default();
        let (mut tu, mut tr) = (0, 0);
        let u = op_unregister(7);
        let s = op_unregister_all();
        let r = op_register(7);
        u[0](&mut w, &mut tu);
        s[0](&mut w, &mut 0);
        r[0](&mut w, &mut tr);
        u[1](&mut w, &mut tu);
        u[2](&mut w, &mut tu);
        r[1](&mut w, &mut tr);
        for order in all_orders(w.queue.len()) {
            let mut drained = w.clone();
            drained.drain(&order);
            assert_eq!(
                drained.local_ids(),
                drained.physical_ids(),
                "triple race disagrees (queue {:?} drained as {:?})",
                w.queue,
                order
            );
            assert!(
                drained.local_ids().contains(&7),
                "newest re-register must survive the triple race \
                 (queue {:?} drained as {:?})",
                w.queue,
                order
            );
        }
    }
}
