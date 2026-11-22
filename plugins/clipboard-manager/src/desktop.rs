// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

#[cfg(all(desktop, not(target_env = "ohos")))]
use arboard::ImageData;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, image::Image, plugin::PluginApi};

use std::borrow::Cow;
#[cfg(all(desktop, not(target_env = "ohos")))]
use std::sync::Mutex;

#[cfg(all(desktop, not(target_env = "ohos")))]
pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Clipboard<R>> {
    Ok(Clipboard {
        app: app.clone(),
        clipboard: arboard::Clipboard::new().map(|c| Mutex::new(Some(c))),
    })
}

/// Access to the clipboard APIs.
#[cfg(all(desktop, not(target_env = "ohos")))]
pub struct Clipboard<R: Runtime> {
    #[allow(dead_code)]
    app: AppHandle<R>,
    // According to arboard docs the clipboard must be dropped before exit.
    // Since tauri doesn't call drop on exit we'll use an Option to take() on RunEvent::Exit.
    clipboard: Result<Mutex<Option<arboard::Clipboard>>, arboard::Error>,
}

#[cfg(all(desktop, not(target_env = "ohos")))]
impl<R: Runtime> Clipboard<R> {
    fn with_clipboard<T>(
        &self,
        f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    ) -> crate::Result<T> {
        match &self.clipboard {
            Ok(clipboard) => f(clipboard.lock().unwrap().as_mut().unwrap()).map_err(Into::into),
            Err(e) => Err(crate::Error::Clipboard(e.to_string())),
        }
    }

    /// Writes plain text to the system clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Clipboard`] if the clipboard could not be initialized or the
    /// underlying [`arboard`] operation fails.
    pub fn write_text<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        self.with_clipboard(|c| c.set_text(text))
    }

    /// Writes an image to the system clipboard as RGBA data.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Clipboard`] if the clipboard could not be initialized or the
    /// underlying [`arboard`] operation fails.
    pub fn write_image(&self, image: &Image<'_>) -> crate::Result<()> {
        self.with_clipboard(|c| {
            c.set_image(ImageData {
                bytes: Cow::Borrowed(image.rgba()),
                width: image.width() as usize,
                height: image.height() as usize,
            })
        })
    }

    /// Warning: This method should not be used on the main thread! Otherwise the underlying libraries may deadlock on Linux, freezing the whole app, when trying to copy data copied from this app, for example if the user copies text from the WebView.
    pub fn read_text(&self) -> crate::Result<String> {
        self.with_clipboard(|c| c.get_text())
    }

    /// Writes HTML to the system clipboard, with an optional plain text fallback for targets
    /// that cannot render it.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Clipboard`] if the clipboard could not be initialized or the
    /// underlying [`arboard`] operation fails.
    pub fn write_html<'a, T: Into<Cow<'a, str>>>(
        &self,
        html: T,
        alt_text: Option<T>,
    ) -> crate::Result<()> {
        self.with_clipboard(|c| c.set_html(html, alt_text))
    }

    /// Clears the system clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Clipboard`] if the clipboard could not be initialized or the
    /// underlying [`arboard`] operation fails.
    pub fn clear(&self) -> crate::Result<()> {
        self.with_clipboard(|c| c.clear())
    }

    /// Warning: This method should not be used on the main thread! Otherwise the underlying libraries may deadlock on Linux, freezing the whole app, when trying to copy data copied from this app, for example if the user copies text from the WebView.
    pub fn read_image(&self) -> crate::Result<Image<'_>> {
        let image = self.with_clipboard(|c| c.get_image())?;
        Ok(Image::new_owned(
            image.bytes.to_vec(),
            image.width as u32,
            image.height as u32,
        ))
    }

    pub(crate) fn cleanup(&self) {
        if let Ok(clipboard) = &self.clipboard {
            clipboard.lock().unwrap().take();
        }
    }

    /// This is the same as write_text but it will set hints using `arboard`'s `SetExt*` extensions so clipboard managers exclude the text from history.
    pub fn write_secret<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        self.with_clipboard(|c| crate::secret::exclude_secret(c.set()).text(text))
    }
}

// ─── OpenHarmony (OHOS) clipboard via openharmony-ability-plugin-clipboard ───

#[cfg(target_env = "ohos")]
fn with_ohos_app<R>(
    f: impl FnOnce(&tauri::ohos::openharmony_ability::OpenHarmonyApp) -> R,
) -> Option<R> {
    let guard = tauri::ohos::APP.lock().ok()?;
    guard.as_ref().map(f)
}

