//! Callbacks da Cloud Filter API: o Windows pede listagens de pastas e dados de arquivos
//! conforme o usuário navega e abre itens, e pede aprovação para apagar e renomear —
//! operações que aplicamos na nuvem antes de aprovar.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cloud_filter::error::{CResult, CloudErrorKind};
use cloud_filter::filter::{info, ticket, Request, SyncFilter};
use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder_file::PlaceholderFile;
use cloud_filter::utility::WriteAt;
use nt_time::FileTime;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::rclone::client::parse_rfc3339_unix;
use crate::rclone::RcClient;

/// Fornece o cliente RC atual (o daemon pode reiniciar e trocar porta/credenciais).
pub type ClientSource = Arc<dyn Fn() -> AppResult<RcClient> + Send + Sync>;

/// Avisos do filtro para o sincronizador da conta.
pub enum FilterEvent {
    /// O Windows listou esta pasta: passa a ser verificada periodicamente.
    Populated(String),
    /// Um item entrou na pasta vindo de fora (ex.: restaurado da Lixeira): precisa subir.
    MovedIn(PathBuf),
    /// Pasta apagada/movida para fora: deixa de ser verificada.
    Gone(String),
    /// Pasta renomeada/movida dentro da raiz: as verificações acompanham o novo caminho.
    DirMoved(String, String),
}

pub type EventSink = Arc<dyn Fn(FilterEvent) + Send + Sync>;

/// Escrita em blocos de 1 MiB (múltiplo de 4 KiB, exigido pela API exceto no fim do arquivo).
const CHUNK: usize = 1024 * 1024;

pub struct CloudFilter {
    pub remote: String,
    pub root: PathBuf,
    pub client: ClientSource,
    pub events: EventSink,
}

/// Item de uma listagem da nuvem.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudItem {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: Option<i64>,
}

impl CloudItem {
    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            name: v.get("Name")?.as_str()?.to_string(),
            is_dir: v.get("IsDir").and_then(Value::as_bool).unwrap_or(false),
            size: v.get("Size").and_then(Value::as_i64).unwrap_or(0).max(0) as u64,
            mtime: v.get("ModTime").and_then(Value::as_str).and_then(parse_rfc3339_unix),
        })
    }

    /// Versão da nuvem guardada no placeholder (para detectar mudanças e conflitos).
    pub fn blob(&self) -> Vec<u8> {
        meta_blob(self.size, self.mtime)
    }

    pub fn placeholder(&self) -> PlaceholderFile {
        let t = file_time(self.mtime);
        let ph = if self.is_dir {
            PlaceholderFile::new(&self.name).metadata(Metadata::directory().created(t).written(t).changed(t).size(0))
        } else {
            PlaceholderFile::new(&self.name)
                .has_no_children()
                .metadata(Metadata::file().created(t).written(t).changed(t).size(self.size))
        };
        ph.mark_in_sync().blob(self.blob())
    }
}

/// Os 4 bytes iniciais são enchimento: `PlaceholderInfo::blob()` do cloud-filter 0.0.6 começa a
/// ler 4 bytes depois do início real (usa o tamanho da struct, não o offset de `FileIdentity`).
/// A leitura procura o marcador `v1:`, então funciona com e sem esse defeito.
pub fn meta_blob(size: u64, mtime: Option<i64>) -> Vec<u8> {
    format!("HSYNv1:{size}:{}", mtime.unwrap_or(0)).into_bytes()
}

/// `None` para blobs da fase 1 (caminho do arquivo) ou corrompidos.
pub fn parse_meta_blob(blob: &[u8]) -> Option<(u64, i64)> {
    // O blob lido com buffer fixo vem completado com zeros.
    let end = blob.iter().position(|b| *b == 0).unwrap_or(blob.len());
    let s = std::str::from_utf8(&blob[..end]).ok()?;
    let start = s.find("v1:")?;
    let mut it = s[start + 3..].split(':');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Caminho relativo à raiz, no formato do rclone (`/`).
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
        .trim_matches('/')
        .to_string()
}

/// Nomes que o Windows não aceita (provedores permitem `: ? * |` etc.) não viram placeholder.
pub(crate) fn windows_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| "<>:\"/\\|?*".contains(c) || c.is_control())
}

pub fn file_time(unix: Option<i64>) -> FileTime {
    unix.and_then(|s| FileTime::from_unix_time(s).ok()).unwrap_or_else(FileTime::now)
}

