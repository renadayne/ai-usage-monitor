//! Tauri commands exposed to the UI.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::engine::model::{ProviderId, ProviderSnapshot};
use crate::engine::process::{listening_sockets, ports_of, ProcessScanner, ToolKind};
use crate::engine::scheduler::Scheduler;
use crate::tray;

pub struct AppState {
    pub scheduler: Arc<Scheduler>,
    pub processes: Arc<ProcessScanner>,
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
