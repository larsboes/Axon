// Declaring the app's own commands here makes Tauri generate an
// `allow-<command>` permission for each, and refuse any command a capability
// in `capabilities/` does not grant. Without the manifest every app command is
// callable from every window.
const COMMANDS: &[&str] = &[
    "mac_request",
    "mac_request_bytes",
    "connection_settings_get",
    "connection_settings_set",
    "sync_status",
    "sync_entries",
    "sync_flush",
    "sync_resolve",
    "device_identity_get",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");

    // Cargo forwards the Swift search paths from Tauri's iOS build scripts but
    // does not retain their `rustc-link-lib` instructions when this package
    // emits both a staticlib and cdylib. Link the generated Swift archives from
    // the final app library explicitly. This is required with Xcode 27, where
    // the unresolved bridge symbols otherwise surface only at the final link.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        for library in [
            "Tauri",
            "tauri-plugin-log",
            "tauri-plugin-notification",
            "tauri-plugin-device-identity",
            "tauri-plugin-roomplan",
        ] {
            println!("cargo:rustc-link-lib=static={library}");
        }
    }
}
