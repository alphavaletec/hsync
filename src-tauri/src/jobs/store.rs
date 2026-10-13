//! Persistência local (SQLite) de tarefas, histórico de execuções e configurações.
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::model::{now_ms, Job, JobInput, RunRecord, RunStatus, RunTotals, Schedule, SyncMode};
use super::selection::{ItemState, Selection};
use crate::error::{AppError, AppResult};

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE jobs (
    id                 INTEGER PRIMARY KEY,
    name               TEXT    NOT NULL,
    source             TEXT    NOT NULL,
    dest               TEXT    NOT NULL,
    mode               TEXT    NOT NULL,
    filters            TEXT    NOT NULL DEFAULT '{}',
    schedule_enabled   INTEGER NOT NULL DEFAULT 0,
    interval_minutes   INTEGER NOT NULL DEFAULT 60,
    bisync_initialized INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE TABLE job_runs (
    id          INTEGER PRIMARY KEY,
    job_id      INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    status      TEXT    NOT NULL,
    dry_run     INTEGER NOT NULL DEFAULT 0,
    trigger     TEXT    NOT NULL DEFAULT 'manual',
    bytes       INTEGER NOT NULL DEFAULT 0,
    files       INTEGER NOT NULL DEFAULT 0,
    deletes     INTEGER NOT NULL DEFAULT 0,
    errors      INTEGER NOT NULL DEFAULT 0,
    error       TEXT
);
CREATE INDEX idx_job_runs_job ON job_runs(job_id, started_at DESC);
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#, r#"
ALTER TABLE jobs ADD COLUMN managed TEXT;
CREATE UNIQUE INDEX idx_jobs_managed ON jobs(managed) WHERE managed IS NOT NULL;
CREATE TABLE selections (
    remote TEXT    NOT NULL,
    path   TEXT    NOT NULL,
    is_dir INTEGER NOT NULL,
    state  TEXT    NOT NULL,
    PRIMARY KEY (remote, path)
);
"#];

const RUNS_KEPT_PER_JOB: i64 = 200;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    // ---- jobs ----

    pub fn list_jobs(&self) -> AppResult<Vec<Job>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM jobs ORDER BY name COLLATE NOCASE")?;
        let mut jobs = stmt.query_map([], job_from_row)?.collect::<Result<Vec<_>, _>>()?;
        for j in &mut jobs {
            j.last_run = last_run(&conn, j.id)?;
        }
        Ok(jobs)
    }

    pub fn get_job(&self, id: i64) -> AppResult<Job> {
        let conn = self.conn.lock().unwrap();
        let mut job = conn
            .query_row("SELECT * FROM jobs WHERE id = ?1", [id], job_from_row)
            .optional()?
            .ok_or_else(|| AppError::invalid("Tarefa não encontrada"))?;
        job.last_run = last_run(&conn, id)?;
        Ok(job)
    }

    pub fn insert_job(&self, input: &JobInput) -> AppResult<i64> {
        let conn = self.conn.lock().unwrap();
        let now = now_ms();
        conn.execute(
            "INSERT INTO jobs (name, source, dest, mode, filters, schedule_enabled, interval_minutes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                input.name,
                input.source,
                input.dest,
                input.mode.as_str(),
                serde_json::to_string(&input.filters)?,
                input.schedule.enabled,
                input.schedule.interval_minutes,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn find_managed_job(&self, key: &str) -> AppResult<Option<i64>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT id FROM jobs WHERE managed = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn insert_managed_job(&self, input: &JobInput, key: &str) -> AppResult<i64> {
        let id = self.insert_job(input)?;
        self.conn.lock().unwrap().execute("UPDATE jobs SET managed = ?1 WHERE id = ?2", params![key, id])?;
        Ok(id)
    }

    // ---- seleção por item (aba Arquivos) ----

    pub fn list_selections(&self, remote: &str) -> AppResult<Vec<Selection>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path, is_dir, state FROM selections WHERE remote = ?1 ORDER BY path")?;
        let rows = stmt
            .query_map([remote], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, String>(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(path, is_dir, state)| Some(Selection { path, is_dir, state: ItemState::parse(&state)? }))
            .collect())
    }

    pub fn set_selection(&self, remote: &str, path: &str, is_dir: bool, state: Option<ItemState>) -> AppResult<()> {
        let conn = self.conn.lock().unwrap();
        match state {
            Some(s) => conn.execute(
                "INSERT INTO selections (remote, path, is_dir, state) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(remote, path) DO UPDATE SET is_dir = excluded.is_dir, state = excluded.state",
                params![remote, path, is_dir, s.as_str()],
            )?,
            None => conn.execute("DELETE FROM selections WHERE remote = ?1 AND path = ?2", params![remote, path])?,
        };
        Ok(())
    }

    /// Remove as seleções de um estado (usado quando a tarefa gerenciada é excluída).
    pub fn clear_selections(&self, remote: &str, state: Option<ItemState>) -> AppResult<()> {
        let conn = self.conn.lock().unwrap();
        match state {
            Some(s) => conn.execute(
                "DELETE FROM selections WHERE remote = ?1 AND state = ?2",
                params![remote, s.as_str()],
            )?,
            None => conn.execute("DELETE FROM selections WHERE remote = ?1", [remote])?,
        };
        Ok(())
    }

    pub fn update_job(&self, id: i64, input: &JobInput) -> AppResult<()> {
        let conn = self.conn.lock().unwrap();
        // Mudou caminho ou modo: o estado do bisync não vale mais, exige nova resync.
        let n = conn.execute(
            "UPDATE jobs SET
                bisync_initialized = CASE WHEN source = ?2 AND dest = ?3 AND mode = ?4 THEN bisync_initialized ELSE 0 END,
                name = ?1, source = ?2, dest = ?3, mode = ?4, filters = ?5,
                schedule_enabled = ?6, interval_minutes = ?7, updated_at = ?8
             WHERE id = ?9",
            params![
                input.name,
                input.source,
                input.dest,
                input.mode.as_str(),
                serde_json::to_string(&input.filters)?,
                input.schedule.enabled,
                input.schedule.interval_minutes,
                now_ms(),
                id
            ],
        )?;
        if n == 0 {
            return Err(AppError::invalid("Tarefa não encontrada"));
        }
        Ok(())
    }

    pub fn delete_job(&self, id: i64) -> AppResult<()> {
        self.conn.lock().unwrap().execute("DELETE FROM jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_bisync_initialized(&self, id: i64, value: bool) -> AppResult<()> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE jobs SET bisync_initialized = ?1 WHERE id = ?2", params![value, id])?;
        Ok(())
    }

    /// Nomes das tarefas que usam o remote `name:` como origem ou destino.
    pub fn jobs_using_remote(&self, name: &str) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let prefix = format!("{name}:");
        let mut stmt = conn.prepare(
            "SELECT name FROM jobs WHERE substr(source, 1, length(?1)) = ?1 OR substr(dest, 1, length(?1)) = ?1",
        )?;
        let names = stmt.query_map([&prefix], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(names)
    }

    // ---- runs ----

    pub fn insert_run(&self, job_id: i64, dry_run: bool, trigger: &str) -> AppResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO job_runs (job_id, started_at, status, dry_run, trigger) VALUES (?1, ?2, 'running', ?3, ?4)",
            params![job_id, now_ms(), dry_run, trigger],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn finish_run(&self, run_id: i64, status: RunStatus, totals: &RunTotals, error: Option<&str>) -> AppResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE job_runs SET finished_at = ?1, status = ?2, bytes = ?3, files = ?4, deletes = ?5, errors = ?6, error = ?7
             WHERE id = ?8",
            params![
                now_ms(),
                status.as_str(),
                totals.bytes as i64,
                totals.files as i64,
                totals.deletes as i64,
                totals.errors as i64,
                error,
                run_id
            ],
        )?;
        conn.execute(
            "DELETE FROM job_runs WHERE job_id = (SELECT job_id FROM job_runs WHERE id = ?1)
               AND id NOT IN (SELECT id FROM job_runs WHERE job_id = (SELECT job_id FROM job_runs WHERE id = ?1)
                              ORDER BY started_at DESC LIMIT ?2)",
            params![run_id, RUNS_KEPT_PER_JOB],
        )?;
        Ok(())
    }

    pub fn list_runs(&self, job_id: Option<i64>, limit: u32) -> AppResult<Vec<RunRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM job_runs WHERE (?1 IS NULL OR job_id = ?1) ORDER BY started_at DESC LIMIT ?2",
        )?;
        let runs = stmt.query_map(params![job_id, limit], run_from_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    /// Execuções que ficaram "rodando" porque o app fechou ou travou.
    pub fn mark_running_interrupted(&self) -> AppResult<usize> {
        Ok(self.conn.lock().unwrap().execute(
            "UPDATE job_runs SET status = 'interrupted', finished_at = ?1, error = 'O aplicativo foi encerrado durante a execução'
             WHERE status = 'running'",
            [now_ms()],
        )?)
    }

    // ---- settings ----

    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn last_run(conn: &Connection, job_id: i64) -> rusqlite::Result<Option<RunRecord>> {
    conn.query_row(
        "SELECT * FROM job_runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT 1",
        [job_id],
        run_from_row,
    )
    .optional()
}

fn job_from_row(r: &Row) -> rusqlite::Result<Job> {
    let filters: String = r.get("filters")?;
    let mode: String = r.get("mode")?;
    Ok(Job {
        id: r.get("id")?,
        name: r.get("name")?,
        source: r.get("source")?,
        dest: r.get("dest")?,
        mode: SyncMode::parse(&mode),
        filters: serde_json::from_str(&filters).unwrap_or_default(),
        schedule: Schedule {
            enabled: r.get("schedule_enabled")?,
            interval_minutes: r.get("interval_minutes")?,
        },
        bisync_initialized: r.get("bisync_initialized")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        managed: r.get("managed")?,
        last_run: None,
        running: false,
        next_run_at: None,
    })
}

fn run_from_row(r: &Row) -> rusqlite::Result<RunRecord> {
    let status: String = r.get("status")?;
    Ok(RunRecord {
        id: r.get("id")?,
        job_id: r.get("job_id")?,
        started_at: r.get("started_at")?,
        finished_at: r.get("finished_at")?,
        status: RunStatus::parse(&status),
        dry_run: r.get("dry_run")?,
        trigger: r.get("trigger")?,
        bytes: r.get::<_, i64>("bytes")? as u64,
        files: r.get::<_, i64>("files")? as u64,
        deletes: r.get::<_, i64>("deletes")? as u64,
        errors: r.get::<_, i64>("errors")? as u64,
        error: r.get("error")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rclone::filters::Filters;

    fn input(name: &str) -> JobInput {
        JobInput {
            name: name.into(),
            source: "C:\\Dados".into(),
            dest: "gd:Backup".into(),
            mode: SyncMode::Bisync,
            filters: Filters { exclude_hidden: true, ..Default::default() },
            schedule: Schedule { enabled: true, interval_minutes: 30 },
        }
    }

    #[test]
    fn job_crud_and_runs() {
        let s = Store::open_in_memory().unwrap();
        let id = s.insert_job(&input("Fotos")).unwrap();
        s.set_bisync_initialized(id, true).unwrap();

        let job = s.get_job(id).unwrap();
        assert_eq!(job.mode, SyncMode::Bisync);
        assert!(job.filters.exclude_hidden);
        assert!(job.bisync_initialized);
        assert_eq!(s.jobs_using_remote("gd").unwrap(), vec!["Fotos".to_string()]);
        assert!(s.jobs_using_remote("g").unwrap().is_empty());

        // Mesmo caminho/modo mantém o estado do bisync; mudar o destino o zera.
        let mut changed = input("Fotos 2");
        s.update_job(id, &changed).unwrap();
        assert!(s.get_job(id).unwrap().bisync_initialized);
        changed.dest = "gd:Outro".into();
        s.update_job(id, &changed).unwrap();
        assert!(!s.get_job(id).unwrap().bisync_initialized);

        let run = s.insert_run(id, false, "manual").unwrap();
        assert_eq!(s.mark_running_interrupted().unwrap(), 1);
        s.finish_run(run, RunStatus::Success, &RunTotals { bytes: 10, files: 1, ..Default::default() }, None)
            .unwrap();
        let runs = s.list_runs(Some(id), 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(s.get_job(id).unwrap().last_run.unwrap().bytes, 10);

        s.delete_job(id).unwrap();
        assert!(s.list_runs(None, 10).unwrap().is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.get_setting("bwlimit").unwrap(), None);
        s.set_setting("bwlimit", "2M").unwrap();
        s.set_setting("bwlimit", "4M").unwrap();
        assert_eq!(s.get_setting("bwlimit").unwrap().as_deref(), Some("4M"));
    }
}
