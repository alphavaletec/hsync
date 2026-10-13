//! Sincronização "na modificação": decide quais pastas locais observar e quais eventos do
//! sistema de arquivos disparam cada tarefa. O laço com debounce fica no `JobManager`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::model::{Endpoint, Job, SyncMode};
use super::selection::{effective_state, parse_managed_key, ItemState, Selection};

#[derive(Debug, Clone, PartialEq)]
pub struct WatchTarget {
    pub job_id: i64,
    pub root: PathBuf,
    /// Tarefas da aba Arquivos: só eventos em itens com este estado (`conta`, `estado`).
    pub scope: Option<(String, ItemState)>,
}

/// Tarefas com "na modificação" cuja pasta local é origem dos dados (⇄ ou envio).
/// Tarefas que só baixam (nuvem → PC) não reagem a mudanças locais.
pub fn watch_targets(jobs: &[Job]) -> Vec<WatchTarget> {
    let mut out = Vec::new();
    for j in jobs {
        if !j.schedule.on_change {
            continue;
        }
        let local = match (Endpoint::parse(&j.source), Endpoint::parse(&j.dest), j.mode) {
            (Some(Endpoint::Local(p)), _, _) => p,
            (_, Some(Endpoint::Local(p)), SyncMode::Bisync) => p,
            _ => continue,
        };
        let scope = j.managed.as_deref().and_then(parse_managed_key);
        out.push(WatchTarget { job_id: j.id, root: PathBuf::from(local), scope });
    }
    out
}

/// Arquivos temporários de editores e do próprio rclone não devem disparar sincronização.
fn is_temporary(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.ends_with(".partial")
        || name.ends_with(".tmp")
        || name.ends_with(".crdownload")
        || name.starts_with("~$")
        || name.starts_with(".~lock")
        || name == "desktop.ini"
        || name == "thumbs.db"
}

/// O evento em `path` deve disparar a tarefa de `target`?
pub fn event_matches(target: &WatchTarget, path: &Path, selections: &HashMap<String, Vec<Selection>>) -> bool {
    let Ok(rel) = path.strip_prefix(&target.root) else { return false };
    if rel.as_os_str().is_empty() || is_temporary(path) {
        return false;
    }
    match &target.scope {
        None => true,
        Some((remote, state)) => {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let sels = selections.get(remote).map(Vec::as_slice).unwrap_or(&[]);
            effective_state(sels, &rel) == Some(*state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::model::Schedule;
    use crate::rclone::filters::Filters;

    fn job(id: i64, source: &str, dest: &str, mode: SyncMode, on_change: bool, managed: Option<&str>) -> Job {
        Job {
            id,
            name: "t".into(),
            source: source.into(),
            dest: dest.into(),
            mode,
            filters: Filters::default(),
            schedule: Schedule { enabled: true, interval_minutes: 15, on_change },
            bisync_initialized: true,
            created_at: 0,
            updated_at: 0,
            managed: managed.map(String::from),
            last_run: None,
            running: false,
            next_run_at: None,
        }
    }

    #[test]
    fn picks_local_sources_only() {
        let jobs = vec![
            job(1, "C:/Dados", "gd:Backup", SyncMode::Copy, true, None),
            job(2, "gd:Fotos", "C:/Fotos", SyncMode::Sync, true, None), // só baixa
            job(3, "gd:x", "C:/Bi", SyncMode::Bisync, true, None),      // bisync com local no path2
            job(4, "C:/Off", "gd:y", SyncMode::Copy, false, None),     // sem "na modificação"
            job(5, "C:/HSync/mega", "mega:", SyncMode::Bisync, true, Some("files:mega:sync")),
        ];
        let t = watch_targets(&jobs);
        assert_eq!(t.iter().map(|w| w.job_id).collect::<Vec<_>>(), vec![1, 3, 5]);
        assert_eq!(t[1].root, PathBuf::from("C:/Bi"));
        assert_eq!(t[2].scope, Some(("mega".into(), ItemState::Sync)));
    }

    /// O observador real do SO entrega eventos de criar/renomear que casam com a tarefa.
    #[test]
    fn os_watcher_reports_create_and_rename() {
        use notify::Watcher;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        })
        .unwrap();
        w.watch(&root, notify::RecursiveMode::Recursive).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        std::fs::rename(root.join("a.txt"), root.join("b.txt")).unwrap();

        let target = WatchTarget { job_id: 1, root: root.clone(), scope: None };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut seen = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Ok(p) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                if event_matches(&target, &p, &HashMap::new()) {
                    seen.push(p.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
            if seen.iter().any(|n| n == "b.txt") {
                break;
            }
        }
        assert!(seen.iter().any(|n| n == "a.txt"), "criação detectada: {seen:?}");
        assert!(seen.iter().any(|n| n == "b.txt"), "renomeação detectada: {seen:?}");
    }

    #[test]
    fn matches_events_in_scope() {
        let plain = WatchTarget { job_id: 1, root: PathBuf::from("C:/Dados"), scope: None };
        let none = HashMap::new();
        assert!(event_matches(&plain, Path::new("C:/Dados/a.txt"), &none));
        assert!(!event_matches(&plain, Path::new("C:/Dados"), &none), "a raiz em si não conta");
        assert!(!event_matches(&plain, Path::new("C:/Outra/a.txt"), &none));
        assert!(!event_matches(&plain, Path::new("C:/Dados/~$relatorio.docx"), &none));
        assert!(!event_matches(&plain, Path::new("C:/Dados/foto.jpg.partial"), &none));

        let managed = WatchTarget {
            job_id: 5,
            root: PathBuf::from("C:/HSync/mega"),
            scope: Some(("mega".into(), ItemState::Sync)),
        };
        let mut sels = HashMap::new();
        sels.insert(
            "mega".to_string(),
            vec![
                Selection { path: "Clientes/DoVale".into(), is_dir: true, state: ItemState::Sync },
                Selection { path: "Clientes/DoVale/RAW".into(), is_dir: true, state: ItemState::Ignore },
            ],
        );
        assert!(event_matches(&managed, Path::new("C:/HSync/mega/Clientes/DoVale/novo.pdf"), &sels));
        assert!(!event_matches(&managed, Path::new("C:/HSync/mega/Clientes/DoVale/RAW/x.raw"), &sels));
        assert!(!event_matches(&managed, Path::new("C:/HSync/mega/Clientes/Outro/a.pdf"), &sels));
    }
}
