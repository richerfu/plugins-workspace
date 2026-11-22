// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

const COMMANDS: &[&str] = &[
    "initialize",
    "destroy",
    "save",
    "create_client",
    "load_client",
    "get_store_record",
    "save_store_record",
    "remove_store_record",
    "save_secret",
    "remove_secret",
    "execute_procedure",
];

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("ohos")
        && !cfg!(target_env = "ohos")
    {
        println!("cargo:rerun-if-env-changed=SODIUM_LIB_DIR");
        match std::env::var("SODIUM_LIB_DIR") {
            Ok(dir) => {
                println!("cargo:warning=[stronghold] OHOS: using prebuilt libsodium from {dir}");
            }
            Err(_) => {
                let target =
                    std::env::var("TARGET").unwrap_or_else(|_| "the OHOS target".to_string());
                panic!(
                    "cross-compiling to OHOS requires SODIUM_LIB_DIR to be set. \
                     libsodium-sys-stable's build script invokes ./configure, which cannot \
                     produce {target} artifacts on this host. Obtain a prebuilt libsodium for \
                     {target} and point SODIUM_LIB_DIR at its lib directory."
                );
            }
        }
    }

    tauri_plugin::Builder::new(COMMANDS)
        .global_api_script_path("./api-iife.js")
        .build();
}
