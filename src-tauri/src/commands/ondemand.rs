use tauri::{AppHandle, State};

use super::files::ensure_remote;
use crate::error::AppResult;
use crate::ondemand::OnDemandStatus;
use crate::state::AppState;

#[tauri::command]
pub fn ondemand_status(state: State<'_, AppState>, remote: String) -> AppResult<OnDemandStatus> {
    state.ondemand.status(&remote)
}

#[tauri::command]
pub async fn enable_ondemand(app: AppHandle, state: State<'_, AppState>, remote: String) -> AppResult<OnDemandStatus> {
    ensure_remote(&state, &remote).await?;
    state.ondemand.enable(&app, &remote)
}

#[tauri::command]
pub fn disable_ondemand(state: State<'_, AppState>, remote: String) -> AppResult<()> {
    state.ondemand.disable(&remote)
}
