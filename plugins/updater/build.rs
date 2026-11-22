// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

const COMMANDS: &[&str] = &[
    "check",
    "download",
    "install",
    "download_and_install",
    "check_app_gallery_update",
    "show_app_gallery_update_dialog",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .global_api_script_path("./api-iife.js")
        .build();

    tauri_plugin::cfg_aliases();
}
