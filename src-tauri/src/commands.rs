//! Tauri commands exposed to the UI.

use std::sync::Arc;

use tauri::State;

use crate::engine::model::{ProviderId, ProviderSnapshot};
use crate::engine::scheduler::Scheduler;

pub struct AppState {
    pub scheduler: Arc<Scheduler>,
}

#[tauri::command]
pub fn get_snapshots(state: State<'_, AppState>) -> Vec<ProviderSnapshot> {
    state.scheduler.snapshots()
}

#[tauri::command]
pub fn refresh(state: State<'_, AppState>, provider: Option<ProviderId>) {
    state.scheduler.refresh(provider);
}
