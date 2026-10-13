//! Arquivos sob demanda, como o OneDrive: uma pasta por conta, registrada no Windows
//! (Cloud Filter API), onde tudo da nuvem aparece sem ocupar espaço e baixa ao abrir.
//! Editar, criar, renomear e apagar refletem na nuvem; novidades da nuvem aparecem sozinhas.
#[cfg(windows)]
mod filter;
#[cfg(windows)]
mod sync;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::jobs::Store;
use crate::rclone::Daemon;

const SETTING_PREFIX: &str = "ondemand.";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnDemandStatus {
    pub supported: bool,
    pub enabled: bool,
    pub connected: bool,
    pub path: Option<String>,
}

#[cfg(windows)]
type Conn = (cloud_filter::root::Connection<filter::CloudFilter>, sync::AccountSync);
#[cfg(not(windows))]
type Conn = ();

pub struct OnDemand {
    daemon: Arc<Daemon>,
    store: Arc<Store>,
    conns: Mutex<HashMap<String, Conn>>,
}

fn setting_key(remote: &str) -> String {
    format!("{SETTING_PREFIX}{remote}")
}

/// Pasta da conta: `~/HSync Nuvem/<conta>` (nome seguro no Windows).
pub fn default_root(app: &AppHandle, remote: &str) -> AppResult<PathBuf> {
    let home = app.path().home_dir().map_err(|e| AppError::invalid(e.to_string()))?;
    let safe: String = remote
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_. ".contains(c) { c } else { '_' })
        .collect();
    Ok(home.join("HSync Nuvem").join(safe.trim()))
}

