//! Ciclo de vida do sidecar `rclone rcd`: inicialização com credenciais efêmeras,
//! captura da porta, health check, reinício com backoff e encerramento seguro.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Notify};

use super::client::{clean_error, RcClient};
use crate::error::{AppError, AppResult};
use crate::secret::{random_token, Secret};

const LOG_CAPACITY: usize = 1000;
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(120);
const ENCRYPTED_MARKER: &str = "RCLONE_ENCRYPT_V0:";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum DaemonStatus {
    Starting,
    /// Rodando, mas o rclone.conf está cifrado e aguarda a senha mestre.
    Locked,
    Ready { port: u16, version: String },
    Restarting { attempt: u32, reason: String },
    Failed { reason: String },
    Maintenance,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub time: String,
    pub level: String,
    pub msg: String,
}

pub struct DaemonPaths {
    pub rclone: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
}

struct Running {
    client: RcClient,
    port: u16,
    version: String,
    kill: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct LogBuffer {
    lines: VecDeque<LogLine>,
    pending: Vec<LogLine>,
}

pub struct Daemon {
    app: AppHandle,
    paths: DaemonPaths,
    status: Mutex<DaemonStatus>,
    running: Mutex<Option<Running>>,
    generation: AtomicU64,
    config_pass: Mutex<Option<Secret>>,
    bwlimit: Mutex<String>,
    logs: Mutex<LogBuffer>,
    shutting_down: AtomicBool,
    maintenance: AtomicBool,
    wake: Notify,
    exited: Notify,
    #[cfg(windows)]
    job: Option<win32job::Job>,
}

impl Daemon {
    pub fn new(app: AppHandle, paths: DaemonPaths, bwlimit: String) -> Arc<Self> {
        Arc::new(Self {
            app,
            paths,
            status: Mutex::new(DaemonStatus::Starting),
            running: Mutex::new(None),
            generation: AtomicU64::new(0),
            config_pass: Mutex::new(None),
            bwlimit: Mutex::new(bwlimit),
            logs: Mutex::new(LogBuffer::default()),
            shutting_down: AtomicBool::new(false),
            maintenance: AtomicBool::new(false),
            wake: Notify::new(),
            exited: Notify::new(),
            #[cfg(windows)]
            job: create_kill_on_close_job(),
        })
    }

