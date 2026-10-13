//! Estruturas das respostas do RC do rclone usadas pelo app.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreStats {
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub eta: Option<f64>,
    #[serde(default)]
    pub errors: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub fatal_error: bool,
    #[serde(default)]
    pub checks: u64,
    #[serde(default)]
    pub total_checks: u64,
    #[serde(default)]
    pub transfers: u64,
    #[serde(default)]
    pub total_transfers: u64,
    #[serde(default)]
    pub deletes: u64,
    #[serde(default)]
    pub renames: u64,
    #[serde(default)]
    pub elapsed_time: f64,
    #[serde(default)]
    pub transferring: Vec<Transferring>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transferring {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub percentage: f64,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub speed_avg: f64,
    #[serde(default)]
    pub eta: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub output: Value,
}

/// Uma pergunta da máquina de estados de `config/create` (modo não interativo).
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigOutput {
    #[serde(rename = "State", default)]
    pub state: String,
    #[serde(rename = "Option", default)]
    pub option: Option<Value>,
    #[serde(rename = "Error", default)]
    pub error: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStatus {
    #[serde(default)]
    pub auth_url: Option<String>,
}
