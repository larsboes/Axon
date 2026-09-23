//! Tauri bridge for the native, offline RoomPlan capture implementation.
//!
//! The iOS target owns camera access, RoomCaptureSession, RoomBuilder, and local asset storage.
//! This crate only transports the structured draft and opaque asset handles across the Tauri
//! boundary. Desktop builds intentionally expose no fake scanner.

#![cfg(mobile)]

use roomplan_contract::CaptureDraft;
use serde::{Deserialize, Serialize};
use tauri::{plugin::Builder, plugin::PluginHandle, plugin::TauriPlugin, Manager, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_roomplan);

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureOptions {
    pub include_mesh: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureEvent {
    pub draft: CaptureDraft,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("native RoomPlan command failed: {0}")]
    Native(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Native RoomPlan plugin handle.
pub struct Roomplan<R: Runtime>(PluginHandle<R>);

pub trait RoomplanExt<R: Runtime> {
    fn roomplan(&self) -> &Roomplan<R>;
}

impl<R: Runtime, T: Manager<R>> RoomplanExt<R> for T {
    fn roomplan(&self) -> &Roomplan<R> {
        self.state::<Roomplan<R>>().inner()
    }
}

impl<R: Runtime> Roomplan<R> {
    /// Starts a local capture. Completion arrives through the `capture-completed` event.
    pub fn start_capture(&self, options: StartCaptureOptions) -> Result<serde_json::Value> {
        self.0
            .run_mobile_plugin("startCapture", options)
            .map_err(|error| Error::Native(error.to_string()))
    }

    pub fn stop_capture(&self) -> Result<serde_json::Value> {
        self.0
            .run_mobile_plugin("stopCapture", ())
            .map_err(|error| Error::Native(error.to_string()))
    }

    pub fn cancel_capture(&self) -> Result<serde_json::Value> {
        self.0
            .run_mobile_plugin("cancelCapture", ())
            .map_err(|error| Error::Native(error.to_string()))
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("roomplan")
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            let handle = api.register_ios_plugin(init_plugin_roomplan)?;
            #[cfg(target_os = "android")]
            compile_error!("RoomPlan is an Apple-only capability; do not add an Android wrapper.");
            app.manage(Roomplan(handle));
            Ok(())
        })
        .build()
}
