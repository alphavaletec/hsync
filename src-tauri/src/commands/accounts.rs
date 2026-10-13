//! Contas (remotes do rclone): provedores, wizard com OAuth, teste de conexão e navegação.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::rclone::client::clean_error;
use crate::rclone::RcClient;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub kind: String,
    pub has_custom_client: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderExample {
    pub value: String,
    pub help: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOption {
    pub name: String,
    pub help: String,
    pub kind: String,
    pub required: bool,
    pub is_password: bool,
    pub default: String,
    pub examples: Vec<ProviderExample>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub name: String,
    pub description: String,
    pub options: Vec<ProviderOption>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRemoteInput {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
}

/// Próximo passo do wizard de configuração.
#[derive(Debug, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum ConfigStep {
    Done,
    /// Pergunta do rclone a ser exibida (repassamos o objeto `Option` original).
    #[serde(rename_all = "camelCase")]
    Question { state: String, option: Value, error: Option<String> },
    /// Autorização OAuth em andamento no navegador; acompanhar com `oauth_progress`.
    #[serde(rename_all = "camelCase")]
    OAuth { job_id: i64 },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthProgress {
    pub finished: bool,
    pub auth_url: Option<String>,
    pub next: Option<ConfigStep>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTest {
    pub ok: bool,
    pub error: Option<String>,
    pub info: Option<Value>,
    pub about: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
}

fn validate_remote_name(name: &str) -> AppResult<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['-', ' '])
        && !name.ends_with(' ')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.+@ ".contains(c));
    if ok {
        Ok(())
    } else {
        Err(AppError::invalid(
            "Nome inválido: use letras, números, espaço e _ - . + @ (sem começar com - ou espaço)",
        ))
    }
}

async fn ensure_remote(client: &RcClient, name: &str) -> AppResult<()> {
    if client.list_remotes().await?.iter().any(|r| r == name) {
        Ok(())
    } else {
        Err(AppError::invalid(format!("A conta \"{name}\" não existe")))
    }
}

#[tauri::command]
pub async fn list_remotes(state: State<'_, AppState>) -> AppResult<Vec<Remote>> {
    let client = state.daemon.client()?;
    let dump = client.config_dump().await?;
    let pending = state.pending_remotes.lock().unwrap().clone();
    let mut remotes: Vec<Remote> = dump
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(name, _)| !pending.contains(*name))
                // Só nome e tipo saem do backend: tokens e segredos nunca chegam ao webview.
                .map(|(name, cfg)| Remote {
                    name: name.clone(),
                    kind: cfg.get("type").and_then(Value::as_str).unwrap_or_default().to_string(),
                    has_custom_client: cfg.get("client_id").and_then(Value::as_str).is_some_and(|s| !s.is_empty()),
                })
                .collect()
        })
        .unwrap_or_default();
    remotes.sort_by_key(|r| r.name.to_lowercase());
    Ok(remotes)
}

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> AppResult<Vec<Provider>> {
    let client = state.daemon.client()?;
    let v = client.providers().await?;
    let list = v.get("providers").and_then(Value::as_array).cloned().unwrap_or_default();
    let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut providers: Vec<Provider> = list
        .iter()
        .map(|p| Provider {
            name: s(p, "Name"),
            description: s(p, "Description"),
            options: p
                .get("Options")
                .and_then(Value::as_array)
                .map(|opts| {
                    opts.iter()
                        .filter(|o| !o.get("Advanced").and_then(Value::as_bool).unwrap_or(false))
                        .filter(|o| o.get("Hide").and_then(Value::as_i64).unwrap_or(0) == 0)
                        .map(|o| ProviderOption {
                            name: s(o, "Name"),
                            help: s(o, "Help"),
                            kind: s(o, "Type"),
                            required: o.get("Required").and_then(Value::as_bool).unwrap_or(false),
                            is_password: o.get("IsPassword").and_then(Value::as_bool).unwrap_or(false),
                            default: s(o, "DefaultStr"),
                            examples: o
                                .get("Examples")
                                .and_then(Value::as_array)
                                .map(|ex| {
                                    ex.iter()
                                        .map(|e| ProviderExample { value: s(e, "Value"), help: s(e, "Help") })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    providers.sort_by_key(|p| p.description.to_lowercase());
    Ok(providers)
}

/// Interpreta a resposta da máquina de estados de `config/create`, respondendo
/// automaticamente as perguntas que o app já resolve sozinho.
async fn advance(state: &AppState, client: &RcClient, name: &str, kind: &str, mut res: Value) -> AppResult<ConfigStep> {
    for _ in 0..20 {
        let out = RcClient::parse_config_output(res.clone())?;
        if out.state.is_empty() {
            state.pending_remotes.lock().unwrap().remove(name);
            return Ok(ConfigStep::Done);
        }
        let option = out.option.unwrap_or(Value::Null);
        let opt_name = option.get("Name").and_then(Value::as_str).unwrap_or_default();
        match opt_name {
            // Fluxo OAuth com servidor local do próprio rclone (redirect para 127.0.0.1:53682).
            "config_is_local" => {
                let job_id = client.call_async("config/create", continue_params(name, kind, &out.state, "true")).await?;
                return Ok(ConfigStep::OAuth { job_id });
            }
            // O wizard já avisou sobre o client_id compartilhado do Google estar sendo descontinuado.
            "config_shared_client_id" => {
                res = client.call("config/create", continue_params(name, kind, &out.state, "true")).await?;
            }
            _ => {
                let error = Some(clean_error(&out.error)).filter(|e| !e.is_empty());
                return Ok(ConfigStep::Question { state: out.state, option, error });
            }
        }
    }
    Err(AppError::invalid("O assistente de configuração do rclone não convergiu"))
}

fn continue_params(name: &str, kind: &str, st: &str, result: &str) -> Value {
    json!({
        "name": name,
        "type": kind,
        "parameters": {},
        "opt": { "nonInteractive": true, "obscure": true, "continue": true, "state": st, "result": result },
    })
}

async fn abort_pending(state: &AppState, client: &RcClient, name: &str) {
    let was_pending = state.pending_remotes.lock().unwrap().remove(name);
    if was_pending {
        let _ = client.oauth_stop().await;
        let _ = client.config_delete(name).await;
    }
}

#[tauri::command]
pub async fn create_remote(state: State<'_, AppState>, input: CreateRemoteInput) -> AppResult<ConfigStep> {
    let name = input.name.trim().to_string();
    validate_remote_name(&name)?;
    let client = state.daemon.client()?;
    if client.list_remotes().await?.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
        return Err(AppError::invalid(format!("Já existe uma conta chamada \"{name}\"")));
    }
    let parameters: HashMap<String, String> = input
        .parameters
        .into_iter()
        .map(|(k, v)| (k, v.trim().to_string()))
        .filter(|(_, v)| !v.is_empty())
        .collect();

    state.pending_remotes.lock().unwrap().insert(name.clone());
    let res = client
        .call(
            "config/create",
            json!({
                "name": name,
                "type": input.kind,
                "parameters": parameters,
                "opt": { "nonInteractive": true, "obscure": true },
            }),
        )
        .await;
    let step = match res {
        Ok(res) => advance(&state, &client, &name, &input.kind, res).await,
        Err(e) => Err(e),
    };
    if step.is_err() {
        abort_pending(&state, &client, &name).await;
    }
    step
}

#[tauri::command]
pub async fn answer_config(
    state: State<'_, AppState>,
    name: String,
    kind: String,
    config_state: String,
    result: String,
) -> AppResult<ConfigStep> {
    if !state.pending_remotes.lock().unwrap().contains(&name) {
        return Err(AppError::invalid("Nenhuma configuração em andamento para esta conta"));
    }
    let client = state.daemon.client()?;
    let res = client.call("config/create", continue_params(&name, &kind, &config_state, &result)).await;
    let step = match res {
        Ok(res) => advance(&state, &client, &name, &kind, res).await,
        Err(e) => Err(e),
    };
    if step.is_err() {
        abort_pending(&state, &client, &name).await;
    }
    step
}

#[tauri::command]
pub async fn oauth_progress(state: State<'_, AppState>, name: String, kind: String, job_id: i64) -> AppResult<OAuthProgress> {
    let client = state.daemon.client()?;
    let st = client.job_status(job_id).await?;
    if !st.finished {
        let auth_url = client.oauth_status().await.ok().and_then(|s| s.auth_url);
        return Ok(OAuthProgress { finished: false, auth_url, next: None, error: None });
    }
    if !st.success {
        abort_pending(&state, &client, &name).await;
        let mut error = clean_error(&st.error);
        if error.contains("cancelled") {
            error = "Autorização cancelada".into();
        }
        return Ok(OAuthProgress { finished: true, auth_url: None, next: None, error: Some(error) });
    }
    match advance(&state, &client, &name, &kind, st.output).await {
        Ok(next) => Ok(OAuthProgress { finished: true, auth_url: None, next: Some(next), error: None }),
        Err(e) => {
            abort_pending(&state, &client, &name).await;
            Ok(OAuthProgress { finished: true, auth_url: None, next: None, error: Some(e.to_string()) })
        }
    }
}

#[tauri::command]
pub async fn cancel_remote_config(state: State<'_, AppState>, name: String) -> AppResult<()> {
    let client = state.daemon.client()?;
    abort_pending(&state, &client, &name).await;
    Ok(())
}

#[tauri::command]
pub async fn test_remote(state: State<'_, AppState>, name: String) -> AppResult<RemoteTest> {
    let client = state.daemon.client()?;
    ensure_remote(&client, &name).await?;
    let fs = format!("{name}:");
    match client.fsinfo(&fs).await {
        Ok(info) => {
            // Nem todo provedor suporta `about` (cota); isso não é falha de conexão.
            let about = client.about(&fs).await.ok();
            // fsinfo só lê a configuração; listar a raiz garante que as credenciais funcionam.
            match client.list_dirs(&fs, "").await {
                Ok(_) => Ok(RemoteTest { ok: true, error: None, info: Some(info), about }),
                Err(e) => Ok(RemoteTest { ok: false, error: Some(e.to_string()), info: Some(info), about: None }),
            }
        }
        Err(e) => Ok(RemoteTest { ok: false, error: Some(e.to_string()), info: None, about: None }),
    }
}

#[tauri::command]
pub async fn delete_remote(state: State<'_, AppState>, name: String) -> AppResult<()> {
    let users = state.store.jobs_using_remote(&name)?;
    if !users.is_empty() {
        return Err(AppError::invalid(format!(
            "A conta é usada pelas tarefas: {}. Remova-as antes.",
            users.join(", ")
        )));
    }
    let client = state.daemon.client()?;
    ensure_remote(&client, &name).await?;
    client.config_delete(&name).await
}

#[tauri::command]
pub async fn list_remote_dirs(state: State<'_, AppState>, remote: String, path: String) -> AppResult<Vec<DirEntry>> {
    let client = state.daemon.client()?;
    ensure_remote(&client, &remote).await?;
    let path = path.trim_matches('/').to_string();
    let v = client.list_dirs(&format!("{remote}:"), &path).await?;
    let mut dirs: Vec<DirEntry> = v
        .get("list")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("IsDir").and_then(Value::as_bool).unwrap_or(false))
                .map(|i| DirEntry {
                    name: i.get("Name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    path: i.get("Path").and_then(Value::as_str).unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    dirs.sort_by_key(|d| d.name.to_lowercase());
    Ok(dirs)
}

#[tauri::command]
pub async fn make_remote_dir(state: State<'_, AppState>, remote: String, path: String) -> AppResult<()> {
    let path = path.trim_matches('/');
    if path.is_empty() || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err(AppError::invalid("Nome de pasta inválido"));
    }
    let client = state.daemon.client()?;
    ensure_remote(&client, &remote).await?;
    client.mkdir(&format!("{remote}:"), path).await
}

#[cfg(test)]
mod tests {
    use super::validate_remote_name;

    #[test]
    fn remote_names() {
        assert!(validate_remote_name("gdrive").is_ok());
        assert!(validate_remote_name("Meu Drive_2").is_ok());
        assert!(validate_remote_name("-x").is_err());
        assert!(validate_remote_name("a:b").is_err());
        assert!(validate_remote_name("").is_err());
    }
}
