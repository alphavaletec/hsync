use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Erro devolvido pelo próprio rclone (já sem códigos ANSI).
    #[error("{0}")]
    Rc(String),
    #[error("O serviço do Rclone não está disponível no momento")]
    DaemonUnavailable,
    #[error("A configuração está bloqueada: informe a senha mestre")]
    Locked,
    #[error("{0}")]
    Invalid(String),
    #[error("Banco de dados: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("Comunicação com o Rclone falhou: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

impl AppError {
    pub fn invalid(msg: impl Into<String>) -> Self {
        AppError::Invalid(msg.into())
    }
}

// Comandos Tauri precisam de erros serializáveis; o frontend recebe só a mensagem.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
