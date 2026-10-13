//! Cliente HTTP do servidor RC local (`rclone rcd`).
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::types::{ConfigOutput, CoreStats, JobStatus, OAuthStatus};
use crate::error::{AppError, AppResult};
use crate::secret::Secret;

#[derive(Clone)]
pub struct RcClient {
    http: reqwest::Client,
    base: String,
    user: Arc<Secret>,
    pass: Arc<Secret>,
}

impl RcClient {
    pub fn new(port: u16, user: Secret, pass: Secret) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            // Nunca passar o tráfego do RC (com credenciais) por um proxy do sistema.
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base: format!("http://127.0.0.1:{port}"),
            user: Arc::new(user),
            pass: Arc::new(pass),
        })
    }

    #[cfg(test)]
    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub async fn call(&self, path: &str, params: Value) -> AppResult<Value> {
        self.call_timeout(path, params, None).await
    }

    pub async fn call_timeout(&self, path: &str, params: Value, timeout: Option<Duration>) -> AppResult<Value> {
        let mut req = self
            .http
            .post(format!("{}/{}", self.base, path))
            .basic_auth(self.user.expose(), Some(self.pass.expose()))
            .json(&params);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = body
                .get("error")
                .and_then(Value::as_str)
                .map(clean_error)
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(AppError::Rc(msg));
        }
        Ok(body)
    }

    /// Inicia uma chamada assíncrona e devolve o `jobid` do rclone.
    pub async fn call_async(&self, path: &str, mut params: Value) -> AppResult<i64> {
        params["_async"] = json!(true);
        let res = self.call(path, params).await?;
        res.get("jobid")
            .and_then(Value::as_i64)
            .ok_or_else(|| AppError::Rc("resposta sem jobid".into()))
    }

    pub async fn noop(&self) -> AppResult<()> {
        self.call_timeout("rc/noop", json!({}), Some(Duration::from_secs(5))).await.map(|_| ())
    }

    pub async fn quit(&self) -> AppResult<()> {
        self.call_timeout("core/quit", json!({}), Some(Duration::from_secs(3))).await.map(|_| ())
    }

    pub async fn version(&self) -> AppResult<String> {
        let v = self.call("core/version", json!({})).await?;
        Ok(v.get("version").and_then(Value::as_str).unwrap_or_default().to_string())
    }

    pub async fn bwlimit(&self, rate: &str) -> AppResult<()> {
        self.call("core/bwlimit", json!({ "rate": rate })).await.map(|_| ())
    }

    // ---- config ----

    pub async fn providers(&self) -> AppResult<Value> {
        self.call("config/providers", json!({})).await
    }

    pub async fn config_dump(&self) -> AppResult<Value> {
        self.call("config/dump", json!({})).await
    }

    pub async fn list_remotes(&self) -> AppResult<Vec<String>> {
        let v = self.call("config/listremotes", json!({})).await?;
        Ok(v.get("remotes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    pub async fn config_delete(&self, name: &str) -> AppResult<()> {
        self.call("config/delete", json!({ "name": name })).await.map(|_| ())
    }

    pub async fn config_unlock(&self, password: &str) -> AppResult<()> {
        self.call("config/unlock", json!({ "configPassword": password })).await.map(|_| ())
    }

    pub async fn oauth_status(&self) -> AppResult<OAuthStatus> {
        Ok(serde_json::from_value(self.call("config/oauthstatus", json!({})).await?)?)
    }

    pub async fn oauth_stop(&self) -> AppResult<()> {
        self.call("config/oauthstop", json!({})).await.map(|_| ())
    }

    pub fn parse_config_output(v: Value) -> AppResult<ConfigOutput> {
        Ok(serde_json::from_value(v)?)
    }

    // ---- operations ----

    pub async fn fsinfo(&self, fs: &str) -> AppResult<Value> {
        self.call_timeout("operations/fsinfo", json!({ "fs": fs }), Some(Duration::from_secs(60))).await
    }

    pub async fn about(&self, fs: &str) -> AppResult<Value> {
        self.call_timeout("operations/about", json!({ "fs": fs }), Some(Duration::from_secs(60))).await
    }

    pub async fn list_dirs(&self, fs: &str, remote: &str) -> AppResult<Value> {
        self.call_timeout(
            "operations/list",
            json!({ "fs": fs, "remote": remote, "opt": { "dirsOnly": true, "noModTime": true, "noMimeType": true } }),
            Some(Duration::from_secs(120)),
        )
        .await
    }

    pub async fn mkdir(&self, fs: &str, remote: &str) -> AppResult<()> {
        self.call("operations/mkdir", json!({ "fs": fs, "remote": remote })).await.map(|_| ())
    }

    // ---- jobs / stats ----

    pub async fn job_status(&self, jobid: i64) -> AppResult<JobStatus> {
        Ok(serde_json::from_value(self.call("job/status", json!({ "jobid": jobid })).await?)?)
    }

    pub async fn job_stop(&self, jobid: i64) -> AppResult<()> {
        self.call("job/stop", json!({ "jobid": jobid })).await.map(|_| ())
    }

    pub async fn core_stats(&self, group: &str) -> AppResult<CoreStats> {
        Ok(serde_json::from_value(self.call("core/stats", json!({ "group": group })).await?)?)
    }

    pub async fn stats_delete(&self, group: &str) -> AppResult<()> {
        self.call("core/stats-delete", json!({ "group": group })).await.map(|_| ())
    }
}

/// Remove códigos de cor ANSI e prefixos de pânico das mensagens do rclone.
pub fn clean_error(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // Consome até o byte final da sequência CSI (0x40..=0x7E).
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    let out = out.trim();
    let out = out.strip_prefix("panic received: ").unwrap_or(out);
    let out = out.strip_prefix("fatal error: ").unwrap_or(out);
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::clean_error;

    #[test]
    fn strips_ansi_and_panic_prefix() {
        let raw = "\u{1b}[31mprior lock file found: \u{1b}[93mC:\\x.lck\u{1b}[0m \n";
        assert_eq!(clean_error(raw), "prior lock file found: C:\\x.lck");
        assert_eq!(
            clean_error("panic received: fatal error: Failed to load config file"),
            "Failed to load config file"
        );
    }
}
