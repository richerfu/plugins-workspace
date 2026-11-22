// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

//! Types and functions related to shell.

#[cfg(not(target_env = "ohos"))]
use std::ffi::OsStr;
use std::path::Path;

#[cfg(not(target_env = "ohos"))]
pub(crate) fn open<P: AsRef<OsStr>, S: AsRef<str>>(path: P, with: Option<S>) -> crate::Result<()> {
    match with {
        Some(program) => ::open::with_detached(path, program.as_ref()),
        None => ::open::that_detached(path),
    }
    .map_err(Into::into)
}

/// Opens URL with the program specified in `with`, or system default if `None`.
///
/// ## Platform-specific:
///
/// - **Android / iOS**: Always opens using default program.
///
/// # Examples
///
/// ```rust,no_run
/// tauri::Builder::default()
///   .setup(|app| {
///     // open the given URL on the system default browser
///     tauri_plugin_opener::open_url("https://github.com/tauri-apps/tauri", None::<&str>)?;
///     Ok(())
///   });
/// ```
#[cfg(not(target_env = "ohos"))]
pub fn open_url<P: AsRef<str>, S: AsRef<str>>(url: P, with: Option<S>) -> crate::Result<()> {
    let url = url.as_ref();
    open(url, with)
}

/// OHOS variant of [`open_url`]: async — routed through the
/// openharmony-ability bridge. `file://` URIs are routed via [`UrlExt::open_file`],
/// while all other schemes (including `https://`, `mailto:`, `tel:`) are routed
/// via [`UrlExt::open_url`]. `'open with'` (`with`) is unsupported on OHOS —
/// system default only.
#[cfg(target_env = "ohos")]
pub async fn open_url<P: AsRef<str>, S: AsRef<str>>(url: P, with: Option<S>) -> crate::Result<()> {
    use openharmony_ability_plugin_url::UrlExt;

    let _ = with;
    let url = url.as_ref();
    let ohos_app = crate::ohos_app()?;
    let res = if url.starts_with("file://") {
        ohos_app.open_file(url.to_string()).await
    } else {
        ohos_app.open_url(url.to_string()).await
    };
    res.map_err(|e| crate::Error::OpenharmonyAbility(e.to_string()))?;
    Ok(())
}

/// Opens path with the program specified in `with`, or system default if `None`.
///
/// ## Platform-specific:
///
/// - **Android / iOS**: Always opens using default program.
///
/// # Examples
///
/// ```rust,no_run
/// tauri::Builder::default()
///   .setup(|app| {
///     // open the given URL on the system default explorer
///     tauri_plugin_opener::open_path("/path/to/file", None::<&str>)?;
///     Ok(())
///   });
/// ```
#[cfg(not(target_env = "ohos"))]
pub fn open_path<P: AsRef<Path>, S: AsRef<str>>(path: P, with: Option<S>) -> crate::Result<()> {
    let path = path.as_ref();
    if with.is_none() {
        // Returns an IO error if not exists, and besides `exists()` is a shorthand for `metadata()`
        _ = path.metadata()?;
    }
    open(path, with)
}

/// OHOS variant of [`open_path`]: async — canonicalizes the path to a `file://`
/// URI and opens it through [`UrlExt::open_file`]. `'open with'` (`with`) is
/// unsupported on OHOS — system default only.
#[cfg(target_env = "ohos")]
pub async fn open_path<P: AsRef<Path>, S: AsRef<str>>(
    path: P,
    with: Option<S>,
) -> crate::Result<()> {
    use openharmony_ability_plugin_url::UrlExt;

    let _ = with;
    let canon = std::fs::canonicalize(path.as_ref())?;
    let uri = url::Url::from_file_path(&canon)
        .map_err(|_| crate::Error::InvalidPath(path.as_ref().to_string_lossy().to_string()))?;
    let ohos_app = crate::ohos_app()?;
    ohos_app
        .open_file(uri.to_string())
        .await
        .map_err(|e| crate::Error::OpenharmonyAbility(e.to_string()))?;
    Ok(())
}
