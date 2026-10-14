//! Pré-visualização de arquivos da aba Arquivos.
//!
//! O arquivo é colocado num cache privado (`<app_local_data>/preview`), o único diretório
//! exposto ao webview pelo asset protocol. A origem é a cópia local da conta, se estiver
//! igual; senão o rclone baixa só aquele arquivo.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use super::files::{base_for, ensure_remote};
use crate::error::{AppError, AppResult};
use crate::jobs::selection::normalize_path;
use crate::state::AppState;

/// Acima disso, só "baixar" (a pré-visualização baixaria o arquivo inteiro).
const MAX_PREVIEW_BYTES: i64 = 150 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

const IMAGE: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif", "ico"];
const AUDIO: &[&str] = &["mp3", "wav", "ogg", "oga", "m4a", "aac", "flac", "opus"];
const VIDEO: &[&str] = &["mp4", "webm", "m4v", "ogv"];
// Renderizados no frontend (docx-preview, SheetJS, pptx-renderer). Formatos binários antigos
// (.doc, .ppt) não têm renderizador local e ficam só com "abrir no aplicativo".
const SHEET: &[&str] = &["xlsx", "xlsm", "xls", "ods", "csv", "tsv"];
const TEXT: &[&str] = &[
    "txt", "md", "markdown", "log", "json", "jsonl", "xml", "yml", "yaml", "toml", "ini", "cfg", "conf",
    "env", "html", "htm", "css", "scss", "js", "mjs", "ts", "tsx", "jsx", "svelte", "vue", "py", "rs", "go", "java",
    "kt", "c", "h", "cpp", "hpp", "cs", "php", "rb", "sh", "bash", "ps1", "bat", "cmd", "sql", "srt", "vtt", "tex",
    "gitignore", "dockerfile", "properties", "rtf",
];
/// Tipos que podem ser abertos no aplicativo padrão do sistema. Lista fechada: nada que
/// execute código (exe, bat, ps1, lnk, js, vbs, msi, scr, hta, reg…).
const OPEN_EXTERNAL: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "csv", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "png", "jpg",
    "jpeg", "gif", "webp", "bmp", "heic", "avif", "tif", "tiff", "mp3", "wav", "ogg", "m4a", "flac", "aac", "mp4",
    "webm", "mkv", "mov", "avi", "m4v", "epub", "zip", "7z", "rar",
];

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    Image,
    Pdf,
    Text,
    Audio,
    Video,
    Docx,
    Sheet,
    Slides,
    Other,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub kind: PreviewKind,
    pub name: String,
    pub local_path: String,
    pub size: u64,
    pub text: Option<String>,
    pub truncated: bool,
    pub can_open_external: bool,
    /// `true` quando veio da cópia local da conta (sem download).
    pub from_local_copy: bool,
}

fn extension(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_string(),
        // ".gitignore", "Dockerfile"…
        _ => lower.trim_start_matches('.').to_string(),
    }
}

fn kind_for(ext: &str) -> PreviewKind {
    if ext == "pdf" {
        PreviewKind::Pdf
    } else if IMAGE.contains(&ext) {
        PreviewKind::Image
    } else if AUDIO.contains(&ext) {
        PreviewKind::Audio
    } else if VIDEO.contains(&ext) {
        PreviewKind::Video
    } else if ext == "docx" {
        PreviewKind::Docx
    } else if ext == "pptx" {
        PreviewKind::Slides
    } else if SHEET.contains(&ext) {
        PreviewKind::Sheet
    } else if TEXT.contains(&ext) {
        PreviewKind::Text
    } else {
        PreviewKind::Other
    }
}

/// Nome válido no Windows (provedores aceitam `: ? * |` etc. em nomes de arquivo).
fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        "arquivo".into()
    } else {
        s
    }
}

/// Hash FNV-1a: só para nomear a pasta do cache de forma estável.
pub(super) fn cache_key(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

pub fn preview_root(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|e| AppError::invalid(e.to_string()))?
        .join("preview"))
}

/// Limpa o cache ao iniciar o app (pré-visualizações são descartáveis).
pub fn clear_preview_cache(app: &AppHandle) {
    if let Ok(root) = preview_root(app) {
        let _ = std::fs::remove_dir_all(root);
    }
}

fn read_text(path: &Path) -> AppResult<Option<(String, bool)>> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(64 * 1024);
    f.by_ref().take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut buf)?;
    let truncated = buf.len() > MAX_TEXT_BYTES;
    buf.truncate(MAX_TEXT_BYTES);
    // NUL nos primeiros KB: é binário, não texto.
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Ok(None);
    }
    let text = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        // Sequência cortada no fim pela truncagem: descarta o pedaço incompleto.
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&buf[..e.valid_up_to()]).into_owned(),
        // Não é UTF-8: arquivos antigos do Windows costumam ser Latin-1/Windows-1252.
        Err(_) => buf.iter().map(|&b| b as char).collect(),
    };
    Ok(Some((text.trim_start_matches('\u{feff}').to_string(), truncated)))
}

