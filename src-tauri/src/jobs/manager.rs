//! Execução de tarefas no rclone, monitoramento em tempo real (polling de `core/stats`)
//! e agendamento por intervalo.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::model::{now_ms, Endpoint, Job, JobInput, RunStatus, RunTotals, SyncMode};
use super::selection::{compile_rules, job_input, managed_key, ItemState, Selection, JOB_STATES};
use super::store::Store;
use super::watcher::{event_matches, watch_targets, WatchTarget};
use crate::error::{AppError, AppResult};
use crate::rclone::client::clean_error;
use crate::rclone::types::{CoreStats, Transferring};
use crate::rclone::Daemon;

const MONITOR_INTERVAL: Duration = Duration::from_secs(1);
const SCHEDULER_INTERVAL: Duration = Duration::from_secs(30);
/// MAX_PATH do Windows: o bisync grava listagens com os dois caminhos no nome do arquivo.
const WINDOWS_MAX_PATH: usize = 259;
const BISYNC_LISTING_SUFFIX_LEN: usize = ".path1.lst-dry-new".len();

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunOptions {
    pub dry_run: bool,
    /// Bisync: força `--resync` (recria o estado a partir dos dois lados).
    pub resync: bool,
    /// Bisync: ignora a trava de segurança de exclusões em massa (`--force`).
    pub force: bool,
}

#[derive(Debug, Clone)]
struct ActiveRun {
    job_id: i64,
    job_name: String,
    run_id: i64,
    /// `None` enquanto a chamada ao rclone ainda não retornou o jobid.
    rc_job_id: Option<i64>,
    generation: u64,
    mode: SyncMode,
    dry_run: bool,
    resync: bool,
    cancel_requested: bool,
    started_at: i64,
    last: CoreStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub job_id: i64,
    pub run_id: i64,
    pub job_name: String,
    pub mode: SyncMode,
    pub dry_run: bool,
    pub started_at: i64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub percentage: f64,
    pub speed: f64,
    pub eta: Option<f64>,
    pub transfers: u64,
    pub total_transfers: u64,
    pub checks: u64,
    pub total_checks: u64,
    pub deletes: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub elapsed: f64,
    pub transferring: Vec<Transferring>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFinished {
    pub job_id: i64,
    pub run_id: i64,
    pub job_name: String,
    pub status: RunStatus,
    pub dry_run: bool,
    pub error: Option<String>,
}

pub struct JobManager {
    app: AppHandle,
    daemon: Arc<Daemon>,
    store: Arc<Store>,
    bisync_root: PathBuf,
    active: Mutex<HashMap<i64, ActiveRun>>,
    /// Nomes das tarefas cuja última execução falhou (ícone de erro da bandeja).
    failed_jobs: Mutex<Vec<String>>,
    paused: AtomicBool,
    /// Ids negativos para transferências avulsas (não persistidas como tarefa).
    next_adhoc: AtomicI64,
    /// "Na modificação": observador das pastas locais e eventos aguardando o debounce.
    watch: Mutex<WatchState>,
    watch_tx: tokio::sync::mpsc::UnboundedSender<PathBuf>,
    watch_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<PathBuf>>>,
}

#[derive(Default)]
struct WatchState {
    watcher: Option<notify::RecommendedWatcher>,
    roots: Vec<PathBuf>,
    targets: Vec<WatchTarget>,
    selections: HashMap<String, Vec<Selection>>,
    /// job → (primeiro evento, último evento) ainda não sincronizados.
    pending: HashMap<i64, (Instant, Instant)>,
    /// Eventos até este instante são da própria sincronização (arquivos baixados) e são ignorados.
    quiet_until: HashMap<i64, Instant>,
}

/// Espera após a última alteração antes de sincronizar (salvar um arquivo gera vários eventos).
const WATCH_DEBOUNCE: Duration = Duration::from_secs(4);
/// Mesmo com alterações contínuas, sincroniza no máximo a cada…
const WATCH_MAX_DELAY: Duration = Duration::from_secs(60);
/// Janela após cada execução em que eventos são atribuídos à própria sincronização.
const WATCH_QUIET_AFTER_RUN: Duration = Duration::from_secs(5);

impl JobManager {
    pub fn new(app: AppHandle, daemon: Arc<Daemon>, store: Arc<Store>, bisync_root: PathBuf) -> Arc<Self> {
        let paused = store.get_setting("schedules_paused").ok().flatten().as_deref() == Some("1");
        let (watch_tx, watch_rx) = tokio::sync::mpsc::unbounded_channel();
        Arc::new(Self {
            app,
            daemon,
            store,
            bisync_root,
            active: Mutex::new(HashMap::new()),
            failed_jobs: Mutex::new(Vec::new()),
            paused: AtomicBool::new(paused),
            next_adhoc: AtomicI64::new(-1),
            watch: Mutex::new(WatchState::default()),
            watch_tx,
            watch_rx: Mutex::new(Some(watch_rx)),
        })
    }

