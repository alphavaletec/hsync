use tauri::State;

use crate::error::{AppError, AppResult};
use crate::jobs::manager::JobProgress;
use crate::jobs::model::{Job, JobInput, RunRecord};
use crate::jobs::selection::parse_managed_key;
use crate::jobs::RunOptions;
use crate::state::AppState;

fn find_job(state: &AppState, id: i64) -> AppResult<Job> {
    state
        .jobs
        .list_jobs()?
        .into_iter()
        .find(|j| j.id == id)
        .ok_or_else(|| AppError::invalid("Tarefa não encontrada"))
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>) -> AppResult<Vec<Job>> {
    state.jobs.list_jobs()
}

#[tauri::command]
pub async fn create_job(state: State<'_, AppState>, input: JobInput) -> AppResult<Job> {
    let input = input.normalize();
    state.jobs.validate_input(&input, None).await?;
    let id = state.store.insert_job(&input)?;
    state.jobs.emit_jobs_changed();
    find_job(&state, id)
}

#[tauri::command]
pub async fn update_job(state: State<'_, AppState>, id: i64, input: JobInput) -> AppResult<Job> {
    let input = input.normalize();
    let job = find_job(&state, id)?;
    if job.managed.is_some() {
        return Err(AppError::invalid("Esta tarefa é gerenciada pela aba Arquivos: altere-a por lá"));
    }
    if job.running {
        return Err(AppError::invalid("Aguarde a execução terminar para editar a tarefa"));
    }
    state.jobs.validate_input(&input, Some(id)).await?;
    state.store.update_job(id, &input)?;
    state.jobs.emit_jobs_changed();
    find_job(&state, id)
}

#[tauri::command]
pub async fn delete_job(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let job = find_job(&state, id)?;
    if job.running {
        return Err(AppError::invalid("Pare a execução antes de excluir a tarefa"));
    }
    // Tarefa da aba Arquivos: desmarca os itens daquele estado (senão ela seria recriada).
    if let Some((remote, item_state)) = job.managed.as_deref().and_then(parse_managed_key) {
        state.store.clear_selections(&remote, Some(item_state))?;
    }
    state.store.delete_job(id)?;
    state.jobs.remove_job_files(id);
    state.jobs.emit_jobs_changed();
    Ok(())
}

#[tauri::command]
pub async fn run_job(state: State<'_, AppState>, id: i64, options: Option<RunOptions>) -> AppResult<i64> {
    state.jobs.start(id, options.unwrap_or_default(), "manual").await
}

#[tauri::command]
pub async fn stop_job(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.jobs.stop(id).await
}

#[tauri::command]
pub async fn run_all_jobs(state: State<'_, AppState>) -> AppResult<()> {
    state.jobs.run_all_now().await;
    Ok(())
}

#[tauri::command]
pub async fn list_runs(state: State<'_, AppState>, job_id: Option<i64>, limit: Option<u32>) -> AppResult<Vec<RunRecord>> {
    state.store.list_runs(job_id, limit.unwrap_or(50).min(500))
}

#[tauri::command]
pub fn get_progress(state: State<'_, AppState>) -> Vec<JobProgress> {
    state.jobs.progress_snapshot()
}

#[tauri::command]
pub fn set_schedules_paused(app: tauri::AppHandle, state: State<'_, AppState>, paused: bool) -> AppResult<()> {
    state.jobs.set_paused(paused)?;
    crate::tray::sync_paused_item(&app, paused);
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    crate::tray::request_quit(&app);
}
