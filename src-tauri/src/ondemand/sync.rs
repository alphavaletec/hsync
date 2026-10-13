//! Sincronizador de uma conta sob demanda (fase 2), num thread próprio:
//! - **PC → nuvem:** observa a pasta e envia o que deixou de estar "em sincronia"
//!   (arquivos editados, novos arquivos e pastas). O Windows não avisa essas mudanças por
//!   callback, então usamos o observador de arquivos + o estado de cada placeholder.
//! - **nuvem → PC:** a cada intervalo, compara as pastas já listadas com a nuvem:
//!   cria placeholders para novidades, remove o que sumiu e atualiza o que mudou.
//! - **Conflito** (mudou nos dois lados): a versão local vira "nome (conflito HSync).ext"
//!   e sobe como arquivo novo; a versão da nuvem volta com o nome original.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder::{ConvertOptions, PinState, Placeholder, PlaceholderInfo, UpdateOptions};
use serde_json::{json, Value};

use super::filter::{file_time, is_not_found, list_cloud, parse_meta_blob, relative, ClientSource, CloudItem};
use crate::error::{AppError, AppResult};
use crate::jobs::Store;

/// Espera após a última alteração de um caminho antes de enviar (salvar gera vários eventos).
const SETTLE: Duration = Duration::from_secs(3);
/// Intervalo das verificações de novidades na nuvem.
const REFRESH_EVERY: Duration = Duration::from_secs(90);
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH.
const MAX_BLOB: usize = 4096;

/// Registra no log do app e, nos testes, também na saída.
macro_rules! report {
    ($lvl:ident, $($arg:tt)*) => {{
        log::$lvl!($($arg)*);
        #[cfg(test)]
        eprintln!("[sincronizador] {}", format!($($arg)*));
    }};
}

enum Msg {
    Path(PathBuf),
    Refresh,
}

/// Mantém o thread e o observador vivos enquanto a conta está conectada.
pub struct AccountSync {
    stop: Arc<AtomicBool>,
    tx: Sender<Msg>,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Drop for AccountSync {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Refresh);
    }
}

impl AccountSync {
    pub fn start(remote: String, root: PathBuf, client: ClientSource, store: Arc<Store>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Msg>();

        let watch_tx = tx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if !matches!(ev.kind, notify::EventKind::Access(_)) {
                    for p in ev.paths {
                        let _ = watch_tx.send(Msg::Path(p));
                    }
                }
            }
        })
        .and_then(|mut w| {
            use notify::Watcher;
            w.watch(&root, notify::RecursiveMode::Recursive).map(|_| w)
        })
        .map_err(|e| report!(warn, "sob demanda: observador de {} indisponível: {e}", root.display()))
        .ok();

        let worker = Worker { remote, root, client, store, pending: HashMap::new() };
        let stop_w = stop.clone();
        std::thread::Builder::new()
            .name("hsync-sob-demanda".into())
            .spawn(move || worker.run(rx, stop_w))
            .expect("thread do sincronizador");
        Self { stop, tx, _watcher: watcher }
    }

    /// Pede uma verificação da nuvem agora.
    pub fn refresh(&self) {
        let _ = self.tx.send(Msg::Refresh);
    }

    /// Função para enfileirar caminhos (ex.: item trazido de fora da pasta) a partir dos callbacks.
    pub fn queuer(&self) -> Arc<dyn Fn(PathBuf) + Send + Sync> {
        let tx = self.tx.clone();
        Arc::new(move |p| {
            let _ = tx.send(Msg::Path(p));
        })
    }
}

enum Outcome {
    Done,
    /// Tentar de novo mais tarde (arquivo em uso, rclone indisponível…).
    Retry,
}

struct Worker {
    remote: String,
    root: PathBuf,
    client: ClientSource,
    store: Arc<Store>,
    /// caminho → instante do último evento
    pending: HashMap<PathBuf, Instant>,
}

/// Arquivos temporários de editores e do Windows não sobem.
fn is_temporary(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with("~$")
        || name.starts_with(".~lock")
        || name.ends_with(".tmp")
        || name.ends_with(".partial")
        || name.ends_with(".crdownload")
        || name == "desktop.ini"
        || name == "thumbs.db"
}

