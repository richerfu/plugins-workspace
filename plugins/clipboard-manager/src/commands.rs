// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use tauri::{AppHandle, Manager, ResourceId, Runtime, State, Webview, command, image::JsImage};

use crate::{Clipboard, Result};

#[command]
#[cfg(any(desktop, target_env = "ohos"))]
pub(crate) async fn write_text<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
    text: &str,
    #[allow(unused)] label: Option<String>,
) -> Result<()> {
    #[cfg(target_env = "ohos")]
    {
        clipboard.write_text(text).await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        clipboard.write_text(text)
    }
}

#[command]
#[cfg(all(not(desktop), not(target_env = "ohos")))]
pub(crate) async fn write_text<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
    text: &str,
    #[allow(unused)] label: Option<&str>,
) -> Result<()> {
    match label {
        Some(label) => clipboard.write_text_with_label(text, label),
        None => clipboard.write_text(text),
    }
}

#[command]
pub(crate) async fn read_text<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
) -> Result<String> {
    #[cfg(target_env = "ohos")]
    {
        clipboard.read_text().await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        clipboard.read_text()
    }
}

#[command]
pub(crate) async fn write_image<R: Runtime>(
    webview: Webview<R>,
    clipboard: State<'_, Clipboard<R>>,
    image: JsImage,
) -> Result<()> {
    #[cfg(target_env = "ohos")]
    {
        let (rgba, width, height) = {
            let resources_table = webview.resources_table();
            let img = image.into_img(&resources_table)?;
            (img.rgba().to_vec(), img.width(), img.height())
        };
        clipboard.write_image(&rgba, width, height).await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        let resources_table = webview.resources_table();
        let image = image.into_img(&resources_table)?;
        clipboard.write_image(&image)
    }
}

#[command]
pub(crate) async fn read_image<R: Runtime>(
    webview: Webview<R>,
    clipboard: State<'_, Clipboard<R>>,
) -> Result<ResourceId> {
    #[cfg(target_env = "ohos")]
    {
        let image = clipboard.read_image().await?;
        let mut resources_table = webview.resources_table();
        let rid = resources_table.add(image);
        Ok(rid)
    }
    #[cfg(not(target_env = "ohos"))]
    {
        let image = clipboard.read_image()?.to_owned();
        let mut resources_table = webview.resources_table();
        let rid = resources_table.add(image);
        Ok(rid)
    }
}

#[command]
pub(crate) async fn write_html<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
    html: &str,
    alt_text: Option<&str>,
) -> Result<()> {
    #[cfg(target_env = "ohos")]
    {
        clipboard.write_html(html, alt_text).await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        clipboard.write_html(html, alt_text)
    }
}

#[command]
pub(crate) async fn clear<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
) -> Result<()> {
    #[cfg(target_env = "ohos")]
    {
        clipboard.clear().await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        clipboard.clear()
    }
}

#[command]
pub(crate) async fn write_secret<R: Runtime>(
    _app: AppHandle<R>,
    clipboard: State<'_, Clipboard<R>>,
    text: &str,
) -> Result<()> {
    #[cfg(target_env = "ohos")]
    {
        clipboard.write_secret(text).await
    }
    #[cfg(not(target_env = "ohos"))]
    {
        clipboard.write_secret(text)
    }
}
