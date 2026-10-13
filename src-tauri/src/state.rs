use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::jobs::{JobManager, Store};
use crate::rclone::Daemon;

pub struct AppState {
    pub daemon: Arc<Daemon>,
    pub jobs: Arc<JobManager>,
    pub store: Arc<Store>,
    /// Remotes criados pelo wizard e ainda não concluídos: só estes podem ser apagados por "cancelar".
    pub pending_remotes: Mutex<HashSet<String>>,
}