/// Estado do placeholder (`None` se for um arquivo comum).
///
/// Usa `fixed_size_info` com o tamanho máximo de blob: `info()` do crate entra em pânico com
/// placeholders sem blob (fatia além do buffer) — e no release um pânico encerraria o app.
fn placeholder_info(path: &Path) -> Option<PlaceholderInfo> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    // Só atributos: abrir com `CfOpenFileWithOplock` (o `Placeholder::open` do crate) faz o
    // Windows baixar de novo um arquivo recém-liberado com "Liberar espaço".
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    Placeholder::from(file).fixed_size_info(MAX_BLOB).ok().flatten()
}

/// Abre para converter/atualizar o placeholder. Essas operações exigem um handle Win32 comum —
/// o `Placeholder::open` do crate devolve um handle protegido do CfAPI, rejeitado com
/// "identificador inválido". `exclusive` é necessário para descartar o conteúdo (desidratar).
fn open_for_update(path: &Path, exclusive: bool) -> std::io::Result<Placeholder> {
    use std::os::windows::fs::OpenOptionsExt;
    const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    // Permite abrir pastas também.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let file = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ_WRITE)
        .share_mode(if exclusive { 0 } else { FILE_SHARE_ALL })
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    Ok(Placeholder::from(file))
}

/// O que o menu do Explorer pediu para um arquivo já sincronizado.
#[derive(Debug, PartialEq)]
enum PinAction {
    /// "Liberar espaço": o Explorer só marca como não fixado; descartar o conteúdo é com o provedor.
    FreeUp,
    /// "Sempre manter neste dispositivo": baixar o que falta.
    Download,
}

fn pin_action(info: &PlaceholderInfo, size: u64) -> Option<PinAction> {
    let on_disk = info.on_disk_data_size().max(0) as u64;
    match info.pin_state() {
        PinState::Unpinned if on_disk > 0 => Some(PinAction::FreeUp),
        PinState::Pinned if on_disk < size => Some(PinAction::Download),
        _ => None,
    }
}

/// Metadados do placeholder para a versão da nuvem. O Windows pede downloads com base neste
/// tamanho: se ficar desatualizado (ex.: depois de enviar uma edição), o download é recusado.
fn file_metadata(size: u64, mtime: Option<i64>) -> Metadata {
    let t = file_time(mtime);
    Metadata::file().created(t).written(t).changed(t).size(size).attributes(FILE_ATTRIBUTE_NORMAL)
}

/// Depois de enviar uma edição, o conteúdo local passa a ser a versão da nuvem. O Windows trata
/// o que foi escrito localmente como "modificado" mesmo marcado em sincronia — e nem libera
/// esse trecho nem conhece o tamanho novo (downloads seguintes seriam recusados). Reverter e
/// converter de novo faz o arquivo inteiro contar como dado da nuvem.
fn adopt_local_data(p: &mut Placeholder, blob: Vec<u8>) -> Result<(), String> {
    p.revert().map_err(|e| format!("reverter placeholder: {e}"))?;
    p.convert_to_placeholder(ConvertOptions::default().mark_in_sync().blob(blob), None)
        .map_err(|e| format!("converter placeholder: {e}"))?;
    Ok(())
}

/// Abre só para leitura, para pedir a hidratação ao Windows (`CfHydratePlaceholder`).
fn open_for_hydrate(path: &Path) -> std::io::Result<Placeholder> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    let file = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_ALL).open(path)?;
    Ok(Placeholder::from(file))
}

