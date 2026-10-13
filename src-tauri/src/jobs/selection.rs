//! Seleção por item da aba Arquivos: cada arquivo/pasta de uma conta recebe um estado
//! (⇄ sincronizar, ↓ manter offline, ↑ só enviar, ⊘ ignorar). Os estados são compilados em
//! até três tarefas gerenciadas por conta (uma por direção), todas com raiz `conta:` ↔ pasta
//! base local, diferenciadas por regras de filtro do rclone.
use serde::{Deserialize, Serialize};

use super::model::{JobInput, Schedule, SyncMode};
use crate::rclone::filters::{escape_glob, Filters};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemState {
    /// Bidirecional: cópia local + mudanças dos dois lados.
    Sync,
    /// Manter offline: a pasta local espelha a nuvem.
    Download,
    /// Só enviar: o que está na pasta local é copiado para a nuvem (nunca apaga na nuvem).
    Upload,
    /// Excluído das tarefas dos itens acima dele.
    Ignore,
}

impl ItemState {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemState::Sync => "sync",
            ItemState::Download => "download",
            ItemState::Upload => "upload",
            ItemState::Ignore => "ignore",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "sync" => ItemState::Sync,
            "download" => ItemState::Download,
            "upload" => ItemState::Upload,
            "ignore" => ItemState::Ignore,
            _ => return None,
        })
    }
}

/// Estados que geram uma tarefa (o ⊘ só restringe as outras).
pub const JOB_STATES: [ItemState; 3] = [ItemState::Sync, ItemState::Download, ItemState::Upload];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    /// Caminho relativo à raiz da conta, com `/` e sem barra inicial.
    pub path: String,
    pub is_dir: bool,
    pub state: ItemState,
}

/// Chave `jobs.managed` da tarefa que implementa `state` para `remote`.
pub fn managed_key(remote: &str, state: ItemState) -> String {
    format!("files:{remote}:{}", state.as_str())
}

pub fn parse_managed_key(key: &str) -> Option<(String, ItemState)> {
    let rest = key.strip_prefix("files:")?;
    let (remote, state) = rest.rsplit_once(':')?;
    Some((remote.to_string(), ItemState::parse(state)?))
}

pub fn normalize_path(p: &str) -> Option<String> {
    let p = p.replace('\\', "/");
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() || parts.iter().any(|s| *s == "." || *s == "..") {
        return None;
    }
    Some(parts.join("/"))
}

/// Estado efetivo de um caminho: o próprio ou o da pasta ancestral marcada mais próxima.
pub fn effective_state(selections: &[Selection], path: &str) -> Option<ItemState> {
    selections
        .iter()
        .filter(|s| s.path == path || (s.is_dir && path.starts_with(&format!("{}/", s.path))))
        .max_by_key(|s| s.path.len())
        .map(|s| s.state)
}

/// Mantém uma cópia local do item (⇄ e ↓ baixam; ↑ e ⊘ não)?
pub fn keeps_local_copy(state: Option<ItemState>) -> bool {
    matches!(state, Some(ItemState::Sync | ItemState::Download))
}

fn pattern(sel: &Selection) -> String {
    let escaped: Vec<String> = sel.path.split('/').map(escape_glob).collect();
    let base = format!("/{}", escaped.join("/"));
    if sel.is_dir {
        format!("{base}/**")
    } else {
        base
    }
}

/// Regras da tarefa de `target`. O rclone usa a primeira regra que casa; ordenando do caminho
/// mais profundo para o mais raso, o estado mais específico de cada item prevalece.
pub fn compile_rules(selections: &[Selection], target: ItemState) -> Option<Vec<String>> {
    if !selections.iter().any(|s| s.state == target) {
        return None;
    }
    let mut sorted: Vec<&Selection> = selections.iter().collect();
    sorted.sort_by(|a, b| {
        let depth = |s: &Selection| s.path.matches('/').count();
        depth(b).cmp(&depth(a)).then(a.is_dir.cmp(&b.is_dir)).then(a.path.cmp(&b.path))
    });
    let mut rules: Vec<String> = sorted
        .into_iter()
        .map(|s| format!("{} {}", if s.state == target { '+' } else { '-' }, pattern(s)))
        .collect();
    rules.push("- **".into());
    Some(rules)
}

