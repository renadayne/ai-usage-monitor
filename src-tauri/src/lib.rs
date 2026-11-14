// The directory keeps the planned name `core/`, but the module is called
// `engine` so it can never shadow the built-in `core` crate.
#[path = "core/mod.rs"]
pub mod engine;
pub mod providers;

mod commands;
mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};

use commands::AppState;
use engine::model::{ProviderId, ProviderSnapshot};
use engine::paths::PathResolver;
use engine::process::ProcessScanner;
use engine::provider::Provider;
use engine::scheduler::{Scheduler, SchedulerConfig, SnapshotSink};
use engine::store::Store;
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::copilot::{CopilotProvider, SharedToken};
use providers::mock::MockProvider;
use providers::ProviderContext;

pub const MAIN_WINDOW: &str = "main";
pub const EVENT_SNAPSHOT_UPDATED: &str = "snapshot-updated";

struct TauriSink(AppHandle);

impl SnapshotSink for TauriSink {
    fn publish(&self, snapshot: &ProviderSnapshot) {
        let _ = self.0.emit(EVENT_SNAPSHOT_UPDATED, snapshot);
    }
}

/// `%LOCALAPPDATA%\<identifier>`; falls back to the temp dir if it cannot be created.
fn data_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_local_data_dir()
        .ok()
        .filter(|dir| std::fs::create_dir_all(dir).is_ok())
        .unwrap_or_else(std::env::temp_dir)
}

fn build_providers(ctx: &ProviderContext, copilot_token: SharedToken) -> Vec<Arc<dyn Provider>> {
    // `AIUM_MOCK=1` runs the UI against fake data.
    if std::env::var_os("AIUM_MOCK").is_some() {
        return ProviderId::ALL
            .into_iter()
            .map(|id| Arc::new(MockProvider::new(id, Duration::from_secs(15))) as Arc<dyn Provider>)
            .collect();
    }
    vec![
        Arc::new(ClaudeProvider::new(ctx.clone())),
        Arc::new(CodexProvider::new(ctx.clone())),
        Arc::new(CopilotProvider::new(ctx.clone(), copilot_token)),
    ]
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = data_dir(&handle);
            let store = Store::open(&data_dir.join("usage.sqlite"))
                .ok()
                .map(Arc::new);
            let ctx = ProviderContext {
                paths: Arc::new(PathResolver::new()),
                processes: Arc::new(ProcessScanner::new()),
                http: engine::http::client(),
                data_dir,
            };
            let scheduler = Arc::new(Scheduler::new(
                build_providers(&ctx, SharedToken::default()),
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

            app.manage(AppState {
                scheduler,
                processes: Arc::clone(&ctx.processes),
            });
            tray::create(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh,
            commands::debug_processes
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
