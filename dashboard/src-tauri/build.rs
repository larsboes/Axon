// Declaring the app's own commands here makes Tauri generate an
// `allow-<command>` permission for each, and refuse any command a capability
// in `capabilities/` does not grant. Without the manifest every app command is
// callable from every window.
const COMMANDS: &[&str] = &[
    "mac_request",
    "mac_request_bytes",
    "mac_settings_get",
    "mac_settings_set",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
