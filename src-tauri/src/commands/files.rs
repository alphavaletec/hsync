//! Aba Arquivos: navegação mesclando nuvem + cópia local, estados por item e transferências avulsas.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::jobs::model::{Endpoint, Job, MAX_INTERVAL_MINUTES, MIN_INTERVAL_MINUTES};
use crate::jobs::selection::{normalize_path, ItemState, Selection};
use crate::jobs::RunOptions;
use crate::rclone::filters::escape_glob;
use crate::state::AppState;

const DEFAULT_INTERVAL: u32 = 30;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: i64,
    pub mod_time: Option<String>,
    pub in_cloud: bool,
    pub local: bool,
    pub local_size: Option<i64>,
    /// Modificação da cópia local (ms desde a época).
    pub local_mod_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesConfig {
    pub remote: String,
    pub base: String,
    pub base_is_default: bool,
    pub interval_minutes: u32,
    pub selections: Vec<Selection>,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRef {
    pub path: String,
    pub is_dir: bool,
}

fn base_key(remote: &str) -> String {
    format!("files.base.{remote}")
}
fn interval_key(remote: &str) -> String {
    format!("files.interval.{remote}")
}

fn default_base(app: &AppHandle, remote: &str) -> AppResult<PathBuf> {
    let home = app.path().home_dir().map_err(|e| AppError::invalid(e.to_string()))?;
    // Nome de pasta seguro no Windows (o nome da conta pode ter espaço, +, @…).
    let safe: String = remote.chars().map(|c| if c.is_ascii_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();
    Ok(home.join("HSync").join(safe.trim()))
}

fn base_for(app: &AppHandle, state: &AppState, remote: &str) -> AppResult<(String, bool)> {
    match state.store.get_setting(&base_key(remote))? {
        Some(b) => Ok((b, false)),
        None => Ok((default_base(app, remote)?.to_string_lossy().into_owned(), true)),
    }
}

fn interval_for(state: &AppState, remote: &str) -> AppResult<u32> {
    Ok(state
        .store
        .get_setting(&interval_key(remote))?
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL))
}

async fn ensure_remote(state: &AppState, remote: &str) -> AppResult<()> {
    let client = state.daemon.client()?;
    if client.list_remotes().await?.iter().any(|r| r == remote) {
        Ok(())
    } else {
        Err(AppError::invalid(format!("A conta \"{remote}\" não existe")))
    }
}

fn ensure_idle(state: &AppState, remote: &str) -> AppResult<()> {
    if state.jobs.managed_job_ids(remote)?.into_iter().any(|id| state.jobs.is_running(id)) {
        return Err(AppError::invalid("Aguarde a sincronização desta conta terminar (ou pare-a no painel)"));
    }
    Ok(())
}

fn build_config(app: &AppHandle, state: &AppState, remote: &str) -> AppResult<FilesConfig> {
    let (base, base_is_default) = base_for(app, state, remote)?;
    let ids = state.jobs.managed_job_ids(remote)?;
    let jobs = state.jobs.list_jobs()?.into_iter().filter(|j| ids.contains(&j.id)).collect();
    Ok(FilesConfig {
        remote: remote.to_string(),
        base,
        base_is_default,
        interval_minutes: interval_for(state, remote)?,
        selections: state.store.list_selections(remote)?,
        jobs,
    })
}

/// Recompila as tarefas da conta e já dispara as que mudaram.
async fn apply_and_run(app: &AppHandle, state: &AppState, remote: &str) -> AppResult<()> {
    let (base, _) = base_for(app, state, remote)?;
    let interval = interval_for(state, remote)?;
    let changed = state.jobs.apply_selection(remote, &base, interval)?;
    for id in changed {
        if let Err(e) = state.jobs.start(id, RunOptions::default(), "selection").await {
            log::warn!("tarefa gerenciada {id} não iniciou: {e}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn files_config(app: AppHandle, state: State<'_, AppState>, remote: String) -> AppResult<FilesConfig> {
    build_config(&app, &state, &remote)
}

#[tauri::command]
pub async fn browse(app: AppHandle, state: State<'_, AppState>, remote: String, path: String) -> AppResult<Vec<FileEntry>> {
    ensure_remote(&state, &remote).await?;
    let rel = if path.trim_matches(['/', '\\']).is_empty() {
        String::new()
    } else {
        normalize_path(&path).ok_or_else(|| AppError::invalid("Caminho inválido"))?
    };
    let client = state.daemon.client()?;
    let listing = client
        .call_timeout(
            "operations/list",
            json!({ "fs": format!("{remote}:"), "remote": rel, "opt": { "noMimeType": true } }),
            Some(Duration::from_secs(120)),
        )
        .await;
    // Pasta que só existe localmente (ainda não enviada) não é erro.
    let cloud_items = match listing {
        Ok(v) => v.get("list").and_then(Value::as_array).cloned().unwrap_or_default(),
        Err(e) if e.to_string().contains("directory not found") => Vec::new(),
        Err(e) => return Err(e),
    };

    let mut entries: BTreeMap<String, FileEntry> = BTreeMap::new();
    for it in cloud_items {
        let name = it.get("Name").and_then(Value::as_str).unwrap_or_default().to_string();
        if name.is_empty() {
            continue;
        }
        entries.insert(
            name.clone(),
            FileEntry {
                path: join_rel(&rel, &name),
                name,
                is_dir: it.get("IsDir").and_then(Value::as_bool).unwrap_or(false),
                size: it.get("Size").and_then(Value::as_i64).unwrap_or(0),
                mod_time: it.get("ModTime").and_then(Value::as_str).map(String::from),
                in_cloud: true,
                local: false,
                local_size: None,
                local_mod_ms: None,
            },
        );
    }

    let (base, _) = base_for(&app, &state, &remote)?;
    let local_dir = rel.split('/').filter(|s| !s.is_empty()).fold(PathBuf::from(&base), |p, s| p.join(s));
    if let Ok(rd) = std::fs::read_dir(&local_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(meta) = e.metadata() else { continue };
            let size = if meta.is_dir() { -1 } else { meta.len() as i64 };
            let local_mod_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            match entries.get_mut(&name) {
                Some(entry) => {
                    entry.local = true;
                    entry.local_size = Some(size);
                    entry.local_mod_ms = local_mod_ms;
                }
                None => {
                    entries.insert(
                        name.clone(),
                        FileEntry {
                            path: join_rel(&rel, &name),
                            name,
                            is_dir: meta.is_dir(),
                            size: size.max(0),
                            mod_time: None,
                            in_cloud: false,
                            local: true,
                            local_size: Some(size),
                            local_mod_ms,
                        },
                    );
                }
            }
        }
    }

    let mut list: Vec<FileEntry> = entries.into_values().collect();
    list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(list)
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

#[tauri::command]
pub async fn set_item_state(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: String,
    path: String,
    is_dir: bool,
    item_state: Option<ItemState>,
) -> AppResult<FilesConfig> {
    ensure_remote(&state, &remote).await?;
    let path = normalize_path(&path).ok_or_else(|| AppError::invalid("Escolha um arquivo ou pasta (não a raiz)"))?;
    ensure_idle(&state, &remote)?;
    state.store.set_selection(&remote, &path, is_dir, item_state)?;
    apply_and_run(&app, &state, &remote).await?;
    build_config(&app, &state, &remote)
}

#[tauri::command]
pub async fn set_files_base(app: AppHandle, state: State<'_, AppState>, remote: String, base: String) -> AppResult<FilesConfig> {
    let base = base.trim().to_string();
    if !matches!(Endpoint::parse(&base), Some(Endpoint::Local(_))) || !Path::new(&base).is_absolute() {
        return Err(AppError::invalid("Escolha uma pasta local (caminho absoluto)"));
    }
    ensure_idle(&state, &remote)?;
    state.store.set_setting(&base_key(&remote), &base)?;
    apply_and_run(&app, &state, &remote).await?;
    build_config(&app, &state, &remote)
}

#[tauri::command]
pub async fn set_files_interval(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: String,
    minutes: u32,
) -> AppResult<FilesConfig> {
    if minutes != 0 && !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&minutes) {
        return Err(AppError::invalid("Intervalo inválido"));
    }
    ensure_idle(&state, &remote)?;
    state.store.set_setting(&interval_key(&remote), &minutes.to_string())?;
    let (base, _) = base_for(&app, &state, &remote)?;
    state.jobs.apply_selection(&remote, &base, minutes)?;
    build_config(&app, &state, &remote)
}

#[tauri::command]
pub async fn sync_account_now(state: State<'_, AppState>, remote: String) -> AppResult<()> {
    let ids = state.jobs.managed_job_ids(&remote)?;
    if ids.is_empty() {
        return Err(AppError::invalid("Nenhum item marcado nesta conta"));
    }
    for id in ids {
        if !state.jobs.is_running(id) {
            state.jobs.start(id, RunOptions::default(), "manual").await?;
        }
    }
    Ok(())
}

fn include_rules(items: &[ItemRef]) -> AppResult<Vec<String>> {
    let mut rules = Vec::new();
    for it in items {
        let p = normalize_path(&it.path).ok_or_else(|| AppError::invalid("Caminho inválido"))?;
        let escaped: Vec<String> = p.split('/').map(escape_glob).collect();
        let pat = format!("/{}", escaped.join("/"));
        rules.push(if it.is_dir { format!("+ {pat}/**") } else { format!("+ {pat}") });
    }
    rules.push("- **".into());
    Ok(rules)
}

/// Baixa agora (cópia avulsa) os itens escolhidos para `target`, mantendo a estrutura de pastas.
#[tauri::command]
pub async fn download_items(
    state: State<'_, AppState>,
    remote: String,
    items: Vec<ItemRef>,
    target: String,
) -> AppResult<i64> {
    if items.is_empty() {
        return Err(AppError::invalid("Nada selecionado"));
    }
    if !matches!(Endpoint::parse(&target), Some(Endpoint::Local(_))) {
        return Err(AppError::invalid("Destino precisa ser uma pasta local"));
    }
    ensure_remote(&state, &remote).await?;
    let params = json!({
        "srcFs": format!("{remote}:"),
        "dstFs": target,
        "_filter": { "FilterRule": include_rules(&items)? },
    });
    let label = format!("baixar {} item(ns) de {remote}:", items.len());
    state.jobs.start_adhoc(label, "sync/copy", params).await
}

/// Envia agora arquivos/pastas locais (de uma mesma pasta de origem) para `remote:dir`.
#[tauri::command]
pub async fn upload_items(
    state: State<'_, AppState>,
    remote: String,
    dir: String,
    local_paths: Vec<String>,
) -> AppResult<i64> {
    let first = local_paths.first().ok_or_else(|| AppError::invalid("Nada selecionado"))?;
    let parent = Path::new(first).parent().ok_or_else(|| AppError::invalid("Caminho inválido"))?.to_path_buf();
    let mut items = Vec::new();
    for p in &local_paths {
        let path = Path::new(p);
        if path.parent() != Some(parent.as_path()) {
            return Err(AppError::invalid("Envie itens de uma mesma pasta por vez"));
        }
        let name = path.file_name().ok_or_else(|| AppError::invalid("Caminho inválido"))?.to_string_lossy().into_owned();
        items.push(ItemRef { path: name, is_dir: path.is_dir() });
    }
    ensure_remote(&state, &remote).await?;
    let dir = dir.trim_matches(['/', '\\']).to_string();
    let params = json!({
        "srcFs": parent.to_string_lossy(),
        "dstFs": format!("{remote}:{dir}"),
        "_filter": { "FilterRule": include_rules(&items)? },
    });
    let label = format!("enviar {} item(ns) para {remote}:{dir}", items.len());
    state.jobs.start_adhoc(label, "sync/copy", params).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_rules_for_items() {
        let r = include_rules(&[
            ItemRef { path: "Fotos/2024".into(), is_dir: true },
            ItemRef { path: "/a*.txt".into(), is_dir: false },
        ])
        .unwrap();
        assert_eq!(r, vec!["+ /Fotos/2024/**", "+ /a\\*.txt", "- **"]);
        assert!(include_rules(&[ItemRef { path: "../x".into(), is_dir: false }]).is_err());
    }
}
