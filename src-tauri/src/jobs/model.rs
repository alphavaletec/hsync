use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::rclone::filters::Filters;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    /// Espelha a origem no destino (remove do destino o que não existe na origem).
    Sync,
    /// Copia novos/alterados, nunca remove do destino.
    Copy,
    /// Sincronização bidirecional.
    Bisync,
}

impl SyncMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncMode::Sync => "sync",
            SyncMode::Copy => "copy",
            SyncMode::Bisync => "bisync",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "sync" => SyncMode::Sync,
            "bisync" => SyncMode::Bisync,
            _ => SyncMode::Copy,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Schedule {
    pub enabled: bool,
    pub interval_minutes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::Interrupted => "interrupted",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "running" => RunStatus::Running,
            "success" => RunStatus::Success,
            "cancelled" => RunStatus::Cancelled,
            "interrupted" => RunStatus::Interrupted,
            _ => RunStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: i64,
    pub job_id: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: RunStatus,
    pub dry_run: bool,
    pub trigger: String,
    pub bytes: u64,
    pub files: u64,
    pub deletes: u64,
    pub errors: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunTotals {
    pub bytes: u64,
    pub files: u64,
    pub deletes: u64,
    pub errors: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub dest: String,
    pub mode: SyncMode,
    pub filters: Filters,
    pub schedule: Schedule,
    pub bisync_initialized: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run: Option<RunRecord>,
    // Preenchidos pelo JobManager:
    pub running: bool,
    pub next_run_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInput {
    pub name: String,
    pub source: String,
    pub dest: String,
    pub mode: SyncMode,
    #[serde(default)]
    pub filters: Filters,
    #[serde(default)]
    pub schedule: Schedule,
}

pub const MIN_INTERVAL_MINUTES: u32 = 5;
pub const MAX_INTERVAL_MINUTES: u32 = 7 * 24 * 60;

impl JobInput {
    pub fn normalize(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.source = self.source.trim().to_string();
        self.dest = self.dest.trim().to_string();
        self
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.name.is_empty() || self.name.chars().count() > 80 {
            return Err(AppError::invalid("Dê um nome à tarefa (até 80 caracteres)"));
        }
        let src = Endpoint::parse(&self.source).ok_or_else(|| AppError::invalid("Origem inválida"))?;
        let dst = Endpoint::parse(&self.dest).ok_or_else(|| AppError::invalid("Destino inválido"))?;
        if src.overlaps(&dst) {
            return Err(AppError::invalid("Origem e destino não podem ser a mesma pasta nem uma conter a outra"));
        }
        if self.schedule.enabled
            && !(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&self.schedule.interval_minutes)
        {
            return Err(AppError::invalid(format!(
                "O intervalo deve ficar entre {MIN_INTERVAL_MINUTES} minutos e 7 dias"
            )));
        }
        self.filters.validate()
    }
}

/// Caminho no formato do rclone: local (`C:\pasta`, `/home/x`, `\\srv\share`) ou remoto (`conta:pasta`).
#[derive(Debug, Clone, PartialEq)]
pub enum Endpoint {
    Local(String),
    Remote { remote: String, path: String },
}

impl Endpoint {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let b = s.as_bytes();
        let is_drive = b.len() >= 2
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/');
        if is_drive || s.starts_with("\\\\") || s.starts_with('/') {
            return Some(Endpoint::Local(s.to_string()));
        }
        let (remote, path) = s.split_once(':')?;
        if remote.is_empty() || remote.contains(['/', '\\']) {
            return None;
        }
        Some(Endpoint::Remote { remote: remote.to_string(), path: path.trim_matches('/').to_string() })
    }

    fn key(&self) -> (String, String) {
        match self {
            Endpoint::Local(p) => {
                let mut p = p.replace('\\', "/");
                if cfg!(windows) {
                    p = p.to_lowercase();
                }
                (String::new(), p.trim_end_matches('/').to_string())
            }
            Endpoint::Remote { remote, path } => (remote.clone(), path.clone()),
        }
    }

    pub fn overlaps(&self, other: &Endpoint) -> bool {
        let (ra, pa) = self.key();
        let (rb, pb) = other.key();
        if ra != rb {
            return false;
        }
        let inside = |a: &str, b: &str| a == b || b.is_empty() || a.starts_with(&format!("{b}/"));
        inside(&pa, &pb) || inside(&pb, &pa)
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_endpoints() {
        assert_eq!(Endpoint::parse("C:\\Users\\x"), Some(Endpoint::Local("C:\\Users\\x".into())));
        assert_eq!(Endpoint::parse("D:"), Some(Endpoint::Local("D:".into())));
        assert_eq!(Endpoint::parse("/home/x"), Some(Endpoint::Local("/home/x".into())));
        assert_eq!(
            Endpoint::parse("gdrive:Backup/Fotos/"),
            Some(Endpoint::Remote { remote: "gdrive".into(), path: "Backup/Fotos".into() })
        );
        assert_eq!(Endpoint::parse("relativo/sem/remote"), None);
    }

    #[test]
    fn detects_overlap() {
        let a = Endpoint::parse("C:\\Dados").unwrap();
        let b = Endpoint::parse("c:\\dados\\sub").unwrap();
        let c = Endpoint::parse("C:\\DadosOutros").unwrap();
        assert!(a.overlaps(&b) == cfg!(windows));
        assert!(!a.overlaps(&c));
        let r1 = Endpoint::parse("gd:").unwrap();
        let r2 = Endpoint::parse("gd:x").unwrap();
        assert!(r1.overlaps(&r2));
        assert!(!r2.overlaps(&Endpoint::parse("od:x").unwrap()));
    }
}
