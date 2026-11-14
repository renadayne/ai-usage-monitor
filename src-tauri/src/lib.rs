// The directory keeps the planned name `core/`, but the module is called
// `engine` so it can never shadow the built-in `core` crate.
#[path = "core/mod.rs"]
pub mod engine;
pub mod providers;

mod commands;
mod tray;

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};

use commands::AppState;
use engine::model::{ProviderId, ProviderSnapshot};
use engine::provider::Provider;
use engine::scheduler::{Scheduler, SchedulerConfig, SnapshotSink};
use engine::store::Store;
use providers::mock::MockProvider;

pub const MAIN_WINDOW: &str = "main";
pub const EVENT_SNAPSHOT_UPDATED: &str = "snapshot-updated";

struct TauriSink(AppHandle);

impl SnapshotSink for TauriSink {
    fn publish(&self, snapshot: &ProviderSnapshot) {
        let _ = self.0.emit(EVENT_SNAPSHOT_UPDATED, snapshot);
    }
}

fn open_store(app: &AppHandle) -> Option<Arc<Store>> {
    let dir = app.path().app_local_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Store::open(&dir.join("usage.sqlite")).ok().map(Arc::new)
}

fn build_providers() -> Vec<Arc<dyn Provider>> {
    // Real providers replace the mocks one by one in later steps.
    ProviderId::ALL
        .into_iter()
        .map(|id| Arc::new(MockProvider::new(id, Duration::from_secs(15))) as Arc<dyn Provider>)
        .collect()
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let store = open_store(&handle);
            let scheduler = Arc::new(Scheduler::new(
                build_providers(),
                SchedulerConfig::default(),
                store,
                Arc::new(TauriSink(handle.clone())),
            ));

            // `start` spawns tokio tasks, so it must run inside the runtime.
            let starter = Arc::clone(&scheduler);
            tauri::async_runtime::block_on(async move {
                starter.start();
            });

            let refresher = Arc::clone(&scheduler);
            handle.listen(tray::EVENT_REFRESH_REQUESTED, move |_event| {
                refresher.refresh(None);
            });

            app.manage(AppState { scheduler });
            tray::create(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh
        ])
        .on_window_event(|window, event| {
            // Closing the dashboard hides it to the tray instead of quitting.
            if window.label() == MAIN_WINDOW {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