/// Monta a definição da tarefa gerenciada para uma direção.
pub fn job_input(
    remote: &str,
    base: &str,
    state: ItemState,
    rules: Vec<String>,
    interval_minutes: u32,
    on_change: bool,
) -> JobInput {
    let cloud = format!("{remote}:");
    let (name, source, dest, mode) = match state {
        // Local como path1: no resync (modo "newer") o mais recente vence de qualquer lado.
        ItemState::Sync => (format!("{remote} ⇄ sincronizados"), base.to_string(), cloud, SyncMode::Bisync),
        ItemState::Download => (format!("{remote} ↓ offline"), cloud, base.to_string(), SyncMode::Sync),
        ItemState::Upload => (format!("{remote} ↑ envio"), base.to_string(), cloud, SyncMode::Copy),
        ItemState::Ignore => unreachable!("ignorar não gera tarefa"),
    };
    JobInput {
        name,
        source,
        dest,
        mode,
        filters: Filters { custom_rules: rules, ..Default::default() },
        schedule: Schedule {
            enabled: interval_minutes > 0,
            interval_minutes,
            // ↓ offline só recebe da nuvem: alterações locais não disparam nada.
            on_change: on_change && state != ItemState::Download,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(path: &str, is_dir: bool, state: ItemState) -> Selection {
        Selection { path: path.into(), is_dir, state }
    }

    #[test]
    fn deepest_rule_wins() {
        let s = vec![
            sel("Fotos", true, ItemState::Sync),
            sel("Fotos/RAW", true, ItemState::Ignore),
            sel("Fotos/2024/capa.jpg", false, ItemState::Download),
            sel("Docs/a[1].pdf", false, ItemState::Sync),
        ];
        let sync = compile_rules(&s, ItemState::Sync).unwrap();
        assert_eq!(
            sync,
            vec!["- /Fotos/2024/capa.jpg", "+ /Docs/a\\[1\\].pdf", "- /Fotos/RAW/**", "+ /Fotos/**", "- **"]
        );
        let down = compile_rules(&s, ItemState::Download).unwrap();
        assert_eq!(down[0], "+ /Fotos/2024/capa.jpg");
        assert_eq!(down.last().unwrap(), "- **");
        assert!(compile_rules(&s, ItemState::Upload).is_none());
    }

    #[test]
    fn effective_state_inherits_from_nearest_folder() {
        let s = vec![
            sel("Fotos", true, ItemState::Download),
            sel("Fotos/RAW", true, ItemState::Ignore),
            sel("Fotos/capa.jpg", false, ItemState::Sync),
        ];
        assert_eq!(effective_state(&s, "Fotos"), Some(ItemState::Download));
        assert_eq!(effective_state(&s, "Fotos/2024/a.jpg"), Some(ItemState::Download));
        assert_eq!(effective_state(&s, "Fotos/RAW/x.raw"), Some(ItemState::Ignore));
        assert_eq!(effective_state(&s, "Fotos/capa.jpg"), Some(ItemState::Sync));
        assert_eq!(effective_state(&s, "Fotoss/a.jpg"), None, "prefixo de nome não é ancestral");
        assert_eq!(effective_state(&s, "Docs"), None);
        assert!(keeps_local_copy(Some(ItemState::Download)) && !keeps_local_copy(Some(ItemState::Upload)));
    }

    #[test]
    fn managed_keys_roundtrip() {
        let k = managed_key("meu mega", ItemState::Download);
        assert_eq!(k, "files:meu mega:download");
        assert_eq!(parse_managed_key(&k), Some(("meu mega".into(), ItemState::Download)));
        assert_eq!(parse_managed_key("outra coisa"), None);
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize_path("/Fotos\\2024/"), Some("Fotos/2024".into()));
        assert_eq!(normalize_path("a/../b"), None);
        assert_eq!(normalize_path("/"), None);
    }

    #[test]
    fn builds_jobs_per_direction() {
        let j = job_input("mega", "C:\\HSync\\mega", ItemState::Download, vec!["+ /x/**".into(), "- **".into()], 30, true);
        assert!(!j.schedule.on_change, "offline não observa a pasta local");
        assert_eq!(j.source, "mega:");
        assert_eq!(j.dest, "C:\\HSync\\mega");
        assert_eq!(j.mode, SyncMode::Sync);
        assert!(j.schedule.enabled);
        assert!(j.validate().is_ok());
        let up = job_input("mega", "C:\\HSync\\mega", ItemState::Upload, vec!["+ /x/**".into()], 0, true);
        assert!(up.schedule.on_change);
        assert_eq!(up.mode, SyncMode::Copy);
        assert!(!up.schedule.enabled);
    }
}