impl OnDemand {
    pub fn new(daemon: Arc<Daemon>, store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self { daemon, store, conns: Mutex::new(HashMap::new()) })
    }

    pub fn status(&self, remote: &str) -> AppResult<OnDemandStatus> {
        let path = self.store.get_setting(&setting_key(remote))?;
        Ok(OnDemandStatus {
            supported: cfg!(windows),
            enabled: path.is_some(),
            connected: self.conns.lock().unwrap().contains_key(remote),
            path,
        })
    }

    /// Pede ao sincronizador da conta uma verificação imediata de novidades da nuvem.
    pub fn refresh_now(&self, remote: &str) -> AppResult<()> {
        #[cfg(windows)]
        if let Some((_, account)) = self.conns.lock().unwrap().get(remote) {
            account.refresh();
            return Ok(());
        }
        Err(AppError::invalid("Arquivos sob demanda não estão ativos nesta conta"))
    }

    pub fn is_enabled(&self, remote: &str) -> bool {
        self.store.get_setting(&setting_key(remote)).ok().flatten().is_some()
    }

    /// Reconecta, ao iniciar o app, todas as contas com sob demanda ativado.
    pub fn reconnect_all(&self) {
        let Ok(entries) = self.store.settings_with_prefix(SETTING_PREFIX) else { return };
        for (key, path) in entries {
            let remote = key[SETTING_PREFIX.len()..].to_string();
            if let Err(e) = self.connect(&remote, PathBuf::from(&path)) {
                log::warn!("sob demanda: não foi possível reconectar {remote}: {e}");
            }
        }
    }

    /// Desconecta tudo (ao sair): o Windows passa a mostrar "provedor não está em execução".
    pub fn disconnect_all(&self) {
        self.conns.lock().unwrap().clear();
    }

    pub fn enable(&self, app: &AppHandle, remote: &str) -> AppResult<OnDemandStatus> {
        let root = default_root(app, remote)?;
        std::fs::create_dir_all(&root)?;
        self.connect(remote, root.clone())?;
        self.store.set_setting(&setting_key(remote), &root.to_string_lossy())?;
        self.status(remote)
    }

    /// Desliga e manda a pasta para a Lixeira (o conteúdo é cópia do que está na nuvem).
    pub fn disable(&self, remote: &str) -> AppResult<()> {
        let path = self.store.get_setting(&setting_key(remote))?;
        self.conns.lock().unwrap().remove(remote);
        #[cfg(windows)]
        {
            let id = sync_root_id(remote)?;
            if id.is_registered().unwrap_or(false) {
                id.unregister().map_err(|e| AppError::invalid(format!("remover do Explorer: {e}")))?;
            }
        }
        self.store.delete_setting(&setting_key(remote))?;
        self.store.remove_ondemand_dirs(remote, "")?;
        if let Some(p) = path {
            if std::path::Path::new(&p).exists() {
                if let Err(e) = trash::delete(&p) {
                    log::warn!("sob demanda: pasta {p} não foi para a Lixeira: {e}");
                }
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    fn connect(&self, remote: &str, root: PathBuf) -> AppResult<()> {
        use cloud_filter::root::{HydrationType, PopulationType, Session, SyncRootInfo};

        if self.conns.lock().unwrap().contains_key(remote) {
            return Ok(());
        }
        std::fs::create_dir_all(&root)?;
        let id = sync_root_id(remote)?;
        fn err(what: &str, e: impl std::fmt::Display) -> AppError {
            AppError::invalid(format!("{what}: {e}"))
        }
        // Registro (persistente no Windows): coloca a pasta no painel de navegação do Explorer.
        let registered_here = id
            .is_registered()
            .unwrap_or(false)
            && id.info().map(|i| i.path() == root).unwrap_or(false);
        if !registered_here {
            if id.is_registered().unwrap_or(false) {
                let _ = id.unregister();
            }
            let exe = std::env::current_exe()?;
            let info = SyncRootInfo::default()
                .with_display_name(format!("HSync - {remote}"))
                .with_hydration_type(HydrationType::Full)
                .with_population_type(PopulationType::Full)
                .with_allow_pinning(true)
                .with_icon(format!("{},0", exe.display()))
                .with_version(env!("CARGO_PKG_VERSION"))
                .with_path(&root)
                .map_err(|e| err("pasta", e))?;
            id.register(info).map_err(|e| err("registrar no Explorer", e))?;
        }
        let daemon = self.daemon.clone();
        let client: filter::ClientSource = Arc::new(move || daemon.client());
        // Sincronizador da conta (PC → nuvem e novidades da nuvem → PC).
        let account = sync::AccountSync::start(remote.to_string(), root.clone(), client.clone(), self.store.clone());
        let queue = account.queuer();
        let (store, name) = (self.store.clone(), remote.to_string());
        let events: filter::EventSink = Arc::new(move |ev| {
            let res = match ev {
                filter::FilterEvent::Populated(dir) => store.add_ondemand_dir(&name, &dir),
                filter::FilterEvent::Gone(dir) => store.remove_ondemand_dirs(&name, &dir),
                filter::FilterEvent::DirMoved(from, to) => store.rename_ondemand_dirs(&name, &from, &to),
                filter::FilterEvent::MovedIn(path) => {
                    queue(path);
                    Ok(())
                }
            };
            if let Err(e) = res {
                log::warn!("sob demanda ({name}): {e}");
            }
        });
        let filter = filter::CloudFilter { remote: remote.to_string(), root: root.clone(), client, events };
        let conn = Session::new().connect(&root, filter).map_err(|e| err("conectar", e))?;
        self.conns.lock().unwrap().insert(remote.to_string(), (conn, account));
        log::info!("sob demanda: {remote} conectado em {}", root.display());
        Ok(())
    }

    #[cfg(not(windows))]
    fn connect(&self, _remote: &str, _root: PathBuf) -> AppResult<()> {
        Err(AppError::invalid("Arquivos sob demanda estão disponíveis apenas no Windows"))
    }
}

/// Id do sync root: `HSync!<SID do usuário>!<conta>` ('!' é separador e não pode aparecer na conta).
#[cfg(windows)]
fn sync_root_id(remote: &str) -> AppResult<cloud_filter::root::SyncRootId> {
    use cloud_filter::root::{SecurityId, SyncRootIdBuilder};
    let sid = SecurityId::current_user().map_err(|e| AppError::invalid(format!("usuário do Windows: {e}")))?;
    Ok(SyncRootIdBuilder::new("HSync").user_security_id(sid).account_name(remote.replace('!', "_")).build())
}
