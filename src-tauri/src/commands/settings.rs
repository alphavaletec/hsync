//! Status do daemon, logs, desbloqueio, senha mestre e preferências.
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::rclone::daemon::LogLine;
use crate::rclone::DaemonStatus;
use crate::secret::Secret;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub bwlimit: String,
    pub schedules_paused: bool,
    pub encrypted: bool,
}

#[tauri::command]
pub fn daemon_status(state: State<'_, AppState>) -> DaemonStatus {
    state.daemon.status()
}

#[tauri::command]
pub fn daemon_logs(state: State<'_, AppState>) -> Vec<LogLine> {
    state.daemon.logs()
}

#[tauri::command]
pub fn daemon_retry(state: State<'_, AppState>) {
    state.daemon.retry();
}

#[tauri::command]
pub async fn unlock_config(state: State<'_, AppState>, password: Secret) -> AppResult<()> {
    if password.is_empty() {
        return Err(AppError::invalid("Informe a senha mestre"));
    }
    state.daemon.unlock(password).await
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    Ok(Settings {
        bwlimit: state.store.get_setting("bwlimit")?.unwrap_or_default(),
        schedules_paused: state.jobs.is_paused(),
        encrypted: state.daemon.is_config_encrypted(),
    })
}

/// Aceita vazio/"off" ou um valor como `512K`, `2M`, `1.5G` (bytes por segundo).
fn normalize_bwlimit(rate: &str) -> AppResult<String> {
    let r = rate.trim();
    if r.is_empty() || r.eq_ignore_ascii_case("off") || r == "0" {
        return Ok(String::new());
    }
    let (num, unit) = match r.char_indices().find(|(_, c)| c.is_ascii_alphabetic()) {
        Some((i, _)) => (&r[..i], &r[i..]),
        None => (r, ""),
    };
    let num_ok = num.replace(',', ".").parse::<f64>().is_ok_and(|n| n > 0.0);
    let unit_ok = matches!(unit.to_ascii_uppercase().as_str(), "" | "B" | "K" | "M" | "G" | "T");
    if !num_ok || !unit_ok {
        return Err(AppError::invalid("Limite inválido: use por exemplo 512K, 2M ou 1.5G"));
    }
    Ok(format!("{}{}", num.replace(',', "."), unit.to_ascii_uppercase()))
}

#[tauri::command]
pub async fn set_bwlimit(state: State<'_, AppState>, rate: String) -> AppResult<String> {
    let rate = normalize_bwlimit(&rate)?;
    state.daemon.set_bwlimit(rate.clone()).await?;
    state.store.set_setting("bwlimit", &rate)?;
    Ok(rate)
}

fn validate_new_password(p: &Secret) -> AppResult<()> {
    if p.expose().chars().count() < 8 {
        return Err(AppError::invalid("A senha mestre precisa ter pelo menos 8 caracteres"));
    }
    Ok(())
}

fn ensure_idle(state: &AppState) -> AppResult<()> {
    if state.jobs.has_active() {
        return Err(AppError::invalid("Aguarde as tarefas em execução terminarem"));
    }
    Ok(())
}

#[tauri::command]
pub async fn set_master_password(state: State<'_, AppState>, current: Option<Secret>, new: Secret) -> AppResult<()> {
    validate_new_password(&new)?;
    ensure_idle(&state)?;
    state.daemon.set_master_password(current, new).await
}

#[tauri::command]
pub async fn remove_master_password(state: State<'_, AppState>, current: Secret) -> AppResult<()> {
    ensure_idle(&state)?;
    state.daemon.remove_master_password(current).await
}

#[cfg(test)]
mod tests {
    use super::normalize_bwlimit;

    #[test]
    fn bwlimit_values() {
        assert_eq!(normalize_bwlimit("").unwrap(), "");
        assert_eq!(normalize_bwlimit("off").unwrap(), "");
        assert_eq!(normalize_bwlimit("2m").unwrap(), "2M");
        assert_eq!(normalize_bwlimit("1,5G").unwrap(), "1.5G");
        assert_eq!(normalize_bwlimit("512").unwrap(), "512");
        assert!(normalize_bwlimit("rápido").is_err());
        assert!(normalize_bwlimit("2X").is_err());
        assert!(normalize_bwlimit("-1M").is_err());
    }
}