pub fn is_not_found(e: &AppError) -> bool {
    let m = e.to_string().to_lowercase();
    m.contains("not found") || m.contains("doesn't exist") || m.contains("does not exist")
}

/// Lista uma pasta da nuvem (só nomes válidos no Windows).
pub fn list_cloud(client: &RcClient, remote: &str, rel: &str) -> AppResult<Vec<CloudItem>> {
    let list = tauri::async_runtime::block_on(client.call_timeout(
        "operations/list",
        json!({ "fs": format!("{remote}:"), "remote": rel, "opt": { "noMimeType": true } }),
        Some(Duration::from_secs(120)),
    ))?;
    Ok(list
        .get("list")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(CloudItem::from_json).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|i| {
            let ok = windows_safe_name(&i.name);
            if !ok {
                log::info!("sob demanda: \"{}\" tem caracteres inválidos no Windows e foi omitido", i.name);
            }
            ok
        })
        .collect())
}

impl CloudFilter {
    fn fail(&self, what: &str, e: impl std::fmt::Display) -> CloudErrorKind {
        log::warn!("sob demanda ({}): {what}: {e}", self.remote);
        #[cfg(test)]
        eprintln!("[sob demanda] {what}: {e}");
        CloudErrorKind::Unsuccessful
    }

    fn client(&self) -> CResult<RcClient> {
        (self.client)().map_err(|e| self.fail("rclone indisponível", e))
    }

    fn rc(&self, path: &str, params: Value) -> AppResult<Value> {
        let client = (self.client)()?;
        tauri::async_runtime::block_on(client.call_timeout(path, params, Some(Duration::from_secs(600))))
    }

    async fn download(&self, ticket: &ticket::FetchData, remote_path: &str, start: u64, end: u64, total: u64) -> Result<(), String> {
        let client = (self.client)().map_err(|e| e.to_string())?;
        let range = if start == 0 && end >= total { None } else { Some((start, end.saturating_sub(1))) };
        let mut resp = client.get_object(&self.remote, remote_path, range).await.map_err(|e| e.to_string())?;
        let mut buf: Vec<u8> = Vec::with_capacity(CHUNK);
        let mut offset = start;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            buf.extend_from_slice(&chunk);
            while buf.len() >= CHUNK {
                ticket.write_at(&buf[..CHUNK], offset).map_err(|e| e.to_string())?;
                offset += CHUNK as u64;
                buf.drain(..CHUNK);
                let _ = ticket.report_progress(total, offset);
            }
        }
        if !buf.is_empty() {
            ticket.write_at(&buf, offset).map_err(|e| e.to_string())?;
            offset += buf.len() as u64;
        }
        let _ = ticket.report_progress(total, offset);
        if offset < end {
            return Err(format!("download incompleto ({offset} de {end} bytes)"));
        }
        Ok(())
    }

    /// Apaga na nuvem. "Não encontrado" conta como sucesso (já não existe).
    fn delete_in_cloud(&self, rel: &str, is_dir: bool) -> CResult<()> {
        if rel.is_empty() {
            return Err(CloudErrorKind::AccessDenied);
        }
        let fs = format!("{}:", self.remote);
        let res = if is_dir {
            self.rc("operations/purge", json!({ "fs": fs, "remote": rel }))
        } else {
            self.rc("operations/deletefile", json!({ "fs": fs, "remote": rel }))
        };
        match res {
            Ok(_) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(self.fail(&format!("apagar {rel} na nuvem"), e)),
        }
    }

    fn move_in_cloud(&self, from: &str, to: &str, is_dir: bool) -> CResult<()> {
        let fs = format!("{}:", self.remote);
        let res = if is_dir {
            // Com destino novo, o rclone usa a renomeação de pasta do provedor quando existe (sem copiar).
            self.rc(
                "sync/move",
                json!({ "srcFs": format!("{fs}{from}"), "dstFs": format!("{fs}{to}"),
                        "createEmptySrcDirs": true, "deleteEmptySrcDirs": true }),
            )
        } else {
            self.rc("operations/movefile", json!({ "srcFs": fs, "srcRemote": from, "dstFs": fs, "dstRemote": to }))
        };
        match res {
            Ok(_) => Ok(()),
            // Ainda não estava na nuvem (ex.: arquivo novo não enviado): o sincronizador sobe com o nome novo.
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(self.fail(&format!("renomear {from} → {to} na nuvem"), e)),
        }
    }
}