    pub fn start_background(self: &Arc<Self>) {
        self.refresh_failed_jobs();
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.monitor_loop().await });
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.scheduler_loop().await });
        if let Some(rx) = self.watch_rx.lock().unwrap().take() {
            let me = self.clone();
            tauri::async_runtime::spawn(async move { me.watch_loop(rx).await });
        }
        self.refresh_watches();
    }

    /// Recalcula as pastas observadas a partir das tarefas com "na modificação".
    /// Só recria o observador do SO quando o conjunto de pastas muda.
    pub fn refresh_watches(&self) {
        let Ok(jobs) = self.store.list_jobs() else { return };
        let targets = watch_targets(&jobs);
        let mut selections = HashMap::new();
        for t in &targets {
            if let Some((remote, _)) = &t.scope {
                if !selections.contains_key(remote) {
                    selections.insert(remote.clone(), self.store.list_selections(remote).unwrap_or_default());
                }
            }
        }
        let mut roots: Vec<PathBuf> = targets.iter().map(|t| t.root.clone()).collect();
        roots.sort();
        roots.dedup();

        let mut st = self.watch.lock().unwrap();
        st.targets = targets;
        st.selections = selections;
        if st.roots == roots && (st.watcher.is_some() || roots.is_empty()) {
            return;
        }
        st.watcher = None;
        st.roots = roots.clone();
        if roots.is_empty() {
            return;
        }
        let tx = self.watch_tx.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if !matches!(ev.kind, notify::EventKind::Access(_)) {
                    for p in ev.paths {
                        let _ = tx.send(p);
                    }
                }
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(mut w) => {
                use notify::Watcher;
                for root in &roots {
                    if let Err(e) = w.watch(root, notify::RecursiveMode::Recursive) {
                        log::warn!("não foi possível observar {}: {e}", root.display());
                    }
                }
                st.watcher = Some(w);
                log::info!("observando alterações em {} pasta(s)", roots.len());
            }
            Err(e) => log::warn!("observador de arquivos indisponível: {e}"),
        }
    }

    async fn watch_loop(self: Arc<Self>, mut rx: tokio::sync::mpsc::UnboundedReceiver<PathBuf>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                Some(path) = rx.recv() => self.on_fs_event(&path),
                _ = tick.tick() => self.flush_watch_pending().await,
            }
        }
    }

    fn on_fs_event(&self, path: &Path) {
        if self.is_paused() {
            return;
        }
        let now = Instant::now();
        let active: Vec<i64> = self.active.lock().unwrap().keys().copied().collect();
        let mut st = self.watch.lock().unwrap();
        let hits: Vec<i64> = st
            .targets
            .iter()
            .filter(|t| event_matches(t, path, &st.selections))
            .map(|t| t.job_id)
            .collect();
        for id in hits {
            // Eventos durante/logo após a execução vêm da própria sincronização.
            let quiet = st.quiet_until.get(&id).is_some_and(|q| now < *q);
            if active.contains(&id) || quiet {
                continue;
            }
            st.pending.entry(id).and_modify(|e| e.1 = now).or_insert((now, now));
        }
    }

    async fn flush_watch_pending(&self) {
        let now = Instant::now();
        let due: Vec<i64> = {
            let mut st = self.watch.lock().unwrap();
            let due: Vec<i64> = st
                .pending
                .iter()
                .filter(|(_, (first, last))| now - *last >= WATCH_DEBOUNCE || now - *first >= WATCH_MAX_DELAY)
                .map(|(id, _)| *id)
                .collect();
            for id in &due {
                st.pending.remove(id);
            }
            due
        };
        for id in due {
            if self.is_running(id) || self.daemon.client().is_err() {
                continue;
            }
            log::info!("alteração local detectada: sincronizando tarefa {id}");
            if let Err(e) = self.start(id, RunOptions::default(), "change").await {
                log::warn!("tarefa {id} (na modificação) não iniciou: {e}");
            }
        }
    }

    pub fn has_active(&self) -> bool {
        !self.active.lock().unwrap().is_empty()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) -> AppResult<()> {
        self.paused.store(paused, Ordering::SeqCst);
        self.store.set_setting("schedules_paused", if paused { "1" } else { "0" })?;
        self.emit_jobs_changed();
        Ok(())
    }

    pub fn list_jobs(&self) -> AppResult<Vec<Job>> {
        let mut jobs = self.store.list_jobs()?;
        let active = self.active.lock().unwrap();
        let paused = self.is_paused();
        for j in &mut jobs {
            j.running = active.contains_key(&j.id);
            j.next_run_at = if paused { None } else { next_run_at(j) };
        }
        Ok(jobs)
    }

    pub fn progress_snapshot(&self) -> Vec<JobProgress> {
        self.active.lock().unwrap().values().map(to_progress).collect()
    }

    /// Validações que dependem do ambiente (remotes existentes, limite de caminho do bisync).
    pub async fn validate_input(&self, input: &JobInput, job_id: Option<i64>) -> AppResult<()> {
        input.validate()?;
        let endpoints = [Endpoint::parse(&input.source), Endpoint::parse(&input.dest)];
        if let Ok(client) = self.daemon.client() {
            let remotes = client.list_remotes().await?;
            for ep in endpoints.iter().flatten() {
                if let Endpoint::Remote { remote, .. } = ep {
                    if !remotes.iter().any(|r| r == remote) {
                        return Err(AppError::invalid(format!("A conta \"{remote}\" não existe")));
                    }
                }
            }
        }
        if input.mode == SyncMode::Bisync && cfg!(windows) {
            // O id ainda não existe na criação: reservamos espaço para um id de 6 dígitos.
            let wd_len = self.bisync_root.as_os_str().len() + 1 + job_id.map_or(6, |id| id.to_string().len());
            let total = wd_len + 1 + input.source.len() + 2 + input.dest.len() + BISYNC_LISTING_SUFFIX_LEN;
            if total > WINDOWS_MAX_PATH {
                return Err(AppError::invalid(format!(
                    "Caminhos longos demais para o modo bidirecional no Windows (limite do rclone: {} caracteres somando origem e destino). Use pastas com caminhos mais curtos.",
                    WINDOWS_MAX_PATH - wd_len - 3 - BISYNC_LISTING_SUFFIX_LEN
                )));
            }
        }
        Ok(())
    }

    pub async fn start(&self, job_id: i64, opts: RunOptions, trigger: &str) -> AppResult<i64> {
        let job = self.store.get_job(job_id)?;
        let client = self.daemon.client()?;
        let generation = self.daemon.generation();
        let group = group_name(job_id);
        let resync = job.mode == SyncMode::Bisync && (opts.resync || !job.bisync_initialized);

        let (path, params) = self.build_call(&job, opts.dry_run, resync, opts.force)?;

        let run_id = {
            let mut active = self.active.lock().unwrap();
            if active.contains_key(&job_id) {
                return Err(AppError::invalid("Esta tarefa já está em execução"));
            }
            let run_id = self.store.insert_run(job_id, opts.dry_run, trigger)?;
            active.insert(
                job_id,
                ActiveRun {
                    job_id,
                    job_name: job.name.clone(),
                    run_id,
                    rc_job_id: None,
                    generation,
                    mode: job.mode,
                    dry_run: opts.dry_run,
                    resync,
                    cancel_requested: false,
                    started_at: now_ms(),
                    last: CoreStats::default(),
                },
            );
            run_id
        };
        self.emit_jobs_changed();

        let _ = client.stats_delete(&group).await;
        match client.call_async(path, params).await {
            Ok(rc_job_id) => {
                if let Some(r) = self.active.lock().unwrap().get_mut(&job_id) {
                    r.rc_job_id = Some(rc_job_id);
                }
                log::info!("tarefa {job_id} iniciada (rclone job {rc_job_id}, {path})");
                Ok(run_id)
            }
            Err(e) => {
                let run = self.active.lock().unwrap().get(&job_id).cloned();
                if let Some(run) = run {
                    self.finish(run, RunStatus::Failed, Some(e.to_string()), CoreStats::default());
                }
                Err(e)
            }
        }
    }

    pub async fn stop(&self, job_id: i64) -> AppResult<()> {
        let rc_job_id = {
            let mut active = self.active.lock().unwrap();
            let run = active.get_mut(&job_id).ok_or_else(|| AppError::invalid("A tarefa não está em execução"))?;
            run.cancel_requested = true;
            run.rc_job_id
        };
        if let Some(id) = rc_job_id {
            self.daemon.client()?.job_stop(id).await?;
        }
        Ok(())
    }

    pub async fn run_all_now(&self) {
        let Ok(jobs) = self.store.list_jobs() else { return };
        for job in jobs {
            if let Err(e) = self.start(job.id, RunOptions::default(), "manual").await {
                log::info!("tarefa {} não iniciada: {e}", job.id);
            }
        }
    }

    /// Usado ao sair do app: pede parada de tudo e espera o monitor registrar o fim.
    pub async fn stop_all_and_wait(&self) {
        let ids: Vec<i64> = self.active.lock().unwrap().keys().copied().collect();
        for id in &ids {
            let _ = self.stop(*id).await;
        }
        for _ in 0..50 {
            if !self.has_active() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let leftovers: Vec<ActiveRun> = self.active.lock().unwrap().values().cloned().collect();
        for run in leftovers {
            let last = run.last.clone();
            self.finish(run, RunStatus::Cancelled, Some("Encerrado ao sair do aplicativo".into()), last);
        }
    }

    /// Remove o estado do bisync de uma tarefa excluída.
    pub fn remove_job_files(&self, job_id: i64) {
        let _ = std::fs::remove_dir_all(self.bisync_root.join(job_id.to_string()));
    }

    pub fn is_running(&self, job_id: i64) -> bool {
        self.active.lock().unwrap().contains_key(&job_id)
    }

    /// Transferência avulsa (baixar/enviar agora): aparece no painel, mas não vira tarefa.
    pub async fn start_adhoc(&self, label: String, path: &'static str, mut params: Value) -> AppResult<i64> {
        let client = self.daemon.client()?;
        let id = self.next_adhoc.fetch_sub(1, Ordering::SeqCst);
        let group = group_name(id);
        params["_group"] = json!(group);
        self.active.lock().unwrap().insert(
            id,
            ActiveRun {
                job_id: id,
                job_name: label,
                run_id: 0,
                rc_job_id: None,
                generation: self.daemon.generation(),
                mode: SyncMode::Copy,
                dry_run: false,
                resync: false,
                cancel_requested: false,
                started_at: now_ms(),
                last: CoreStats::default(),
            },
        );
        match client.call_async(path, params).await {
            Ok(rc_job_id) => {
                if let Some(r) = self.active.lock().unwrap().get_mut(&id) {
                    r.rc_job_id = Some(rc_job_id);
                }
                Ok(id)
            }
            Err(e) => {
                self.active.lock().unwrap().remove(&id);
                Err(e)
            }
        }
    }

    /// Recompila as tarefas gerenciadas da aba Arquivos para `remote` a partir das seleções.
    /// Devolve os ids das tarefas criadas ou alteradas.
    pub fn apply_selection(&self, remote: &str, base: &str, interval_minutes: u32, on_change: bool) -> AppResult<Vec<i64>> {
        let selections = self.store.list_selections(remote)?;
        let mut changed = Vec::new();
        for state in JOB_STATES {
            let key = managed_key(remote, state);
            let existing = self.store.find_managed_job(&key)?;
            match (compile_rules(&selections, state), existing) {
                (None, Some(id)) => {
                    self.store.delete_job(id)?;
                    self.remove_job_files(id);
                }
                (None, None) => {}
                (Some(rules), Some(id)) => {
                    let input = job_input(remote, base, state, rules, interval_minutes, on_change);
                    let old = self.store.get_job(id)?;
                    let rules_changed = old.filters.custom_rules != input.filters.custom_rules;
                    self.store.update_job(id, &input)?;
                    if rules_changed || old.source != input.source || old.dest != input.dest {
                        // Mudou o conjunto sincronizado: refaz o estado do bisync (resync nunca apaga).
                        if state == ItemState::Sync {
                            self.store.set_bisync_initialized(id, false)?;
                        }
                        changed.push(id);
                    }
                }
                (Some(rules), None) => {
                    let input = job_input(remote, base, state, rules, interval_minutes, on_change);
                    input.validate()?;
                    changed.push(self.store.insert_managed_job(&input, &key)?);
                }
            }
        }
        self.emit_jobs_changed();
        Ok(changed)
    }

    /// Ids das tarefas gerenciadas de uma conta.
    pub fn managed_job_ids(&self, remote: &str) -> AppResult<Vec<i64>> {
        let mut ids = Vec::new();
        for state in JOB_STATES {
            if let Some(id) = self.store.find_managed_job(&managed_key(remote, state))? {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    // ------------------------------------------------------------------

    fn build_call(&self, job: &Job, dry_run: bool, resync: bool, force: bool) -> AppResult<(&'static str, Value)> {
        let group = group_name(job.id);
        let rules = job.filters.to_rules();
        let mut params = match job.mode {
            SyncMode::Sync | SyncMode::Copy => json!({
                "srcFs": job.source,
                "dstFs": job.dest,
                "createEmptySrcDirs": true,
                "_config": { "DryRun": dry_run },
            }),
            SyncMode::Bisync => {
                let workdir = self.bisync_root.join(job.id.to_string());
                std::fs::create_dir_all(&workdir)?;
                remove_stale_locks(&workdir);
                bisync_params(&job.source, &job.dest, &workdir, resync, dry_run, force)
            }
        };
        params["_group"] = json!(group);
        if !rules.is_empty() {
            params["_filter"] = json!({ "FilterRule": rules });
        }
        let path = match job.mode {
            SyncMode::Sync => "sync/sync",
            SyncMode::Copy => "sync/copy",
            SyncMode::Bisync => "sync/bisync",
        };
        Ok((path, params))
    }

    fn finish(&self, run: ActiveRun, status: RunStatus, error: Option<String>, stats: CoreStats) {
        self.active.lock().unwrap().remove(&run.job_id);
        self.watch
            .lock()
            .unwrap()
            .quiet_until
            .insert(run.job_id, Instant::now() + WATCH_QUIET_AFTER_RUN);
        let totals = RunTotals {
            bytes: stats.bytes,
            files: stats.transfers,
            deletes: stats.deletes,
            errors: stats.errors,
        };
        let mut error = error.map(|e| clean_error(&e)).filter(|e| !e.is_empty());
        if run.mode == SyncMode::Bisync && status == RunStatus::Failed {
            if let Some(e) = error.as_mut() {
                if e.contains("too many deletes") {
                    *e = "Interrompido por segurança: mais da metade dos arquivos seriam apagados de um dos lados. \
                          Se foi intencional, use \"forçar exclusões\" na tarefa."
                        .into();
                } else if e.contains("--resync") {
                    e.push_str("\n\nUse \"Ressincronizar\" para recriar o estado do bidirecional.");
                }
            }
        }
        // Transferências avulsas (id negativo) não têm registro no histórico.
        if run.job_id > 0 {
            if let Err(e) = self.store.finish_run(run.run_id, status, &totals, error.as_deref()) {
                log::error!("falha ao gravar execução {}: {e}", run.run_id);
            }
        }
        if run.mode == SyncMode::Bisync && run.resync && !run.dry_run && status == RunStatus::Success {
            let _ = self.store.set_bisync_initialized(run.job_id, true);
        }
        log::info!("tarefa {} terminou: {:?}", run.job_id, status);
        let _ = self.app.emit(
            "job-finished",
            JobFinished {
                job_id: run.job_id,
                run_id: run.run_id,
                job_name: run.job_name.clone(),
                status,
                dry_run: run.dry_run,
                error,
            },
        );
        self.emit_jobs_changed();
    }

    /// Recalcula quais tarefas terminaram a última execução com falha (alimenta o ícone de erro da bandeja).
    fn refresh_failed_jobs(&self) {
        let failed: Vec<String> = self
            .store
            .list_jobs()
            .unwrap_or_default()
            .into_iter()
            .filter(|j| j.last_run.as_ref().is_some_and(|r| r.status == RunStatus::Failed))
            .map(|j| j.name)
            .collect();
        *self.failed_jobs.lock().unwrap() = failed;
    }

    /// Ícone e dica da bandeja: sincronizando > erro > sincronizado.
    fn update_tray(&self, running: usize, percent: Option<u64>) {
        use crate::rclone::daemon::DaemonStatus;
        use crate::tray::TrayStatus;
        let failed = self.failed_jobs.lock().unwrap().clone();
        let daemon_problem = match self.daemon.status() {
            DaemonStatus::Failed { .. } => Some("o rclone parou"),
            DaemonStatus::Locked => Some("aguardando a senha mestre"),
            _ => None,
        };
        let status = if running > 0 {
            TrayStatus::Syncing
        } else if !failed.is_empty() || daemon_problem.is_some() {
            TrayStatus::Error
        } else {
            TrayStatus::Synced
        };
        if status != TrayStatus::Syncing {
            let text = match (status, daemon_problem) {
                (TrayStatus::Error, Some(p)) => format!("HSync — erro: {p}"),
                (TrayStatus::Error, None) => format!("HSync — erro em: {}", failed.join(", ")),
                _ => "HSync — tudo sincronizado".to_string(),
            };
            crate::tray::set_tooltip(&self.app, &text);
        }
        crate::tray::set_status(&self.app, status, percent);
    }

    pub fn emit_jobs_changed(&self) {
        self.refresh_failed_jobs();
        self.refresh_watches();
        let _ = self.app.emit("jobs-changed", ());
    }

    async fn monitor_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(MONITOR_INTERVAL);
        let mut was_busy = false;
        loop {
            tick.tick().await;
            let runs: Vec<ActiveRun> = self
                .active
                .lock()
                .unwrap()
                .values()
                .filter(|r| r.rc_job_id.is_some())
                .cloned()
                .collect();
            if runs.is_empty() {
                if was_busy {
                    let _ = self.app.emit("job-progress", Vec::<JobProgress>::new());
                    crate::tray::set_tooltip(&self.app, "HSync — ocioso");
                    was_busy = false;
                }
                continue;
            }
            was_busy = true;

            let client = self.daemon.client().ok();
            let generation = self.daemon.generation();
            for run in runs {
                let Some(c) = client.as_ref().filter(|_| run.generation == generation) else {
                    let last = run.last.clone();
                    self.finish(
                        run,
                        RunStatus::Interrupted,
                        Some("O serviço do Rclone foi reiniciado durante a execução".into()),
                        last,
                    );
                    continue;
                };
                let group = group_name(run.job_id);
                let stats = c.core_stats(&group).await.unwrap_or_else(|_| run.last.clone());
                if let Some(r) = self.active.lock().unwrap().get_mut(&run.job_id) {
                    r.last = stats.clone();
                }
                let rc_job_id = run.rc_job_id.unwrap_or_default();
                match c.job_status(rc_job_id).await {
                    Ok(st) if st.finished => {
                        let stats = c.core_stats(&group).await.unwrap_or(stats);
                        let status = if st.success {
                            RunStatus::Success
                        } else if run.cancel_requested {
                            RunStatus::Cancelled
                        } else {
                            RunStatus::Failed
                        };
                        let error = (!st.success).then(|| st.error.clone());
                        self.finish(run, status, error, stats);
                        let _ = c.stats_delete(&group).await;
                    }
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("job not found") => {
                        self.finish(run, RunStatus::Interrupted, Some("O rclone perdeu o controle da execução".into()), stats);
                    }
                    Err(e) => log::debug!("job/status falhou (transitório?): {e}"),
                }
            }

            let progress = self.progress_snapshot();
            let speed: f64 = progress.iter().map(|p| p.speed).sum();
            if !progress.is_empty() {
                crate::tray::set_tooltip(
                    &self.app,
                    &format!("HSync — sincronizando: {} em execução · {}/s", progress.len(), format_bytes(speed as u64)),
                );
            }
            let total: u64 = progress.iter().map(|p| p.total_bytes).sum();
            let done: u64 = progress.iter().map(|p| p.bytes).sum();
            self.update_tray(progress.len(), (total > 0).then(|| done * 100 / total));
            let _ = self.app.emit("job-progress", progress);
        }
    }

    async fn scheduler_loop(self: Arc<Self>) {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let mut tick = tokio::time::interval(SCHEDULER_INTERVAL);
        loop {
            tick.tick().await;
            if self.is_paused() || self.daemon.client().is_err() {
                continue;
            }
            let Ok(jobs) = self.store.list_jobs() else { continue };
            // Relógio de parede: após suspensão/hibernação, tarefas atrasadas rodam uma vez.
            let now = now_ms();
            for job in jobs {
                let due = next_run_at(&job).is_some_and(|t| now >= t);
                let busy = self.active.lock().unwrap().contains_key(&job.id);
                if due && !busy {
                    if let Err(e) = self.start(job.id, RunOptions::default(), "schedule").await {
                        log::warn!("agendamento da tarefa {} falhou: {e}", job.id);
                    }
                }
            }
        }
    }
}

/// Parâmetros de `sync/bisync` (sem `_group`/`_filter`). Usado pelo app e pelos testes de integração.
pub(crate) fn bisync_params(
    path1: &str,
    path2: &str,
    workdir: &std::path::Path,
    resync: bool,
    dry_run: bool,
    force: bool,
) -> Value {
    let mut p = json!({
        "path1": path1,
        "path2": path2,
        "workdir": workdir.to_string_lossy(),
        "resync": resync,
        "recover": true,
        "resilient": true,
        "createEmptySrcDirs": true,
        "dryRun": dry_run,
        // Via RC as opções não herdam os padrões do CLI: sem isto o limite seria 0% e
        // qualquer exclusão (inclusive renomear) abortaria com "too many deletes".
        "maxDelete": 50,
        "force": force,
    });
    // resyncMode IMPLICA --resync no rclone: só pode ir junto de uma ressincronização,
    // senão toda execução vira resync (que nunca propaga exclusões nem renomeações).
    if resync {
        // Na ressincronização o arquivo mais recente vence, de qualquer lado.
        p["resyncMode"] = json!("newer");
    }
    p
}

fn group_name(job_id: i64) -> String {
    format!("job-{job_id}")
}

fn next_run_at(job: &Job) -> Option<i64> {
    if !job.schedule.enabled || job.schedule.interval_minutes == 0 {
        return None;
    }
    let base = job.last_run.as_ref().map_or(job.updated_at, |r| r.started_at);
    Some(base + job.schedule.interval_minutes as i64 * 60_000)
}

/// Este app garante uma única execução por tarefa; um lock que sobrou é de uma execução interrompida.
fn remove_stale_locks(workdir: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(workdir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "lck") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn to_progress(r: &ActiveRun) -> JobProgress {
    let s = &r.last;
    let percentage = if s.total_bytes > 0 {
        (s.bytes as f64 / s.total_bytes as f64 * 100.0).min(100.0)
    } else if s.total_transfers > 0 {
        s.transfers as f64 / s.total_transfers as f64 * 100.0
    } else {
        0.0
    };
    JobProgress {
        job_id: r.job_id,
        run_id: r.run_id,
        job_name: r.job_name.clone(),
        mode: r.mode,
        dry_run: r.dry_run,
        started_at: r.started_at,
        bytes: s.bytes,
        total_bytes: s.total_bytes,
        percentage,
        speed: s.speed,
        eta: s.eta,
        transfers: s.transfers,
        total_transfers: s.total_transfers,
        checks: s.checks,
        total_checks: s.total_checks,
        deletes: s.deletes,
        errors: s.errors,
        last_error: s.last_error.as_deref().map(clean_error).filter(|e| !e.is_empty()),
        elapsed: s.elapsed_time,
        transferring: s.transferring.clone(),
    }
}

pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i]).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::model::{RunRecord, Schedule};
    use crate::rclone::filters::Filters;

    fn job(interval: u32, enabled: bool, last_started: Option<i64>) -> Job {
        Job {
            id: 1,
            name: "t".into(),
            source: "C:\\a".into(),
            dest: "gd:b".into(),
            mode: SyncMode::Copy,
            filters: Filters::default(),
            schedule: Schedule { enabled, interval_minutes: interval, on_change: false },
            bisync_initialized: false,
            created_at: 0,
            updated_at: 1_000,
            managed: None,
            last_run: last_started.map(|s| RunRecord {
                id: 1,
                job_id: 1,
                started_at: s,
                finished_at: None,
                status: RunStatus::Success,
                dry_run: false,
                trigger: "manual".into(),
                bytes: 0,
                files: 0,
                deletes: 0,
                errors: 0,
                error: None,
            }),
            running: false,
            next_run_at: None,
        }
    }

    #[test]
    fn computes_next_run() {
        assert_eq!(next_run_at(&job(10, false, None)), None);
        assert_eq!(next_run_at(&job(10, true, None)), Some(1_000 + 600_000));
        assert_eq!(next_run_at(&job(10, true, Some(5_000))), Some(5_000 + 600_000));
    }

    #[test]
    fn progress_percentage() {
        let mut r = ActiveRun {
            job_id: 1,
            job_name: "t".into(),
            run_id: 1,
            rc_job_id: Some(1),
            generation: 1,
            mode: SyncMode::Sync,
            dry_run: false,
            resync: false,
            cancel_requested: false,
            started_at: 0,
            last: CoreStats { bytes: 50, total_bytes: 200, ..Default::default() },
        };
        assert_eq!(to_progress(&r).percentage, 25.0);
        r.last = CoreStats { transfers: 1, total_transfers: 4, ..Default::default() };
        assert_eq!(to_progress(&r).percentage, 25.0);
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1,5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5,0 MB");
    }
}
