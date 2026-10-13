use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::jobs::{JobManager, Store};
use crate::rclone::Daemon;

pub struct AppState {
    pub daemon: Arc<Daemon>,
    pub jobs: Arc<JobManager>,
    pub store: Arc<Store>,
    /// Configurações de conta em andamento no wizard.
    pub pending_remotes: Mutex<HashMap<String, PendingKind>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    /// Conta nova: cancelar apaga o remote parcial.
    Create,
    /// Renovação do login de uma conta existente: cancelar nunca apaga a conta.
    Reauth,
}

impl PendingKind {
    /// Chamada RC que continua a máquina de estados do rclone.
    pub fn rc_path(self) -> &'static str {
        match self {
            PendingKind::Create => "config/create",
            PendingKind::Reauth => "config/update",
        }
    }
}