fn modified_marker(path: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// "relatório.docx" → "relatório (conflito HSync).docx" (ou "… 2", "… 3" se já existir).
pub fn conflict_name(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let suffix = if n == 1 { String::new() } else { format!(" {n}") };
        let candidate = dir.join(format!("{stem} (conflito HSync{suffix}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Msg>, stop: Arc<AtomicBool>) {
        // Ao conectar: alterações feitas com o app fechado + novidades da nuvem.
        self.scan_unsynced(&self.root.clone());
        let mut last_refresh = Instant::now() - REFRESH_EVERY + Duration::from_secs(5);
        let mut force_refresh = false;
        while !stop.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Msg::Path(p)) => {
                    self.pending.insert(p, Instant::now());
                }
                Ok(Msg::Refresh) => force_refresh = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Path(p) => {
                        self.pending.insert(p, Instant::now());
                    }
                    Msg::Refresh => force_refresh = true,
                }
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            self.process_due();
            if force_refresh || last_refresh.elapsed() >= REFRESH_EVERY {
                force_refresh = false;
                last_refresh = Instant::now();
                if let Err(e) = self.refresh_cloud() {
                    report!(warn, "sob demanda ({}): verificar novidades da nuvem: {e}", self.remote);
                }
            }
        }
    }

    fn rc(&self, path: &str, params: Value, secs: u64) -> AppResult<Value> {
        let client = (self.client)()?;
        tauri::async_runtime::block_on(client.call_timeout(path, params, Some(Duration::from_secs(secs))))
    }

    fn stat_cloud(&self, rel: &str) -> AppResult<Option<CloudItem>> {
        let v = match self.rc("operations/stat", json!({ "fs": format!("{}:", self.remote), "remote": rel }), 60) {
            Ok(v) => v,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        let item = v.get("item").cloned().unwrap_or(Value::Null);
        if item.is_null() {
            return Ok(None);
        }
        Ok(Some(CloudItem {
            name: item.get("Name").and_then(Value::as_str).unwrap_or_default().to_string(),
            is_dir: item.get("IsDir").and_then(Value::as_bool).unwrap_or(false),
            size: item.get("Size").and_then(Value::as_i64).unwrap_or(0).max(0) as u64,
            mtime: item.get("ModTime").and_then(Value::as_str).and_then(crate::rclone::client::parse_rfc3339_unix),
        }))
    }

    /// Enfileira tudo que não está em sincronia (o que não é placeholder ou foi alterado)
    /// e os arquivos com pedido pendente do Explorer (liberar espaço / manter no dispositivo).
    fn scan_unsynced(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if is_temporary(&p) {
                continue;
            }
            match placeholder_info(&p) {
                Some(i) if i.is_in_sync() => {
                    if p.is_dir() {
                        self.scan_unsynced(&p);
                    } else if pin_action(&i, e.metadata().map(|m| m.len()).unwrap_or(0)).is_some() {
                        self.pending.insert(p, Instant::now() - SETTLE);
                    }
                }
                _ => {
                    self.pending.insert(p, Instant::now() - SETTLE);
                }
            }
        }
    }

    fn process_due(&mut self) {
        let due: Vec<PathBuf> =
            self.pending.iter().filter(|(_, t)| t.elapsed() >= SETTLE).map(|(p, _)| p.clone()).collect();
        for p in due {
            self.pending.remove(&p);
            match self.process_local(&p) {
                Ok(Outcome::Done) => {}
                Ok(Outcome::Retry) => {
                    self.pending.insert(p, Instant::now() + Duration::from_secs(10));
                }
                Err(e) => {
                    report!(warn, "sob demanda ({}): enviar {}: {e}", self.remote, p.display());
                    self.pending.insert(p, Instant::now() + Duration::from_secs(30));
                }
            }
        }
    }

    fn process_local(&mut self, path: &Path) -> AppResult<Outcome> {
        if is_temporary(path) || !path.starts_with(&self.root) {
            return Ok(Outcome::Done);
        }
        let Ok(meta) = std::fs::symlink_metadata(path) else { return Ok(Outcome::Done) };
        let rel = relative(&self.root, path);
        if rel.is_empty() {
            return Ok(Outcome::Done);
        }
        let info = placeholder_info(path);
        if let Some(i) = info.as_ref().filter(|i| i.is_in_sync()) {
            if meta.is_dir() {
                // Pedido numa pasta: o Explorer marca cada item; os já listados são revistos.
                if matches!(i.pin_state(), PinState::Pinned | PinState::Unpinned) {
                    self.scan_unsynced(path);
                }
                return Ok(Outcome::Done);
            }
            return Ok(self.apply_pin(path, &rel, i, meta.len()));
        }
        let fs = format!("{}:", self.remote);

        if meta.is_dir() {
            if info.is_none() {
                self.rc("operations/mkdir", json!({ "fs": fs, "remote": rel }), 120)?;
                let mut p = open_for_update(path, false)?;
                p.convert_to_placeholder(ConvertOptions::default().mark_in_sync().has_children().blob(b"dir".to_vec()), None)
                    .map_err(|e| AppError::invalid(format!("converter pasta: {e}")))?;
            } else if let Ok(mut p) = open_for_update(path, false) {
                let _ = p.mark_in_sync(true, None);
            }
            self.store.add_ondemand_dir(&self.remote, &rel)?;
            // Pasta nova copiada com conteúdo: os filhos também sobem.
            self.scan_unsynced(path);
            return Ok(Outcome::Done);
        }

        if let Some(i) = &info {
            // Mudança só de metadados num placeholder não baixado: não há conteúdo novo para enviar.
            if (i.on_disk_data_size().max(0) as u64) < meta.len() {
                if let Ok(mut p) = open_for_update(path, false) {
                    let _ = p.mark_in_sync(true, None);
                }
                return Ok(Outcome::Done);
            }
            // Conflito: também mudou na nuvem desde a última sincronização deste arquivo.
            if let Some(known) = parse_meta_blob(i.blob()) {
                if let Some(cloud) = self.stat_cloud(&rel)? {
                    if (cloud.size, cloud.mtime.unwrap_or(0)) != known {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let target = conflict_name(path.parent().unwrap_or(&self.root), &name);
                        std::fs::rename(path, &target)?;
                        report!(warn, 
                            "sob demanda ({}): conflito em {rel}; a versão deste PC foi salva como {}",
                            self.remote,
                            target.display()
                        );
                        self.pending.insert(target, Instant::now());
                        // A versão da nuvem volta com o nome original na próxima verificação.
                        return Ok(Outcome::Done);
                    }
                }
            }
        }

        let before = modified_marker(path);
        let parent = path.parent().unwrap_or(&self.root);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        report!(info, "sob demanda ({}): enviando {rel}", self.remote);
        match self.rc(
            "operations/copyfile",
            json!({ "srcFs": parent.to_string_lossy(), "srcRemote": name, "dstFs": fs, "dstRemote": rel }),
            6 * 3600,
        ) {
            Ok(_) => {}
            // Arquivo aberto/bloqueado por outro programa: tenta de novo depois.
            Err(e) if e.to_string().to_lowercase().contains("being used") => return Ok(Outcome::Retry),
            Err(e) => return Err(e),
        }
        // Mudou de novo durante o envio: envia outra vez depois.
        if modified_marker(path) != before {
            return Ok(Outcome::Retry);
        }
        let cloud = self.stat_cloud(&rel)?;
        let blob = cloud.as_ref().map(CloudItem::blob).unwrap_or_default();
        // Exclusivo para refazer o placeholder; aberto em outro programa → tenta depois.
        let mut ph = match open_for_update(path, info.is_some()) {
            Ok(p) => p,
            Err(_) => return Ok(Outcome::Retry),
        };
        let res = if info.is_none() {
            ph.convert_to_placeholder(ConvertOptions::default().mark_in_sync().blob(blob), None).map(|_| ()).map_err(|e| e.to_string())
        } else {
            adopt_local_data(&mut ph, blob)
        };
        res.map_err(|e| AppError::invalid(format!("marcar como sincronizado: {e}")))?;
        Ok(Outcome::Done)
    }

    /// Atende "Liberar espaço" e "Sempre manter neste dispositivo" num arquivo sincronizado.
    fn apply_pin(&self, path: &Path, rel: &str, info: &PlaceholderInfo, size: u64) -> Outcome {
        match pin_action(info, size) {
            Some(PinAction::FreeUp) => {
                // Exclusivo: não dá para descartar o conteúdo de um arquivo aberto em outro programa.
                let Ok(mut p) = open_for_update(path, true) else { return Outcome::Retry };
                // Editado aqui antes desta versão: o trecho local ainda conta como "modificado".
                if info.modified_data_size() > 0 {
                    if let Err(e) = adopt_local_data(&mut p, info.blob().to_vec()) {
                        report!(info, "sob demanda ({}): liberar espaço de {rel}: {e}", self.remote);
                        return Outcome::Retry;
                    }
                }
                if let Err(e) = p.update(UpdateOptions::default().dehydrate().mark_in_sync(), None) {
                    report!(info, "sob demanda ({}): liberar espaço de {rel}: {e}", self.remote);
                    return Outcome::Retry;
                }
                // Volta ao estado neutro: se for aberto de novo, fica até o próximo "liberar espaço".
                let _ = p.mark_pin(PinState::Unspecified, Default::default());
                report!(info, "sob demanda ({}): espaço liberado: {rel}", self.remote);
                Outcome::Done
            }
            Some(PinAction::Download) => {
                let Ok(mut p) = open_for_hydrate(path) else { return Outcome::Retry };
                match p.hydrate(..) {
                    Ok(()) => Outcome::Done,
                    Err(e) => {
                        report!(info, "sob demanda ({}): baixar {rel}: {e}", self.remote);
                        Outcome::Retry
                    }
                }
            }
            None => Outcome::Done,
        }
    }

    /// Traz para as pastas já listadas as mudanças feitas na nuvem.
    fn refresh_cloud(&mut self) -> AppResult<()> {
        let client = (self.client)()?;
        let mut dirs = self.store.list_ondemand_dirs(&self.remote)?;
        if !dirs.iter().any(String::is_empty) {
            dirs.insert(0, String::new());
        }
        for dir in dirs {
            let local_dir = if dir.is_empty() { self.root.clone() } else { self.root.join(&dir) };
            if !local_dir.is_dir() {
                self.store.remove_ondemand_dirs(&self.remote, &dir)?;
                continue;
            }
            let cloud = match list_cloud(&client, &self.remote, &dir) {
                Ok(c) => c,
                // A pasta sumiu da nuvem: a verificação da pasta-mãe remove a cópia local.
                Err(e) if is_not_found(&e) => continue,
                Err(e) => return Err(e),
            };
            self.refresh_dir(&dir, &local_dir, &cloud);
        }
        Ok(())
    }

    fn refresh_dir(&mut self, dir: &str, local_dir: &Path, cloud: &[CloudItem]) {
        let cloud_by_name: HashMap<&str, &CloudItem> = cloud.iter().map(|c| (c.name.as_str(), c)).collect();
        let local: HashSet<String> = std::fs::read_dir(local_dir)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();

        // Novidades na nuvem → placeholders.
        for item in cloud {
            let path = local_dir.join(&item.name);
            if !local.contains(&item.name) && !self.pending.contains_key(&path) {
                if let Err(e) = item.placeholder().create::<&Path>(local_dir) {
                    report!(info, "sob demanda ({}): criar {}: {e}", self.remote, path.display());
                }
            }
        }

        for name in &local {
            let path = local_dir.join(name);
            if is_temporary(&path) {
                continue;
            }
            // Só mexe no que está em sincronia; o resto é do envio (PC → nuvem). Eventos pendentes
            // de um item em sincronia (ex.: gerados pela própria marcação) não impedem a verificação.
            let Some(info) = placeholder_info(&path) else { continue };
            if !info.is_in_sync() {
                continue;
            }
            let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
            match cloud_by_name.get(name.as_str()) {
                None => {
                    // Apagado na nuvem → some deste PC (o próprio provedor não dispara callbacks).
                    let res = if path.is_dir() {
                        let _ = self.store.remove_ondemand_dirs(&self.remote, &rel);
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    if let Err(e) = res {
                        report!(info, "sob demanda ({}): remover {rel}: {e}", self.remote);
                    }
                }
                Some(c) if !c.is_dir => self.refresh_file(&path, &rel, c, &info),
                _ => {}
            }
        }
    }

    fn refresh_file(&self, path: &Path, rel: &str, cloud: &CloudItem, info: &PlaceholderInfo) {
        let metadata = || file_metadata(cloud.size, cloud.mtime);
        let blob = cloud.blob();
        match parse_meta_blob(info.blob()) {
            // Placeholder da fase 1 (somente leitura, sem versão guardada): adota a versão atual.
            None => {
                if let Ok(mut p) = open_for_update(path, false) {
                    let _ = p.update(UpdateOptions::default().metadata(metadata()).blob(&blob).mark_in_sync(), None);
                }
            }
            Some(known) if known != (cloud.size, cloud.mtime.unwrap_or(0)) => {
                // Mudou na nuvem: descarta a cópia local antiga; o próximo acesso baixa a nova.
                let pinned = info.pin_state() == PinState::Pinned;
                match open_for_update(path, true) {
                    Ok(mut p) => {
                        let res = p.update(
                            UpdateOptions::default().metadata(metadata()).blob(&blob).dehydrate().mark_in_sync(),
                            None,
                        );
                        match res {
                            Ok(_) if pinned => {
                                // "Sempre manter neste dispositivo": baixa a versão nova já.
                                if let Err(e) = p.hydrate(..) {
                                    report!(info, "sob demanda ({}): baixar nova versão de {rel}: {e}", self.remote);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => report!(info, "sob demanda ({}): atualizar {rel}: {e}", self.remote),
                        }
                    }
                    // Aberto em outro programa: tenta na próxima verificação.
                    Err(_) => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_names() {
        let dir = tempfile::tempdir().unwrap();
        let first = conflict_name(dir.path(), "relatório.docx");
        assert_eq!(first.file_name().unwrap(), "relatório (conflito HSync).docx");
        std::fs::write(&first, "x").unwrap();
        assert_eq!(conflict_name(dir.path(), "relatório.docx").file_name().unwrap(), "relatório (conflito HSync 2).docx");
        assert_eq!(conflict_name(dir.path(), "LEIAME").file_name().unwrap(), "LEIAME (conflito HSync)");
    }

    /// Fase 2 de ponta a ponta no Windows real + rclone isolado (sync root temporário).
    /// As operações de "usuário" são feitas por outro processo (cmd), como o Explorer faria.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_way_sync_with_real_windows_and_rclone() {
        use super::super::filter::{CloudFilter, EventSink, FilterEvent};
        use crate::rclone::integration_tests::{rclone_bin, start_rcd};
        use cloud_filter::root::{HydrationType, PopulationType, SecurityId, Session, SyncRootIdBuilder, SyncRootInfo};

        let Some(bin) = rclone_bin() else {
            eprintln!("binário do rclone ausente: teste pulado");
            return;
        };
        let tmp = tempfile::Builder::new().prefix("hsf2").tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("Pasta")).unwrap();
        std::fs::write(data.join("a.txt"), "apagar").unwrap();
        std::fs::write(data.join("b.txt"), "renomear").unwrap();
        std::fs::write(data.join("Pasta").join("c.txt"), "original").unwrap();
        let conf = tmp.path().join("rclone.conf");
        std::fs::write(&conf, format!("[teste]\ntype = alias\nremote = {}\n", data.to_string_lossy().replace('\\', "/"))).unwrap();
        let (mut child, client) = start_rcd(&bin, &conf).await;

        let root = tmp.path().join("raiz");
        std::fs::create_dir_all(&root).unwrap();
        let id = SyncRootIdBuilder::new("HSyncTest")
            .user_security_id(SecurityId::current_user().unwrap())
            .account_name(format!("f2{}", std::process::id()))
            .build();
        id.register(
            SyncRootInfo::default()
                .with_display_name("HSync teste fase 2")
                .with_hydration_type(HydrationType::Full)
                .with_population_type(PopulationType::Full)
                .with_allow_pinning(true)
                .with_icon("%SystemRoot%\\system32\\imageres.dll,0")
                .with_version("1.0")
                .with_path(&root)
                .unwrap(),
        )
        .expect("registrar sync root");

        let store = Arc::new(Store::open_in_memory().unwrap());
        let source: ClientSource = Arc::new(move || Ok(client.clone()));
        // O sincronizador só começa depois de conectar (como no app).
        let slot: Arc<std::sync::Mutex<Option<Arc<dyn Fn(PathBuf) + Send + Sync>>>> = Arc::default();
        let queue = slot.clone();
        let st = store.clone();
        let events: EventSink = Arc::new(move |ev| match ev {
            FilterEvent::Populated(d) => drop(st.add_ondemand_dir("teste", &d)),
            FilterEvent::Gone(d) => drop(st.remove_ondemand_dirs("teste", &d)),
            FilterEvent::DirMoved(a, b) => drop(st.rename_ondemand_dirs("teste", &a, &b)),
            FilterEvent::MovedIn(p) => {
                if let Some(q) = queue.lock().unwrap().as_ref() {
                    q(p)
                }
            }
        });
        let filter = CloudFilter { remote: "teste".into(), root: root.clone(), client: source.clone(), events };
        let conn = Session::new().connect(&root, filter).expect("conectar");
        let account = AccountSync::start("teste".into(), root.clone(), source, store.clone());
        *slot.lock().unwrap() = Some(account.queuer());

        let (r, d) = (root.clone(), data.clone());
        let account = Arc::new(account);
        let acc = account.clone();
        let outcome = tokio::task::spawn_blocking(move || -> Result<(), String> {
            // raw_arg: a linha vai literal para o cmd (Command escaparia as aspas internas com \").
            let cmd = |line: &str| {
                use std::os::windows::process::CommandExt;
                let out = std::process::Command::new("cmd").arg("/c").raw_arg(line).output().map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(format!("cmd falhou: {line}: {}", String::from_utf8_lossy(&out.stderr)));
                }
                Ok(out)
            };
            let q = |p: &Path| format!("\"{}\"", p.display());
            let wait = |what: &str, secs: u64, cond: &dyn Fn() -> bool| -> Result<(), String> {
                let end = Instant::now() + Duration::from_secs(secs);
                while Instant::now() < end {
                    if cond() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(format!("tempo esgotado: {what}"))
            };

            cmd(&format!("dir {}", q(&r)))?;
            cmd(&format!("type {}", q(&r.join("b.txt"))))?;

            // 1. apagar no PC → sai da nuvem (o callback aplica antes de aprovar).
            cmd(&format!("type {}", q(&r.join("a.txt"))))?;
            cmd(&format!("del /f /q {}", q(&r.join("a.txt"))))?;
            if d.join("a.txt").exists() {
                return Err("apagar: a.txt continua na nuvem".into());
            }
            // 2. renomear no PC → renomeia na nuvem.
            cmd(&format!("ren {} b2.txt", q(&r.join("b.txt"))))?;
            if !d.join("b2.txt").exists() || d.join("b.txt").exists() {
                return Err("renomear: nuvem não acompanhou".into());
            }
            // 3. arquivo novo no PC → sobe.
            cmd(&format!("echo novo> {}", q(&r.join("novo.txt"))))?;
            wait("arquivo novo subir", 20, &|| d.join("novo.txt").exists())?;
            // 4. editar arquivo baixado → a edição sobe.
            cmd(&format!("echo mais>> {}", q(&r.join("b2.txt"))))?;
            wait("edição subir", 20, &|| std::fs::read_to_string(d.join("b2.txt")).unwrap_or_default().contains("mais"))?;
            // 5. pasta nova com arquivo → sobem.
            cmd(&format!("mkdir {} && echo x> {}", q(&r.join("Nova")), q(&r.join("Nova").join("x.txt"))))?;
            wait("pasta nova subir", 20, &|| d.join("Nova").join("x.txt").exists())?;
            // 6. arquivo novo na nuvem → aparece no PC.
            std::fs::write(d.join("da-nuvem.txt"), "oi").map_err(|e| e.to_string())?;
            acc.refresh();
            wait("novidade da nuvem aparecer", 20, &|| r.join("da-nuvem.txt").exists())?;
            // 7. apagado na nuvem → some do PC.
            std::fs::remove_file(d.join("novo.txt")).map_err(|e| e.to_string())?;
            wait("envio de novo.txt assentar", 10, &|| {
                placeholder_info(&r.join("novo.txt")).is_some_and(|i| i.is_in_sync())
            })
            .ok();
            acc.refresh();
            wait("exclusão na nuvem refletir", 20, &|| !r.join("novo.txt").exists())?;
            // 8. conflito: muda na nuvem e no PC antes de sincronizar.
            cmd(&format!("dir {}", q(&r.join("Pasta"))))?;
            cmd(&format!("type {}", q(&r.join("Pasta").join("c.txt"))))?;
            std::fs::write(d.join("Pasta").join("c.txt"), "versão da nuvem, mais longa").map_err(|e| e.to_string())?;
            cmd(&format!("echo local>> {}", q(&r.join("Pasta").join("c.txt"))))?;
            let conflict = d.join("Pasta").join("c (conflito HSync).txt");
            wait("cópia de conflito subir", 20, &|| conflict.exists())?;
            let saved = std::fs::read_to_string(&conflict).unwrap_or_default();
            if !saved.contains("local") {
                return Err(format!("conflito: versão do PC errada: {saved:?}"));
            }
            if std::fs::read_to_string(d.join("Pasta").join("c.txt")).unwrap_or_default() != "versão da nuvem, mais longa" {
                return Err("conflito: a versão da nuvem foi sobrescrita".into());
            }
            acc.refresh();
            wait("versão da nuvem voltar ao PC", 20, &|| r.join("Pasta").join("c.txt").exists())?;
            // 9. "Liberar espaço" no Explorer (só marca como não fixado) → o conteúdo sai do disco.
            let b2 = r.join("b2.txt");
            let size = std::fs::metadata(d.join("b2.txt")).map_err(|e| e.to_string())?.len() as i64;
            wait("b2.txt baixado e sincronizado", 20, &|| {
                placeholder_info(&b2).is_some_and(|i| i.is_in_sync() && i.on_disk_data_size() == size)
            })?;
            let state = |p: &Path| {
                placeholder_info(p).map(|i| {
                    format!(
                        "em disco {} validado {} modificado {} pin {:?} em sincronia {} tamanho {:?}",
                        i.on_disk_data_size(),
                        i.validated_data_size(),
                        i.modified_data_size(),
                        i.pin_state(),
                        i.is_in_sync(),
                        std::fs::metadata(p).map(|m| m.len()).ok()
                    )
                })
            };
            let freed = || {
                placeholder_info(&b2).is_some_and(|i| i.on_disk_data_size() == 0 && i.pin_state() == PinState::Unspecified)
            };
            cmd(&format!("attrib -P +U {}", q(&b2)))?;
            wait("liberar espaço", 20, &freed).map_err(|e| format!("{e}: {:?} (nuvem: {size} bytes)", state(&b2)))?;
            // 10. "Sempre manter neste dispositivo" → baixa de novo sem ninguém abrir.
            cmd(&format!("attrib -U +P {}", q(&b2)))?;
            wait("manter neste dispositivo", 20, &|| placeholder_info(&b2).is_some_and(|i| i.on_disk_data_size() == size))
                .map_err(|e| format!("{e}: {:?}", state(&b2)))?;
            // 11. Liberado e aberto de novo: fica no PC até o próximo "Liberar espaço".
            cmd(&format!("attrib -P +U {}", q(&b2)))?;
            wait("liberar espaço de novo", 20, &freed).map_err(|e| format!("{e}: {:?}", state(&b2)))?;
            cmd(&format!("type {}", q(&b2)))?;
            std::thread::sleep(SETTLE + Duration::from_secs(3));
            if !placeholder_info(&b2).is_some_and(|i| i.on_disk_data_size() == size) {
                return Err(format!("aberto de novo: deveria continuar no PC: {:?}", state(&b2)));
            }
            Ok(())
        })
        .await
        .unwrap();

        drop(conn);
        drop(account);
        let _ = id.unregister();
        let _ = child.kill().await;
        outcome.unwrap();
    }

    #[test]
    fn temporary_files_are_skipped() {
        assert!(is_temporary(Path::new(r"C:\x\~$relatorio.docx")));
        assert!(is_temporary(Path::new(r"C:\x\download.crdownload")));
        assert!(!is_temporary(Path::new(r"C:\x\relatorio.docx")));
    }
}