    /// Inicia o supervisor (spawn + watchdog) e o despachante de logs.
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.supervise().await });
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.flush_logs_loop().await });
    }

    pub fn status(&self) -> DaemonStatus {
        self.status.lock().unwrap().clone()
    }

    /// Incrementa a cada novo processo; jobs iniciados em outra geração foram perdidos.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Cliente para uso normal: exige o daemon pronto e desbloqueado.
    pub fn client(&self) -> AppResult<RcClient> {
        match self.status() {
            DaemonStatus::Ready { .. } => self.raw_client().ok_or(AppError::DaemonUnavailable),
            DaemonStatus::Locked => Err(AppError::Locked),
            _ => Err(AppError::DaemonUnavailable),
        }
    }

    fn raw_client(&self) -> Option<RcClient> {
        self.running.lock().unwrap().as_ref().map(|r| r.client.clone())
    }

    pub fn is_config_encrypted(&self) -> bool {
        std::fs::read_to_string(&self.paths.config)
            .map(|s| s.contains(ENCRYPTED_MARKER))
            .unwrap_or(false)
    }

    pub fn logs(&self) -> Vec<LogLine> {
        self.logs.lock().unwrap().lines.iter().cloned().collect()
    }

    /// Sai do estado `Failed` e tenta novamente.
    pub fn retry(&self) {
        self.wake.notify_one();
    }

    pub async fn set_bwlimit(&self, rate: String) -> AppResult<()> {
        *self.bwlimit.lock().unwrap() = rate.clone();
        if let Ok(c) = self.client() {
            c.bwlimit(if rate.is_empty() { "off" } else { &rate }).await?;
        }
        Ok(())
    }

    pub async fn unlock(&self, password: Secret) -> AppResult<()> {
        let client = self.raw_client().ok_or(AppError::DaemonUnavailable)?;
        try_unlock(&client, &password).await?;
        *self.config_pass.lock().unwrap() = Some(password);
        self.set_ready_from_running();
        Ok(())
    }

    /// Confere se `password` é a senha mestre atualmente em uso.
    pub fn verify_master_password(&self, password: &Secret) -> bool {
        self.config_pass.lock().unwrap().as_ref() == Some(password)
    }

    /// Define ou troca a senha mestre (criptografia nativa do rclone).
    pub async fn set_master_password(&self, current: Option<Secret>, new: Secret) -> AppResult<()> {
        let encrypted = self.is_config_encrypted();
        if encrypted {
            let cur = current.as_ref().ok_or_else(|| AppError::invalid("Informe a senha atual"))?;
            if !self.verify_master_password(cur) {
                return Err(AppError::invalid("Senha atual incorreta"));
            }
        }
        if !self.paths.config.exists() {
            if let Some(dir) = self.paths.config.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&self.paths.config, "")?;
        }
        let input = format!("{0}\n{0}\n", new.expose());
        let pass_env = if encrypted { current } else { None };
        self.with_daemon_stopped(|| self.run_cli(&["config", "encryption", "set"], Some(input), pass_env.as_ref()))
            .await?;
        *self.config_pass.lock().unwrap() = Some(new);
        Ok(())
    }

    pub async fn remove_master_password(&self, current: Secret) -> AppResult<()> {
        if !self.is_config_encrypted() {
            return Ok(());
        }
        if !self.verify_master_password(&current) {
            return Err(AppError::invalid("Senha atual incorreta"));
        }
        self.with_daemon_stopped(|| self.run_cli(&["config", "encryption", "remove"], None, Some(&current)))
            .await?;
        *self.config_pass.lock().unwrap() = None;
        Ok(())
    }

    /// Encerramento definitivo: `core/quit`, espera até 3s e mata o processo se preciso.
    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.stop_current_and_wait().await;
        self.wake.notify_one();
        self.set_status(DaemonStatus::Stopped);
    }

    // ------------------------------------------------------------------

    async fn with_daemon_stopped<F, Fut>(&self, f: F) -> AppResult<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<String>>,
    {
        self.maintenance.store(true, Ordering::SeqCst);
        self.stop_current_and_wait().await;
        let result = f().await;
        self.maintenance.store(false, Ordering::SeqCst);
        self.wake.notify_one();
        result.map(|_| ())
    }

    async fn stop_current_and_wait(&self) {
        let kill = self.running.lock().unwrap().as_mut().and_then(|r| r.kill.take());
        if let Some(k) = kill {
            let _ = k.send(());
        }
        let _ = tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                let notified = self.exited.notified();
                if self.running.lock().unwrap().is_none() {
                    break;
                }
                notified.await;
            }
        })
        .await;
    }

    fn set_status(&self, status: DaemonStatus) {
        let mut cur = self.status.lock().unwrap();
        if *cur != status {
            *cur = status.clone();
            drop(cur);
            log::info!("rclone daemon: {status:?}");
            let _ = self.app.emit("daemon-status", status);
        }
    }

    fn set_ready_from_running(&self) {
        let info = self.running.lock().unwrap().as_ref().map(|r| (r.port, r.version.clone()));
        if let Some((port, version)) = info {
            self.set_status(DaemonStatus::Ready { port, version });
        }
    }

    async fn supervise(self: Arc<Self>) {
        let mut failures: VecDeque<Instant> = VecDeque::new();
        loop {
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            if self.maintenance.load(Ordering::SeqCst) {
                self.set_status(DaemonStatus::Maintenance);
                self.wake.notified().await;
                continue;
            }
            if failures.is_empty() {
                self.set_status(DaemonStatus::Starting);
            }

            let reason = match self.spawn_once().await {
                Ok((child, client, port)) => {
                    let (kill_tx, kill_rx) = oneshot::channel();
                    *self.running.lock().unwrap() = Some(Running {
                        client: client.clone(),
                        port,
                        version: String::new(),
                        kill: Some(kill_tx),
                    });
                    self.generation.fetch_add(1, Ordering::SeqCst);
                    self.after_start(&client).await;
                    let reason = watch(child, &client, kill_rx).await;
                    *self.running.lock().unwrap() = None;
                    self.exited.notify_waiters();
                    reason
                }
                Err(e) => e.to_string(),
            };

            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            if self.maintenance.load(Ordering::SeqCst) {
                continue;
            }

            log::warn!("rclone daemon caiu: {reason}");
            let now = Instant::now();
            failures.push_back(now);
            while failures.front().is_some_and(|t| now.duration_since(*t) > FAILURE_WINDOW) {
                failures.pop_front();
            }
            if failures.len() >= MAX_FAILURES {
                self.set_status(DaemonStatus::Failed { reason });
                self.wake.notified().await;
                failures.clear();
                continue;
            }
            let attempt = failures.len() as u32;
            self.set_status(DaemonStatus::Restarting { attempt, reason });
            let delay = Duration::from_secs((1u64 << (attempt - 1)).min(30));
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = self.wake.notified() => {}
            }
        }
        self.set_status(DaemonStatus::Stopped);
    }

    async fn after_start(&self, client: &RcClient) {
        let version = client.version().await.unwrap_or_default();
        if let Some(r) = self.running.lock().unwrap().as_mut() {
            r.version = version;
        }
        let rate = self.bwlimit.lock().unwrap().clone();
        if !rate.is_empty() {
            if let Err(e) = client.bwlimit(&rate).await {
                log::warn!("bwlimit '{rate}' rejeitado: {e}");
            }
        }
        if self.is_config_encrypted() {
            let pass = self.config_pass.lock().unwrap().clone();
            match pass {
                Some(p) if try_unlock(client, &p).await.is_ok() => self.set_ready_from_running(),
                _ => {
                    *self.config_pass.lock().unwrap() = None;
                    self.set_status(DaemonStatus::Locked);
                }
            }
        } else {
            self.set_ready_from_running();
        }
    }

    fn base_command(&self) -> Command {
        let mut cmd = Command::new(&self.paths.rclone);
        // Variáveis RCLONE_* do ambiente do usuário poderiam alterar o comportamento do daemon
        // (ex.: habilitar a web GUI ou desligar a autenticação). Removemos todas.
        for (k, _) in std::env::vars_os() {
            if k.to_string_lossy().to_ascii_uppercase().starts_with("RCLONE_") {
                cmd.env_remove(k);
            }
        }
        cmd.kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        cmd
    }

    fn attach_to_job(&self, _child: &Child) {
        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, _child.raw_handle()) {
            if let Err(e) = job.assign_process(handle as isize) {
                log::warn!("falha ao associar rclone ao Job Object: {e:?}");
            }
        }
    }

    async fn spawn_once(self: &Arc<Self>) -> AppResult<(Child, RcClient, u16)> {
        if !self.paths.rclone.exists() {
            return Err(AppError::invalid(format!(
                "binário do rclone não encontrado em {}",
                self.paths.rclone.display()
            )));
        }
        std::fs::create_dir_all(&self.paths.cache)?;
        if let Some(dir) = self.paths.config.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let user = Secret::from(random_token(16));
        let pass = Secret::from(random_token(32));

        let mut cmd = self.base_command();
        cmd.arg("rcd")
            .args(["--rc-addr", "127.0.0.1:0"])
            .arg("--config")
            .arg(&self.paths.config)
            .arg("--cache-dir")
            .arg(&self.paths.cache)
            .args(["--ask-password=false", "--use-json-log", "--log-level", "INFO"])
            .args(["--rc-job-expire-duration", "10m"])
            // Credenciais via ambiente: não aparecem na linha de comando do processo.
            .env("RCLONE_RC_USER", user.expose())
            .env("RCLONE_RC_PASS", pass.expose())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        self.attach_to_job(&child);

        let stderr = child.stderr.take().ok_or_else(|| AppError::invalid("stderr indisponível"))?;
        let (port_tx, port_rx) = oneshot::channel::<u16>();
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut port_tx = Some(port_tx);
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for entry in parse_log_lines(&line) {
                    if let Some(tx) = port_tx.take() {
                        match parse_port(&entry.msg) {
                            Some(p) => {
                                let _ = tx.send(p);
                            }
                            None => port_tx = Some(tx),
                        }
                    }
                    me.push_log(entry);
                }
            }
        });

        let port = match tokio::time::timeout(Duration::from_secs(15), port_rx).await {
            Ok(Ok(p)) => p,
            _ => {
                let _ = child.kill().await;
                return Err(AppError::invalid("o rclone não informou a porta do RC a tempo"));
            }
        };

        let client = RcClient::new(port, user, pass)?;
        for _ in 0..10 {
            if client.noop().await.is_ok() {
                return Ok((child, client, port));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let _ = child.kill().await;
        Err(AppError::invalid("o RC do rclone não respondeu"))
    }

    /// Executa um subcomando curto do rclone (fora do daemon) com o mesmo config.
    async fn run_cli(&self, args: &[&str], stdin: Option<String>, config_pass: Option<&Secret>) -> AppResult<String> {
        let mut cmd = self.base_command();
        cmd.args(args)
            .arg("--config")
            .arg(&self.paths.config)
            .arg("--ask-password=false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(p) = config_pass {
            cmd.env("RCLONE_CONFIG_PASS", p.expose());
        }
        let mut child = cmd.spawn()?;
        self.attach_to_job(&child);
        if let Some(mut pipe) = child.stdin.take() {
            if let Some(data) = stdin {
                let data = zeroize::Zeroizing::new(data);
                pipe.write_all(data.as_bytes()).await?;
            }
            drop(pipe);
        }
        let out = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
            .await
            .map_err(|_| AppError::invalid("o rclone demorou demais para responder"))??;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("falha desconhecida");
            return Err(AppError::Rc(clean_error(last)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn push_log(&self, line: LogLine) {
        let mut logs = self.logs.lock().unwrap();
        if logs.lines.len() >= LOG_CAPACITY {
            logs.lines.pop_front();
        }
        logs.lines.push_back(line.clone());
        if logs.pending.len() >= LOG_CAPACITY {
            logs.pending.remove(0);
        }
        logs.pending.push(line);
    }

    /// Envia logs ao frontend em lotes para não inundar o IPC durante syncs grandes.
    async fn flush_logs_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            let batch = std::mem::take(&mut self.logs.lock().unwrap().pending);
            if !batch.is_empty() {
                let _ = self.app.emit("daemon-log", batch);
            }
            if self.shutting_down.load(Ordering::SeqCst) && self.running.lock().unwrap().is_none() {
                break;
            }
        }
    }
}

async fn try_unlock(client: &RcClient, password: &Secret) -> AppResult<()> {
    client.config_unlock(password.expose()).await?;
    // `config/unlock` não falha com senha errada: confirmamos lendo o config.
    client
        .list_remotes()
        .await
        .map(|_| ())
        .map_err(|_| AppError::invalid("Senha mestre incorreta"))
}

/// Observa o processo até ele sair, ser morto pelo health check ou receber pedido de parada.
async fn watch(mut child: Child, client: &RcClient, mut kill_rx: oneshot::Receiver<()>) -> String {
    let mut misses = 0;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.tick().await;
    loop {
        tokio::select! {
            st = child.wait() => {
                return match st {
                    Ok(s) => format!("processo encerrado ({s})"),
                    Err(e) => format!("processo encerrado ({e})"),
                };
            }
            _ = &mut kill_rx => {
                let _ = client.quit().await;
                if tokio::time::timeout(Duration::from_secs(3), child.wait()).await.is_err() {
                    let _ = child.kill().await;
                }
                return "encerrado pelo app".into();
            }
            _ = tick.tick() => {
                if client.noop().await.is_ok() {
                    misses = 0;
                } else {
                    misses += 1;
                    if misses >= 3 {
                        let _ = child.kill().await;
                        return "sem resposta ao health check".into();
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn create_kill_on_close_job() -> Option<win32job::Job> {
    let job = win32job::Job::create().ok()?;
    let mut info = job.query_extended_limit_info().ok()?;
    info.limit_kill_on_job_close();
    job.set_extended_limit_info(&info).ok()?;
    Some(job)
}

#[derive(serde::Deserialize)]
struct RawLog {
    #[serde(default)]
    time: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    msg: String,
}

/// Converte uma linha de stderr do rclone em entradas de log. O bisync, quando roda via RC,
/// embrulha os próprios logs num único evento cujo `msg` são várias linhas JSON; essas
/// linhas internas viram entradas separadas, com o nível e o horário originais.
pub(crate) fn parse_log_lines(line: &str) -> Vec<LogLine> {
    let Ok(outer) = serde_json::from_str::<RawLog>(line) else {
        return vec![finish_entry(LogLine { time: String::new(), level: "info".into(), msg: clean_error(line) })];
    };
    let inner: Vec<&str> = outer.msg.lines().filter(|l| !l.trim().is_empty()).collect();
    if inner.first().is_some_and(|l| l.trim_start().starts_with('{')) {
        return inner
            .into_iter()
            .map(|l| match serde_json::from_str::<RawLog>(l) {
                Ok(r) => LogLine { time: r.time, level: r.level, msg: clean_error(&r.msg) },
                Err(_) => LogLine { time: outer.time.clone(), level: outer.level.clone(), msg: clean_error(l) },
            })
            .map(finish_entry)
            .collect();
    }
    vec![finish_entry(LogLine { time: outer.time, level: outer.level, msg: clean_error(&outer.msg) })]
}

fn finish_entry(mut entry: LogLine) -> LogLine {
    // O rclone registra o usuário do RC em claro; ele não deve chegar à UI nem ao log.
    if entry.msg.starts_with("Using --user") {
        entry.msg = "Autenticação do RC ativa (credenciais efêmeras)".into();
    }
    entry
}

pub(crate) fn parse_port(msg: &str) -> Option<u16> {
    let rest = msg.strip_prefix("Serving remote control on ")?;
    let after = &rest[rest.find("127.0.0.1:")? + "127.0.0.1:".len()..];
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port_from_json_log() {
        let line = r#"{"time":"2026-10-13T10:06:17-03:00","level":"notice","msg":"Serving remote control on http://127.0.0.1:59879/","source":"rcserver/rcserver.go:151"}"#;
        let entry = parse_log_lines(line).remove(0);
        assert_eq!(entry.level, "notice");
        assert_eq!(parse_port(&entry.msg), Some(59879));
    }

    #[test]
    fn ignores_other_lines() {
        assert_eq!(parse_port("Using --user u1 --pass XXXX as authenticated user"), None);
        let entry = parse_log_lines("texto solto\n").remove(0);
        assert_eq!(entry.msg, "texto solto");
    }

    #[test]
    fn redacts_rc_user() {
        let line = r#"{"level":"info","msg":"Using --user dce14d7e52ad --pass XXXX as authenticated user"}"#;
        assert!(!parse_log_lines(line)[0].msg.contains("dce14d7e52ad"));
    }

    #[test]
    fn unwraps_bisync_nested_json() {
        // Formato real do bisync via RC: um "notice" cujo msg são linhas JSON com cores ANSI.
        let inner = [
            r#"{"time":"t1","level":"info","msg":"\u001b[2mSetting x\u001b[0m"}"#,
            r#"{"time":"t2","level":"error","msg":"falhou"}"#,
        ]
        .join("\n");
        let line = serde_json::json!({ "time": "t0", "level": "notice", "msg": inner }).to_string();
        let entries = parse_log_lines(&line);
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].level.as_str(), entries[0].msg.as_str(), entries[0].time.as_str()), ("info", "Setting x", "t1"));
        assert_eq!((entries[1].level.as_str(), entries[1].msg.as_str()), ("error", "falhou"));
    }
}
