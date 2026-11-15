// The directory keeps the planned name `core/`, but the module is called
// `engine` so it can never shadow the built-in `core` crate.
#[path = "core/mod.rs"]
pub mod engine;
pub mod providers;

mod commands;
mod tray;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use commands::AppState;
use engine::model::{ProviderId, ProviderSnapshot};
use engine::notify::Notifier;
use engine::paths::PathResolver;
use engine::process::ProcessScanner;
use engine::provider::Provider;
use engine::scheduler::{Scheduler, SchedulerConfig, SnapshotSink};
use engine::settings::Settings;
use engine::store::Store;
use providers::antigravity::AntigravityProvider;
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::copilot::{CopilotProvider, SharedToken};
use providers::mock::MockProvider;
use providers::{copilot_auth, ProviderContext};

pub const MAIN_WINDOW: &str = "main";
pub const EVENT_SNAPSHOT_UPDATED: &str = "snapshot-updated";
/// Passed by the autostart entry so the app starts in the tray.
const ARG_MINIMIZED: &str = "--minimized";

/// Forwards snapshots to the UI, the tray tooltip and the notification rules.
struct TauriSink {
    app: AppHandle,
    settings: Arc<RwLock<Settings>>,
    notifier: Mutex<Notifier>,
    data_dir: PathBuf,
}

impl TauriSink {
    fn notify(&self, snapshot: &ProviderSnapshot) {
        let settings = self
            .settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let alerts = {
            let mut notifier = self.notifier.lock().unwrap_or_else(|e| e.into_inner());
            let before = notifier.clone();
            let alerts = notifier.evaluate(
                snapshot,
                settings.warn_threshold,
                settings.critical_threshold,
            );
            if *notifier != before {
                notifier.save(&self.data_dir);
            }
            alerts
        };
        if !settings.notifications_enabled {
            return;
        }
        for alert in alerts {
            let _ = self
                .app
                .notification()
                .builder()
                .title(alert.title())
                .body(alert.body())
                .show();
        }
    }
}

impl SnapshotSink for TauriSink {
    fn publish(&self, snapshot: &ProviderSnapshot) {
        let _ = self.app.emit(EVENT_SNAPSHOT_UPDATED, snapshot);
        if let Some(state) = self.app.try_state::<AppState>() {
            tray::update_tooltip(&self.app, &state.scheduler.snapshots());
        }
        self.notify(snapshot);
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
        Arc::new(AntigravityProvider::new(ctx.clone())),
        Arc::new(CopilotProvider::new(ctx.clone(), copilot_token)),
    ]
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![ARG_MINIMIZED]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = data_dir(&handle);
            let settings = Arc::new(RwLock::new(Settings::load(&data_dir)));
            let store = Store::open(&data_dir.join("usage.sqlite"))
                .ok()
                .map(Arc::new);
            let ctx = ProviderContext {
                paths: Arc::new(PathResolver::new()),
                processes: Arc::new(ProcessScanner::new()),
                http: engine::http::client(),
                data_dir: data_dir.clone(),
            };
            let copilot_token: SharedToken = Arc::new(RwLock::new(copilot_auth::load_token()));

            let scheduler = Arc::new(Scheduler::new(
                build_providers(&ctx, Arc::clone(&copilot_token)),
                SchedulerConfig::default(),
                store,
                Arc::new(TauriSink {
                    app: handle.clone(),
                    settings: Arc::clone(&settings),
                    notifier: Mutex::new(Notifier::load(&data_dir)),
                    data_dir: data_dir.clone(),
                }),
            ));

            let state = AppState {
                scheduler: Arc::clone(&scheduler),
                processes: Arc::clone(&ctx.processes),
                paths: Arc::clone(&ctx.paths),
                settings,
                copilot_token,
                http: ctx.http.clone(),
                data_dir,
            };
            // Settings must be in place before the first poll.
            commands::apply_settings(&handle, &state, &state.settings());
            app.manage(state);

            // `start` spawns tokio tasks, so it must run inside the runtime.
            let starter = Arc::clone(&scheduler);
            tauri::async_runtime::block_on(async move {
                starter.start();
            });

            let refresher = Arc::clone(&scheduler);
            handle.listen(tray::EVENT_REFRESH_REQUESTED, move |_event| {
                refresher.refresh(None);
            });

            tray::create(&handle)?;
            tray::create_popup(&handle)?;
            if !std::env::args().any(|arg| arg == ARG_MINIMIZED) {
                tray::show_dashboard(&handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::refresh,
            commands::show_dashboard,
            commands::get_settings,
            commands::set_settings,
            commands::get_history,
            commands::copilot_status,
            commands::copilot_connect,
            commands::copilot_open_verification,
            commands::copilot_disconnect,
            commands::debug_processes
        ])
        .on_window_event(|window, event| {
            match (window.label(), event) {
                // Closing the dashboard hides it to the tray instead of quitting.
                (MAIN_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // The tray popup hides itself as soon as it loses focus.
                (tray::POPUP_WINDOW, WindowEvent::Focused(false)) => {
                    tray::hide_popup(window.app_handle());
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