impl SyncFilter for CloudFilter {
    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        let rel = relative(&self.root, &request.path());
        let client = self.client()?;
        let items = list_cloud(&client, &self.remote, &rel).map_err(|e| self.fail("listar", e))?;
        let mut placeholders: Vec<PlaceholderFile> = items.iter().map(CloudItem::placeholder).collect();
        ticket.pass_with_placeholder(&mut placeholders).map_err(|e| self.fail("criar placeholders", e))?;
        (self.events)(FilterEvent::Populated(rel));
        Ok(())
    }

    fn fetch_data(&self, request: Request, ticket: ticket::FetchData, info: info::FetchData) -> CResult<()> {
        // Caminho atual (não o blob): continua certo depois de renomeações.
        let remote_path = relative(&self.root, &request.path());
        let range = info.required_file_range();
        let total = request.file_size();
        log::info!("sob demanda: baixando {}:{remote_path} ({}..{} de {total})", self.remote, range.start, range.end);
        tauri::async_runtime::block_on(self.download(&ticket, &remote_path, range.start, range.end, total))
            .map_err(|e| self.fail(&format!("baixar {remote_path}"), e))
    }

    /// "Liberar espaço": os dados continuam na nuvem, então sempre pode.
    fn dehydrate(&self, _request: Request, ticket: ticket::Dehydrate, _info: info::Dehydrate) -> CResult<()> {
        ticket.pass().map_err(|e| self.fail("liberar espaço", e))
    }

    /// Apagar no Explorer apaga na nuvem (só aprova depois que a nuvem confirmou).
    fn delete(&self, request: Request, ticket: ticket::Delete, info: info::Delete) -> CResult<()> {
        if !info.is_undelete() {
            let rel = relative(&self.root, &request.path());
            self.delete_in_cloud(&rel, info.is_directory())?;
            if info.is_directory() {
                (self.events)(FilterEvent::Gone(rel));
            }
        }
        ticket.pass().map_err(|e| self.fail("aprovar exclusão", e))
    }

    /// Renomear/mover dentro da pasta renomeia na nuvem; mover para fora (inclusive para a
    /// Lixeira) apaga da nuvem; trazer de fora para dentro sobe o item.
    fn rename(&self, request: Request, ticket: ticket::Rename, info: info::Rename) -> CResult<()> {
        let (src_in, dst_in) = (info.source_in_scope(), info.target_in_scope());
        let src = relative(&self.root, &request.path());
        let dst = relative(&self.root, &info.target_path());
        match (src_in, dst_in) {
            (true, true) => self.move_in_cloud(&src, &dst, info.is_directory())?,
            (true, false) => self.delete_in_cloud(&src, info.is_directory())?,
            _ => {}
        }
        ticket.pass().map_err(|e| self.fail("aprovar renomeação", e))?;
        if info.is_directory() && src_in {
            (self.events)(if dst_in { FilterEvent::DirMoved(src, dst) } else { FilterEvent::Gone(src) });
        }
        if dst_in && !src_in {
            (self.events)(FilterEvent::MovedIn(info.target_path()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_use_slashes() {
        let root = PathBuf::from(r"C:\Users\x\HSync Nuvem\mega");
        assert_eq!(relative(&root, &root), "");
        assert_eq!(relative(&root, &root.join("Clientes").join("Pasta X")), "Clientes/Pasta X");
    }

    /// Integração real com o Windows: registra um sync root temporário (removido no fim),
    /// servido por um remote local num rclone isolado — nada da conta do usuário é tocado.
    #[tokio::test(flavor = "multi_thread")]
    async fn placeholders_and_hydration_with_real_windows_and_rclone() {
        use cloud_filter::root::{HydrationType, PopulationType, SecurityId, Session, SyncRootIdBuilder, SyncRootInfo};
        use crate::rclone::integration_tests::{rclone_bin, start_rcd};

        let Some(bin) = rclone_bin() else {
            eprintln!("binário do rclone ausente: teste pulado");
            return;
        };
        let tmp = tempfile::Builder::new().prefix("hsod").tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("Pasta X")).unwrap();
        std::fs::write(data.join("a.txt"), "conteúdo A").unwrap();
        let big: Vec<u8> = (0..3 * 1024 * 1024 + 123).map(|i| (i % 251) as u8).collect();
        std::fs::write(data.join("Pasta X").join("grande ç.bin"), &big).unwrap();
        let conf = tmp.path().join("rclone.conf");
        std::fs::write(&conf, format!("[teste]\ntype = alias\nremote = {}\n", data.to_string_lossy().replace('\\', "/"))).unwrap();
        let (mut child, client) = start_rcd(&bin, &conf).await;

        let root = tmp.path().join("raiz");
        std::fs::create_dir_all(&root).unwrap();
        let id = SyncRootIdBuilder::new("HSyncTest")
            .user_security_id(SecurityId::current_user().unwrap())
            .account_name(format!("t{}", std::process::id()))
            .build();
        id.register(
            SyncRootInfo::default()
                .with_display_name("HSync teste")
                .with_hydration_type(HydrationType::Full)
                .with_population_type(PopulationType::Full)
                .with_allow_pinning(true)
                .with_icon("%SystemRoot%\\system32\\imageres.dll,0")
                .with_version("1.0")
                .with_path(&root)
                .unwrap(),
        )
        .expect("registrar sync root");
        let filter = CloudFilter { remote: "teste".into(), root: root.clone(), client: Arc::new(move || Ok(client.clone())), events: Arc::new(|_| {}) };
        let conn = Session::new().connect(&root, filter).expect("conectar");

        // O Windows não dispara callbacks para I/O do próprio processo do provedor (evita deadlock):
        // as operações são feitas por outro processo, como o Explorer faria.
        let r = root.clone();
        let out_dir = tmp.path().join("copias");
        std::fs::create_dir_all(&out_dir).unwrap();
        let o = out_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let cmd = |args: &[&str]| {
                std::process::Command::new("cmd").arg("/c").args(args).output().expect("cmd")
            };
            let listing = cmd(&["dir", "/b", &r.to_string_lossy()]);
            let mut names: Vec<String> =
                String::from_utf8_lossy(&listing.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            names.sort();
            // Copiar com outro processo força o download (hidratação) do conteúdo.
            cmd(&["copy", "/y", &r.join("a.txt").to_string_lossy(), &o.join("a.txt").to_string_lossy()]);
            cmd(&["dir", "/b", &r.join("Pasta X").to_string_lossy()]);
            cmd(&["copy", "/y", &r.join("Pasta X").join("grande ç.bin").to_string_lossy(), &o.join("grande.bin").to_string_lossy()]);
            let a = std::fs::read_to_string(o.join("a.txt")).unwrap_or_default();
            let editable = std::fs::metadata(r.join("a.txt")).map(|m| !m.permissions().readonly()).unwrap_or(false);
            let big_read = std::fs::read(o.join("grande.bin")).unwrap_or_default();
            cmd(&["del", "/f", "/q", &r.join("a.txt").to_string_lossy()]);
            let deleted_locally = !r.join("a.txt").exists();
            (names, a, editable, big_read, deleted_locally)
        })
        .await;

        drop(conn);
        let _ = id.unregister();
        let _ = child.kill().await;

        let (names, a, editable, big_read, deleted_locally) = result.expect("operações na pasta sob demanda");
        assert_eq!(names, vec!["Pasta X".to_string(), "a.txt".to_string()], "listagem sob demanda");
        assert_eq!(a, "conteúdo A", "abrir baixa o conteúdo");
        assert!(editable, "arquivos sob demanda são editáveis");
        assert_eq!(big_read.len(), big.len(), "arquivo grande completo (blocos de 1 MiB)");
        assert!(big_read == big, "conteúdo do arquivo grande íntegro");
        assert!(deleted_locally && !data.join("a.txt").exists(), "apagar no PC apaga também na nuvem");
    }

    #[test]
    fn meta_blob_roundtrip() {
        let b = meta_blob(1234, Some(1_791_896_400));
        assert_eq!(parse_meta_blob(&b), Some((1234, 1_791_896_400)));
        let mut padded = b.clone();
        padded.extend([0u8; 16]);
        assert_eq!(parse_meta_blob(&padded), Some((1234, 1_791_896_400)), "zeros de preenchimento");
        assert_eq!(parse_meta_blob(&b[4..]), Some((1234, 1_791_896_400)), "leitura deslocada do crate");
        assert_eq!(parse_meta_blob(b"Clientes/a.pdf"), None, "blob da fase 1");
        assert_eq!(parse_meta_blob(b""), None);
    }

    #[test]
    fn rejects_windows_invalid_names() {
        assert!(windows_safe_name("Relatório final.pdf"));
        assert!(!windows_safe_name("a:b.txt"));
        assert!(!windows_safe_name("pergunta?.doc"));
        assert!(!windows_safe_name("fim."));
        assert!(!windows_safe_name(""));
    }
}
