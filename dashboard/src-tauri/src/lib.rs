mod mac_bridge;
mod sync;

use std::time::Duration;

use tauri::Manager;

/// How often pending edits are retried while any wait (sync step 2, item 6).
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_roomplan::init());

    builder
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            mac_bridge::mac_request,
            mac_bridge::mac_request_bytes,
            mac_bridge::mac_settings_get,
            mac_bridge::mac_settings_set,
            mac_bridge::sync_status,
            mac_bridge::sync_entries,
            mac_bridge::sync_flush,
            mac_bridge::sync_resolve,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            app.manage(mac_bridge::Local::open(app.handle()));
            spawn_flush_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Flushes pending edits once at start and then every [`FLUSH_INTERVAL`] while any wait. The
/// foreground flush and "Sync now" come from the page (`sync_flush`). On iOS the thread is
/// suspended with the app, so this does not run in the background.
fn spawn_flush_loop<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || loop {
        let local = app.state::<mac_bridge::Local>();
        if let Some(store) = &local.store {
            if store.has_pending() {
                if let Ok(t) = mac_bridge::transport(&app) {
                    tauri::async_runtime::block_on(sync::flush(store, &t, mac_bridge::now_ms()));
                }
            }
        }
        std::thread::sleep(FLUSH_INTERVAL);
    });
}
