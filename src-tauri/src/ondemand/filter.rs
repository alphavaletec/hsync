//! Callbacks da Cloud Filter API: o Windows pede listagens de pastas e dados de arquivos
//! conforme o usuário navega e abre itens na pasta sob demanda.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cloud_filter::error::{CResult, CloudErrorKind};
use cloud_filter::filter::{info, ticket, Request, SyncFilter};
use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder_file::PlaceholderFile;
use cloud_filter::utility::WriteAt;
use nt_time::FileTime;
use serde_json::{json, Value};

use crate::rclone::client::parse_rfc3339_unix;
use crate::error::AppResult;
use crate::rclone::RcClient;

/// Fornece o cliente RC atual (o daemon pode reiniciar e trocar porta/credenciais).
pub type ClientSource = Arc<dyn Fn() -> AppResult<RcClient> + Send + Sync>;

/// Escrita em blocos de 1 MiB (múltiplo de 4 KiB, exigido pela API exceto no fim do arquivo).
const CHUNK: usize = 1024 * 1024;
const FILE_ATTRIBUTE_READONLY: u32 = 0x1;

pub struct CloudFilter {
    pub remote: String,
    pub root: PathBuf,
    pub client: ClientSource,
}

/// Caminho relativo à raiz, no formato do rclone (`/`).
fn relative(root: &Path, path: &Path) -> String {
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

fn file_time(unix: Option<i64>) -> FileTime {
    unix.and_then(|s| FileTime::from_unix_time(s).ok()).unwrap_or_else(FileTime::now)
}

impl CloudFilter {
    fn fail(&self, what: &str, e: impl std::fmt::Display) -> CloudErrorKind {
        log::warn!("sob demanda ({}): {what}: {e}", self.remote);
        #[cfg(test)]
        eprintln!("[sob demanda] {what}: {e}");
        CloudErrorKind::Unsuccessful
    }

    fn placeholders(&self, rel: &str) -> Result<Vec<PlaceholderFile>, String> {
        let client = (self.client)().map_err(|e| e.to_string())?;
        let list = tauri::async_runtime::block_on(client.call_timeout(
            "operations/list",
            json!({ "fs": format!("{}:", self.remote), "remote": rel, "opt": { "noMimeType": true } }),
            Some(std::time::Duration::from_secs(120)),
        ))
        .map_err(|e| e.to_string())?;
        let items = list.get("list").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let name = it.get("Name").and_then(Value::as_str).unwrap_or_default();
            if !windows_safe_name(name) {
                log::info!("sob demanda: \"{name}\" tem caracteres inválidos no Windows e foi omitido");
                continue;
            }
            let path = it.get("Path").and_then(Value::as_str).unwrap_or(name).to_string();
            let is_dir = it.get("IsDir").and_then(Value::as_bool).unwrap_or(false);
            let size = it.get("Size").and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
            let t = file_time(it.get("ModTime").and_then(Value::as_str).and_then(parse_rfc3339_unix));
            let ph = if is_dir {
                PlaceholderFile::new(name).metadata(Metadata::directory().created(t).written(t).changed(t).size(0))
            } else {
                // Fase 1: somente leitura (edições ainda não sobem para a nuvem).
                PlaceholderFile::new(name).has_no_children().metadata(
                    Metadata::file()
                        .created(t)
                        .written(t)
                        .changed(t)
                        .size(size)
                        .attributes(FILE_ATTRIBUTE_READONLY),
                )
            };
            out.push(ph.mark_in_sync().blob(path.into_bytes()));
        }
        Ok(out)
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
}

impl SyncFilter for CloudFilter {
    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        let rel = relative(&self.root, &request.path());
        let mut placeholders = self.placeholders(&rel).map_err(|e| self.fail("listar", e))?;
        ticket.pass_with_placeholder(&mut placeholders).map_err(|e| self.fail("criar placeholders", e))?;
        Ok(())
    }

    fn fetch_data(&self, request: Request, ticket: ticket::FetchData, info: info::FetchData) -> CResult<()> {
        let remote_path = String::from_utf8(request.file_blob().to_vec()).map_err(|e| self.fail("blob", e))?;
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

    // delete/rename: o padrão do crate é NotSupported — na fase 1 a pasta é somente leitura.
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
        let filter = CloudFilter { remote: "teste".into(), root: root.clone(), client: Arc::new(move || Ok(client.clone())) };
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
            let readonly = std::fs::metadata(r.join("a.txt")).map(|m| m.permissions().readonly()).unwrap_or(false);
            let big_read = std::fs::read(o.join("grande.bin")).unwrap_or_default();
            cmd(&["del", "/f", "/q", &r.join("a.txt").to_string_lossy()]);
            let delete_blocked = r.join("a.txt").exists();
            (names, a, readonly, big_read, delete_blocked)
        })
        .await;

        drop(conn);
        let _ = id.unregister();
        let _ = child.kill().await;

        let (names, a, readonly, big_read, delete_blocked) = result.expect("operações na pasta sob demanda");
        assert_eq!(names, vec!["Pasta X".to_string(), "a.txt".to_string()], "listagem sob demanda");
        assert_eq!(a, "conteúdo A", "abrir baixa o conteúdo");
        assert!(readonly, "fase 1: somente leitura");
        assert_eq!(big_read.len(), big.len(), "arquivo grande completo (blocos de 1 MiB)");
        assert!(big_read == big, "conteúdo do arquivo grande íntegro");
        assert!(delete_blocked, "fase 1: apagar é bloqueado");
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
