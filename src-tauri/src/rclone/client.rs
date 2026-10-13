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

    // ---- leitura direta de objetos (rcd --rc-serve) ----

    /// Abre `remote:path` para leitura via HTTP (autenticado), opcionalmente só um trecho.
    /// Usado pelos arquivos sob demanda para baixar exatamente o que o Windows pede.
    pub async fn get_object(&self, remote: &str, path: &str, range: Option<(u64, u64)>) -> AppResult<reqwest::Response> {
        let url = format!("{}/{}", self.base, object_url_path(remote, path));
        let mut req = self
            .http
            .get(url)
            .basic_auth(self.user.expose(), Some(self.pass.expose()))
            // Arquivos grandes: o tempo total não pode ser limitado pelo timeout padrão de 30 s.
            .timeout(Duration::from_secs(6 * 3600));
        if let Some((start, end_inclusive)) = range {
            req = req.header(reqwest::header::RANGE, format!("bytes={start}-{end_inclusive}"));
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Rc(format!("HTTP {} ao ler {remote}:{path}", resp.status())));
        }
        Ok(resp)
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

/// Caminho HTTP de um objeto servido pelo rcd: `/[conta:]/pasta/arquivo`, com cada parte codificada.
fn object_url_path(remote: &str, path: &str) -> String {
    let mut out = percent_encode(&format!("[{remote}:]"));
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(&percent_encode(seg));
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Converte o `ModTime` do rclone (RFC 3339, ex. `2026-10-13T10:00:00.123-03:00`) em
/// segundos desde 1970 (UTC). Implementação mínima, sem dependência de biblioteca de datas.
pub fn parse_rfc3339_unix(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    let offset = match b.get(i) {
        Some(b'Z') | Some(b'z') | None => 0,
        Some(&sign) if sign == b'+' || sign == b'-' => {
            let oh = num(i + 1..i + 3)?;
            let om = num(i + 4..i + 6)?;
            let o = oh * 3600 + om * 60;
            if sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    // Dias desde 1970-01-01 (algoritmo "days from civil" de Howard Hinnant).
    let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
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
    fn parses_rfc3339() {
        use super::parse_rfc3339_unix as p;
        assert_eq!(p("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(p("2026-10-13T13:00:00Z"), Some(1_791_896_400));
        assert_eq!(p("2026-10-13T10:00:00.123456789-03:00"), Some(1_791_896_400));
        assert_eq!(p("2024-02-29T12:00:00+02:00"), Some(1_709_200_800));
        assert_eq!(p("lixo"), None);
    }

    #[test]
    fn encodes_object_urls() {
        assert_eq!(
            super::object_url_path("Mega - AVT", "Clientes/Pasta X/arq ç.txt"),
            "%5BMega%20-%20AVT%3A%5D/Clientes/Pasta%20X/arq%20%C3%A7.txt"
        );
    }

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
