#![cfg(mobile)]

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_device_identity::Identity;

/// Returns only the public identity used by the pairing claim. The private signing key remains
/// inside the platform Keychain implementation in `plugins/device-identity`.
#[tauri::command]
pub fn device_identity_get<R: Runtime>(app: AppHandle<R>) -> Result<Identity, String> {
    app.state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
        .get()
        .map_err(|error| error.to_string())
}

/// Replaces the native identity after an explicit re-pairing action. The old public key remains
/// revoked at the node; this creates a new key rather than silently restoring trust.
#[tauri::command]
pub fn device_identity_reset<R: Runtime>(app: AppHandle<R>) -> Result<Identity, String> {
    app.state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
        .reset()
        .map_err(|error| error.to_string())
}
