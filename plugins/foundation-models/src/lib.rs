//! Tauri bridge for the optional on-device Apple language model.
//!
//! The iOS target owns the model: availability, token counting and generation all run in
//! `ios/Sources/FoundationModelsPlugin`. The WebView calls it directly as
//! `plugin:foundation-models|availability` and `plugin:foundation-models|respond`, so this crate
//! only registers it. Desktop builds expose nothing: on the Mac the same model is served by
//! `capabilities/foundation-models` over loopback.

#![cfg(mobile)]

use tauri::{plugin::Builder, plugin::TauriPlugin, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_foundation_models);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("foundation-models")
        .setup(|_app, _api| {
            #[cfg(target_os = "ios")]
            _api.register_ios_plugin(init_plugin_foundation_models)?;
            #[cfg(target_os = "android")]
            compile_error!("Foundation Models is an Apple framework; do not add an Android wrapper.");
            Ok(())
        })
        .build()
}