#[cfg(target_env = "ohos")]
fn clipboard_client() -> crate::Result<openharmony_ability_plugin_clipboard::ClipboardClient> {
    use openharmony_ability_plugin_clipboard::ClipboardExt;

    with_ohos_app(|app| app.clipboard().ok())
        .flatten()
        .ok_or_else(|| {
            crate::Error::Clipboard(
                "Failed to create ClipboardClient: OHOS APP not initialized".to_string(),
            )
        })
}

#[cfg(target_env = "ohos")]
pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Clipboard<R>> {
    use openharmony_ability_plugin_clipboard::ClipboardBridgePlugin;

    let _ = with_ohos_app(|ohos_app| {
        match ohos_app.registered_plugin::<ClipboardBridgePlugin>() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(e) = ohos_app.register_plugin(ClipboardBridgePlugin::default()) {
                    log::error!(
                        "[clipboard-manager] failed to register ClipboardBridgePlugin: {}",
                        e
                    );
                }
            }
            Err(e) => {
                log::error!(
                    "[clipboard-manager] failed to check ClipboardBridgePlugin registration: {}",
                    e
                );
            }
        }
    });
    Ok(Clipboard { app: app.clone() })
}

/// Access to the clipboard APIs on OpenHarmony.
#[cfg(target_env = "ohos")]
pub struct Clipboard<R: Runtime> {
    #[allow(dead_code)]
    app: AppHandle<R>,
}

#[cfg(target_env = "ohos")]
impl<R: Runtime> Clipboard<R> {
    /// Writes plain text to the system pasteboard.
    pub async fn write_text<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .write_text(text.into().into_owned())
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    /// Reads plain text from the system pasteboard.
    pub async fn read_text(&self) -> crate::Result<String> {
        let client = clipboard_client()?;
        client
            .read_text()
            .await
            .map(|opt| opt.unwrap_or_default())
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    /// Writes an RGBA image to the system pasteboard.
    pub async fn write_image(&self, rgba: &[u8], width: u32, height: u32) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .write_image(rgba, width, height)
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    /// Writes HTML to the system pasteboard, optionally accompanied by a `text/plain` fallback.
    pub async fn write_html<'a, T: Into<Cow<'a, str>>>(
        &self,
        html: T,
        alt_text: Option<T>,
    ) -> crate::Result<()> {
        use openharmony_ability_plugin_clipboard::ClipboardMimeContent;

        let client = clipboard_client()?;
        let html = html.into().into_owned();
        let alt_text = alt_text
            .map(|t| t.into().into_owned())
            .filter(|t| !t.is_empty());

        if let Some(alt) = alt_text {
            client
                .write_mime(vec![
                    ClipboardMimeContent {
                        mime: "text/html".to_string(),
                        data: html.into_bytes(),
                    },
                    ClipboardMimeContent {
                        mime: "text/plain".to_string(),
                        data: alt.into_bytes(),
                    },
                ])
                .await
                .map_err(|e| crate::Error::Clipboard(e.to_string()))
        } else {
            client
                .write_html(html)
                .await
                .map_err(|e| crate::Error::Clipboard(e.to_string()))
        }
    }

    /// Clears the system pasteboard.
    pub async fn clear(&self) -> crate::Result<()> {
        let client = clipboard_client()?;
        client
            .clear()
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))
    }

    /// Reads an image from the system pasteboard and decodes it to owned RGBA pixels.
    pub async fn read_image(&self) -> crate::Result<Image<'static>> {
        let client = clipboard_client()?;
        let content = client
            .read_content()
            .await
            .map_err(|e| crate::Error::Clipboard(e.to_string()))?;

        let encoded = match content.png {
            Some(bytes) => bytes,
            None => client
                .read_records()
                .await
                .map_err(|e| crate::Error::Clipboard(e.to_string()))?
                .into_iter()
                .find_map(|record| record.encoded_image)
                .ok_or_else(|| {
                    crate::Error::Clipboard("clipboard does not contain an image".to_string())
                })?,
        };

        let decoded = image::load_from_memory(&encoded)
            .map_err(|e| crate::Error::Clipboard(format!("failed to decode clipboard image: {e}")))?
            .to_rgba8();
        let (width, height) = decoded.dimensions();
        Ok(Image::new_owned(decoded.into_raw(), width, height))
    }

    /// Writes sensitive plain text to the system pasteboard.
    ///
    /// OpenHarmony's native pasteboard API does not expose a public history-exclusion flag,
    /// so this delegates to [`Self::write_text`].
    pub async fn write_secret<'a, T: Into<Cow<'a, str>>>(&self, text: T) -> crate::Result<()> {
        self.write_text(text).await
    }

    pub(crate) fn cleanup(&self) {}
}
