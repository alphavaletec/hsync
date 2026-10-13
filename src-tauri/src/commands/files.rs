//! Aba Arquivos: navegação mesclando nuvem + cópia local, estados por item e transferências avulsas.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::jobs::model::{Endpoint, Job, MAX_INTERVAL_MINUTES, MIN_INTERVAL_MINUTES};
use crate::jobs::selection::{effective_state, keeps_local_copy, normalize_path, ItemState, Selection};
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

pub(crate) fn base_key(remote: &str) -> String {
    format!("files.base.{remote}")
}
fn interval_key(remote: &str) -> String {
    format!("files.interval.{remote}")
}

pub(crate) fn default_base(app: &AppHandle, remote: &str) -> AppResult<PathBuf> {
    let home = app.path().home_dir().map_err(|e| AppError::invalid(e.to_string()))?;
    // Nome de pasta seguro no Windows (o nome da conta pode ter espaço, +, @…).
    let safe: String = remote.chars().map(|c| if c.is_ascii_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();
    Ok(home.join("HSync").join(safe.trim()))
}

pub(crate) fn base_for(app: &AppHandle, state: &AppState, remote: &str) -> AppResult<(String, bool)> {
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

pub(crate) async fn ensure_remote(state: &AppState, remote: &str) -> AppResult<()> {
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
) -> AppResult<SetStateResult> {
    ensure_remote(&state, &remote).await?;
    let path = normalize_path(&path).ok_or_else(|| AppError::invalid("Escolha um arquivo ou pasta (não a raiz)"))?;
    // A mudança vale na hora: uma sincronização em andamento desta conta é interrompida
    // (ela é refeita em seguida com a nova seleção).
    stop_account_jobs(&state, &remote).await?;

    let before = effective_state(&state.store.list_selections(&remote)?, &path);
    state.store.set_selection(&remote, &path, is_dir, item_state)?;
    let after = effective_state(&state.store.list_selections(&remote)?, &path);

    // Deixou de ter cópia local (ex.: "manter offline" → "nenhum"): libera o espaço.
    // Não vale para ↑ só enviar (os arquivos locais são os originais) nem para ⊘ ignorar
    // (o usuário só quer tirar o item da sincronização).
    let notice = if keeps_local_copy(before) && after.is_none() {
        free_local_copy(&app, &state, &remote, &path, is_dir).await?
    } else {
        None
    };

    apply_and_run(&app, &state, &remote).await?;
    Ok(SetStateResult { config: build_config(&app, &state, &remote)?, notice })
}

/// "Liberar espaço": manda para a Lixeira a cópia local dos itens escolhidos que não estão
/// marcados para ficar offline (com as mesmas garantias de `free_local_copy`).
#[tauri::command]
pub async fn free_space(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: String,
    items: Vec<ItemRef>,
) -> AppResult<Vec<String>> {
    ensure_remote(&state, &remote).await?;
    let selections = state.store.list_selections(&remote)?;
    let mut notices = Vec::new();
    for it in items {
        let Some(rel) = normalize_path(&it.path) else { continue };
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let current = effective_state(&selections, &rel);
        if keeps_local_copy(current) {
            notices.push(format!(
                "\"{name}\" está marcado para ficar neste PC; mude o estado para \"nenhum\" para liberar o espaço."
            ));
            continue;
        }
        if current == Some(ItemState::Upload) {
            notices.push(format!("\"{name}\" é \"só enviar\": os arquivos locais são os originais e não são removidos."));
            continue;
        }
        if let Some(n) = free_local_copy(&app, &state, &remote, &rel, it.is_dir).await? {
            notices.push(n);
        }
    }
    Ok(notices)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStateResult {
    pub config: FilesConfig,
    /// Mensagem sobre a cópia local (liberada ou mantida e por quê).
    pub notice: Option<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum LocalVsCloud {
    Same,
    Different { only_local: usize, differ: usize },
}

/// Tudo que existe em `local` (arquivo ou pasta) também existe, igual, na nuvem?
/// `cloud_fs` é a pasta na nuvem correspondente à pasta `local` ou, para arquivo, à pasta-mãe dele.
pub(crate) async fn local_copy_in_cloud(
    client: &crate::rclone::RcClient,
    local: &Path,
    cloud_fs: &str,
    is_dir: bool,
) -> AppResult<LocalVsCloud> {
    // operations/check não aceita arquivo como raiz: para arquivo, compara a pasta-mãe
    // filtrando só aquele nome.
    let params = if is_dir {
        json!({ "srcFs": local.to_string_lossy(), "dstFs": cloud_fs, "oneWay": true })
    } else {
        let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        json!({
            "srcFs": local.parent().unwrap_or(local).to_string_lossy(),
            "dstFs": cloud_fs,
            "oneWay": true,
            "_filter": { "FilterRule": [format!("+ /{}", escape_glob(&name)), "- **"] },
        })
    };
    let report = client.call_timeout("operations/check", params, Some(Duration::from_secs(600))).await?;
    if report.get("success").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(LocalVsCloud::Same);
    }
    let count = |k: &str| report.get(k).and_then(Value::as_array).map_or(0, Vec::len);
    Ok(LocalVsCloud::Different { only_local: count("missingOnDst"), differ: count("differ") })
}

/// Interrompe as tarefas gerenciadas da conta e espera o monitor registrar o fim.
async fn stop_account_jobs(state: &AppState, remote: &str) -> AppResult<()> {
    let running: Vec<i64> =
        state.jobs.managed_job_ids(remote)?.into_iter().filter(|id| state.jobs.is_running(*id)).collect();
    for id in &running {
        let _ = state.jobs.stop(*id).await;
    }
    for _ in 0..60 {
        if !running.iter().any(|id| state.jobs.is_running(*id)) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(AppError::invalid("A sincronização desta conta não parou a tempo; tente novamente"))
}

/// Manda a cópia local para a Lixeira, mas só se tudo nela também estiver na nuvem
/// (nada que exista só neste PC ou esteja diferente é descartado).
async fn free_local_copy(
    app: &AppHandle,
    state: &AppState,
    remote: &str,
    rel: &str,
    is_dir: bool,
) -> AppResult<Option<String>> {
    let (base, _) = base_for(app, state, remote)?;
    let local = rel.split('/').fold(PathBuf::from(&base), |p, s| p.join(s));
    if !local.exists() {
        return Ok(None);
    }
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();

    let client = state.daemon.client()?;
    let cloud_parent = format!("{remote}:{}", rel.rsplit_once('/').map(|(p, _)| p).unwrap_or(""));
    let cloud_item = format!("{remote}:{rel}");
    match local_copy_in_cloud(&client, &local, if is_dir { &cloud_item } else { &cloud_parent }, is_dir).await {
        Ok(LocalVsCloud::Same) => {}
        Ok(LocalVsCloud::Different { only_local, differ }) => {
            return Ok(Some(format!(
                "Mantive a cópia local de \"{name}\": {only_local} arquivo(s) só existem neste PC e {differ} estão \
                 diferentes da nuvem. Envie-os ou apague-os manualmente se quiser liberar o espaço."
            )))
        }
        Err(e) => {
            return Ok(Some(format!(
                "Mantive a cópia local de \"{name}\": não foi possível conferir com a nuvem ({e})."
            )))
        }
    }

    let target = local.clone();
    tauri::async_runtime::spawn_blocking(move || trash::delete(&target))
        .await
        .map_err(|e| AppError::invalid(e.to_string()))?
        .map_err(|e| AppError::invalid(format!("Não foi possível mover \"{name}\" para a Lixeira: {e}")))?;
    Ok(Some(format!("Cópia local de \"{name}\" enviada para a Lixeira. O item continua disponível na nuvem.")))
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