#[tauri::command]
pub async fn preview_file(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: String,
    path: String,
    size: i64,
    mod_time: Option<String>,
) -> AppResult<Preview> {
    ensure_remote(&state, &remote).await?;
    let rel = normalize_path(&path).ok_or_else(|| AppError::invalid("Caminho inválido"))?;
    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
    let ext = extension(&name);
    let mut kind = kind_for(&ext);

    if size > MAX_PREVIEW_BYTES {
        return Err(AppError::invalid(format!(
            "Arquivo grande demais para pré-visualizar ({} MB). Use \"baixar\".",
            size / 1024 / 1024
        )));
    }

    let dir = preview_root(&app)?.join(cache_key(&[&remote, &rel, &size.to_string(), mod_time.as_deref().unwrap_or("")]));
    let target = dir.join(safe_file_name(&name));
    let mut from_local_copy = false;

    let cached = std::fs::metadata(&target).is_ok_and(|m| m.len() as i64 == size);
    if !cached {
        std::fs::create_dir_all(&dir)?;
        let (base, _) = base_for(&app, &state, &remote)?;
        let local = rel.split('/').fold(PathBuf::from(&base), |p, s| p.join(s));
        if std::fs::metadata(&local).is_ok_and(|m| m.is_file() && m.len() as i64 == size) {
            std::fs::copy(&local, &target)?;
            from_local_copy = true;
        } else {
            let client = state.daemon.client()?;
            client
                .call_timeout(
                    "operations/copyfile",
                    json!({
                        "srcFs": format!("{remote}:"),
                        "srcRemote": rel,
                        "dstFs": dir.to_string_lossy(),
                        "dstRemote": target.file_name().unwrap().to_string_lossy(),
                    }),
                    Some(Duration::from_secs(300)),
                )
                .await?;
        }
    }

    let actual = std::fs::metadata(&target)?.len();
    let mut text = None;
    let mut truncated = false;
    // Extensão desconhecida mas pequena: tenta como texto (README, LICENSE…).
    if kind == PreviewKind::Text || (kind == PreviewKind::Other && actual <= MAX_TEXT_BYTES as u64) {
        match read_text(&target)? {
            Some((t, tr)) => {
                kind = PreviewKind::Text;
                text = Some(t);
                truncated = tr;
            }
            None => kind = PreviewKind::Other,
        }
    }

    Ok(Preview {
        kind,
        name,
        local_path: target.to_string_lossy().into_owned(),
        size: actual,
        text,
        truncated,
        can_open_external: OPEN_EXTERNAL.contains(&ext.as_str()),
        from_local_copy,
    })
}

/// Abre no aplicativo padrão um arquivo do cache de pré-visualização (tipos de documento apenas).
#[tauri::command]
pub fn open_preview_external(app: AppHandle, local_path: String) -> AppResult<()> {
    let root = std::fs::canonicalize(preview_root(&app)?)?;
    let file = std::fs::canonicalize(&local_path)?;
    if !file.starts_with(&root) || !file.is_file() {
        return Err(AppError::invalid("Arquivo fora do cache de pré-visualização"));
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !OPEN_EXTERNAL.contains(&extension(&name).as_str()) {
        return Err(AppError::invalid("Por segurança, este tipo de arquivo não é aberto pelo HSync"));
    }
    app.opener()
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_for(&extension("Relatório.PDF")), PreviewKind::Pdf);
        assert_eq!(kind_for(&extension("foto.JPeG")), PreviewKind::Image);
        assert_eq!(kind_for(&extension("notas.md")), PreviewKind::Text);
        assert_eq!(kind_for(&extension(".gitignore")), PreviewKind::Text);
        assert_eq!(kind_for(&extension("planilha.xlsx")), PreviewKind::Sheet);
        assert_eq!(kind_for(&extension("dados.CSV")), PreviewKind::Sheet);
        assert_eq!(kind_for(&extension("contrato.docx")), PreviewKind::Docx);
        assert_eq!(kind_for(&extension("aula.pptx")), PreviewKind::Slides);
        assert_eq!(kind_for(&extension("antigo.doc")), PreviewKind::Other);
        assert!(!OPEN_EXTERNAL.contains(&extension("setup.exe").as_str()));
        assert!(!OPEN_EXTERNAL.contains(&extension("atalho.lnk").as_str()));
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(safe_file_name("a:b?c*.txt"), "a_b_c_.txt");
        assert_eq!(safe_file_name("fim. "), "fim");
        assert_eq!(safe_file_name("..."), "arquivo");
    }

    #[test]
    fn cache_key_is_stable_and_distinct() {
        assert_eq!(cache_key(&["a", "b"]), cache_key(&["a", "b"]));
        assert_ne!(cache_key(&["ab", ""]), cache_key(&["a", "b"]));
    }

    #[test]
    fn reads_text_and_detects_binary() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("a.txt");
        std::fs::write(&t, "\u{feff}olá\nmundo").unwrap();
        assert_eq!(read_text(&t).unwrap().unwrap().0, "olá\nmundo");
        let latin = dir.path().join("b.txt");
        std::fs::write(&latin, [b'a', 0xe7, b'b']).unwrap();
        assert_eq!(read_text(&latin).unwrap().unwrap().0, "açb");
        let bin = dir.path().join("c.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3]).unwrap();
        assert!(read_text(&bin).unwrap().is_none());
    }
}
