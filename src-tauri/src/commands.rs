//! Tauri commands exposed to the UI.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;

use crate::engine::cmd::hidden_command;
use crate::engine::model::{ProviderId, ProviderSnapshot};
use crate::engine::paths::{PathResolver, Tool};
use crate::engine::process::{listening_sockets, ports_of, ProcessScanner, ToolKind};
use crate::engine::scheduler::{ProviderSettings, Scheduler};
use crate::engine::settings::Settings;
use crate::engine::store::HistoryPoint;
use crate::providers::copilot::SharedToken;
use crate::providers::copilot_auth::{self, DeviceCode, PollOutcome};
use crate::tray;

pub const EVENT_COPILOT_AUTH: &str = "copilot-auth";
pub const EVENT_SETTINGS_CHANGED: &str = "settings-changed";
const MAX_HISTORY_DAYS: i64 = 30;

pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub processes: Arc<ProcessScanner>,
    pub paths: Arc<PathResolver>,
    pub settings: Arc<RwLock<Settings>>,
    pub copilot_token: SharedToken,
    pub http: reqwest::Client,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Pushes settings into the running services. Nothing here needs a restart.
pub fn apply_settings(app: &AppHandle, state: &AppState, settings: &Settings) {
    for id in ProviderId::ALL {
        state.scheduler.set_provider_settings(
            id,
            ProviderSettings {
                enabled: settings.prefs(id).enabled,
                poll_interval: settings.poll_interval(id),
            },
        );
    }
    state.paths.set_manual(
        Tool::Claude,
        settings.claude_dir.as_ref().map(PathBuf::from),
    );
    state
        .paths
        .set_manual(Tool::Codex, settings.codex_dir.as_ref().map(PathBuf::from));

    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false) != settings.autostart {
        let _ = if settings.autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
    }
}

#[tauri::command]
pub fn get_snapshots(state: State<'_, AppState>) -> Vec<ProviderSnapshot> {
    state.scheduler.snapshots()
}

#[tauri::command]
pub fn refresh(state: State<'_, AppState>, provider: Option<ProviderId>) {
    state.scheduler.refresh(provider);
}

/// Opens the dashboard (used by the tray popup).
#[tauri::command]
pub fn show_dashboard(app: AppHandle) {
    tray::hide_popup(&app);
    tray::show_dashboard(&app);
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Validates, saves and applies the settings; returns what was actually stored.
#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let settings = settings.sanitized();
    settings
        .save(&state.data_dir)
        .map_err(|_| "could not write the settings file".to_string())?;
    *state.settings.write().unwrap_or_else(|e| e.into_inner()) = settings.clone();
    apply_settings(&app, &state, &settings);
    let _ = app.emit(EVENT_SETTINGS_CHANGED, &settings);
    Ok(settings)
}

#[tauri::command]
pub fn get_history(
    state: State<'_, AppState>,
    provider: ProviderId,
    days: i64,
) -> Result<Vec<HistoryPoint>, String> {
    let Some(store) = state.scheduler.store() else {
        return Ok(Vec::new());
    };
    let since = Utc::now() - chrono::Duration::days(days.clamp(1, MAX_HISTORY_DAYS));
    store
        .history(provider, since)
        .map_err(|_| "could not read the usage history".to_string())
}

#[derive(Debug, Serialize)]
pub struct CopilotStatus {
    /// True when the app holds its own GitHub token (from the device flow).
    pub connected: bool,
}

#[tauri::command]
pub fn copilot_status(state: State<'_, AppState>) -> CopilotStatus {
    CopilotStatus {
        connected: state
            .copilot_token
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some(),
    }
}

#[derive(Clone, Serialize)]
struct CopilotAuthEvent {
    /// "connected" or "failed"
    state: &'static str,
    message: Option<String>,
}

/// Starts the GitHub device flow. Returns the code the user must enter on
/// github.com; the result arrives later as a `copilot-auth` event.
#[tauri::command]
pub async fn copilot_connect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DeviceCode, String> {
    let client = state.http.clone();
    let code = copilot_auth::request_device_code(&client)
        .await
        .map_err(|error| error.to_string())?;

    let token_slot = Arc::clone(&state.copilot_token);
    let scheduler = Arc::clone(&state.scheduler);
    let pending = code.clone();
    tauri::async_runtime::spawn(async move {
        let deadline = Instant::now() + Duration::from_secs(pending.expires_in.min(1800));
        let mut interval = Duration::from_secs(pending.interval);
        let outcome = loop {
            tokio::time::sleep(interval).await;
            if Instant::now() >= deadline {
                break PollOutcome::Failed("The code expired. Try again.".to_string());
            }
            match copilot_auth::poll_token(&client, &pending.device_code).await {
                PollOutcome::Pending => {}
                PollOutcome::SlowDown => interval += Duration::from_secs(5),
                done => break done,
            }
        };
        let event = match outcome {
            PollOutcome::Token(token) => match copilot_auth::store_token(&token) {
                Ok(()) => {
                    *token_slot.write().unwrap_or_else(|e| e.into_inner()) = Some(token);
                    scheduler.refresh(Some(ProviderId::Copilot));
                    CopilotAuthEvent {
                        state: "connected",
                        message: None,
                    }
                }
                Err(message) => CopilotAuthEvent {
                    state: "failed",
                    message: Some(message),
                },
            },
            PollOutcome::Failed(message) => CopilotAuthEvent {
                state: "failed",
                message: Some(message),
            },
            PollOutcome::Pending | PollOutcome::SlowDown => return,
        };
        let _ = app.emit(EVENT_COPILOT_AUTH, event);
    });
    Ok(code)
}

/// Opens GitHub's device page in the default browser (fixed URL).
#[tauri::command]
pub fn copilot_open_verification() -> Result<(), String> {
    hidden_command("explorer.exe")
        .arg(copilot_auth::VERIFICATION_URL)
        .spawn()
        .map(|_| ())
        .map_err(|_| "could not open the browser".to_string())
}

#[tauri::command]
pub fn copilot_disconnect(state: State<'_, AppState>) {
    copilot_auth::delete_token();
    *state
        .copilot_token
        .write()
        .unwrap_or_else(|e| e.into_inner()) = None;
    state.scheduler.refresh(Some(ProviderId::Copilot));
}

/// What the UI may see of a process. The command line is deliberately left
/// out because it can contain secrets (CSRF tokens).
#[derive(Debug, Serialize)]
pub struct ProcessDebugInfo {
    pub kind: ToolKind,
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub exe: Option<String>,
    pub arg_count: usize,
    pub start_time: Option<DateTime<Utc>>,
    pub listening_ports: Vec<u16>,
}

#[tauri::command]
pub async fn debug_processes(state: State<'_, AppState>) -> Result<Vec<ProcessDebugInfo>, String> {
    let scanner = Arc::clone(&state.processes);
    tauri::async_runtime::spawn_blocking(move || {
        let sockets = listening_sockets();
        scanner
            .scan()
            .iter()
            .map(|process| ProcessDebugInfo {
                kind: process.kind,
                pid: process.info.pid,
                parent_pid: process.info.parent_pid,
                name: process.info.name.clone(),
                exe: process.info.exe.clone(),
                arg_count: process.info.cmdline.len(),
                start_time: process.info.start_time,
                listening_ports: ports_of(&sockets, process.info.pid),
            })
            .collect()
    })
    .await
    .map_err(|error| error.to_string())
}
