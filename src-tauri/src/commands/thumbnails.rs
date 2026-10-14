//! Miniaturas de imagens para a visualização em grade da aba Arquivos.
//!
//! A imagem é lida da cópia local da conta (se estiver igual) ou baixada do provedor, reduzida e
//! guardada como JPEG pequeno em `<app_local_data>/preview/thumbs` — dentro da pasta que o asset
//! protocol já expõe ao webview. Só as imagens visíveis na tela são pedidas, poucas por vez.
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::OnceLock;

use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use tauri::{AppHandle, State};
use tokio::sync::Semaphore;

use super::files::{base_for, ensure_remote};
use super::preview::{cache_key, preview_root};
use crate::error::{AppError, AppResult};
use crate::jobs::selection::normalize_path;
use crate::state::AppState;

/// Imagens maiores que isso não ganham miniatura (a tela mostra só o ícone).
const MAX_SOURCE_BYTES: i64 = 25 * 1024 * 1024;
/// Memória máxima para decodificar uma imagem (fotos de câmera de 50 MP cabem).
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;
const FORMATS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const JPEG_QUALITY: u8 = 80;
/// Downloads/decodificações simultâneos: uma pasta com centenas de fotos não pode travar o rclone.
const CONCURRENCY: usize = 3;

fn limiter() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(CONCURRENCY))
}

pub fn supports(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && FORMATS.contains(&ext.to_lowercase().as_str()))
}

/// Decodifica (respeitando a rotação do EXIF), reduz para caber em `edge`×`edge` e codifica em JPEG.
fn make_thumbnail(bytes: &[u8], edge: u32) -> AppResult<Vec<u8>> {
    let invalid = |e: image::ImageError| AppError::invalid(format!("Imagem não pôde ser lida: {e}"));
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(invalid)?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(invalid)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    // Fundo branco para PNG/WebP/GIF com transparência (JPEG não tem canal alfa).
    let rgb = if img.color().has_alpha() {
        let small = img.thumbnail(edge, edge).to_rgba8();
        let mut out = image::RgbImage::from_pixel(small.width(), small.height(), image::Rgb([255, 255, 255]));
        for (x, y, p) in small.enumerate_pixels() {
            let a = p[3] as u32;
            let blend = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
            out.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
        }
        out
    } else {
        img.thumbnail(edge, edge).to_rgb8()
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(invalid)?;
    Ok(out)
}

/// Caminho (local) da miniatura de uma imagem, ou `None` se o formato/tamanho não é suportado.
#[tauri::command]
pub async fn thumbnail(
    app: AppHandle,
    state: State<'_, AppState>,
    remote: String,
    path: String,
    size: i64,
    mod_time: Option<String>,
    edge: Option<u32>,
) -> AppResult<Option<String>> {
    let rel = normalize_path(&path).ok_or_else(|| AppError::invalid("Caminho inválido"))?;
    let name = rel.rsplit('/').next().unwrap_or(&rel);
    if !supports(name) || size > MAX_SOURCE_BYTES {
        return Ok(None);
    }
    let edge = edge.unwrap_or(256).clamp(64, 512);
    let target: PathBuf = preview_root(&app)?
        .join("thumbs")
        .join(format!("{}.jpg", cache_key(&[&remote, &rel, &size.to_string(), mod_time.as_deref().unwrap_or(""), &edge.to_string()])));
    if target.is_file() {
        return Ok(Some(target.to_string_lossy().into_owned()));
    }

    ensure_remote(&state, &remote).await?;
    let _permit = limiter().acquire().await.map_err(|e| AppError::invalid(e.to_string()))?;
    // Outra chamada pode ter feito esta miniatura enquanto esperávamos a vez.
    if target.is_file() {
        return Ok(Some(target.to_string_lossy().into_owned()));
    }

    let (base, _) = base_for(&app, &state, &remote)?;
    let local = rel.split('/').fold(PathBuf::from(&base), |p, s| p.join(s));
    let bytes = if std::fs::metadata(&local).is_ok_and(|m| m.is_file() && m.len() as i64 == size) {
        std::fs::read(&local)?
    } else {
        let client = state.daemon.client()?;
        client.get_object(&remote, &rel, None).await?.bytes().await?.to_vec()
    };

    let jpeg = tokio::task::spawn_blocking(move || make_thumbnail(&bytes, edge))
        .await
        .map_err(|e| AppError::invalid(e.to_string()))??;
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Escreve num temporário e renomeia: o webview nunca lê uma miniatura pela metade.
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, &target)?;
    Ok(Some(target.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_png(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        if alpha {
            image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 128]))
                .write_to(&mut buf, image::ImageFormat::Png)
                .unwrap();
        } else {
            image::RgbImage::from_pixel(w, h, image::Rgb([30, 120, 200]))
                .write_to(&mut buf, image::ImageFormat::Png)
                .unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn reduces_keeping_aspect_ratio() {
        let jpeg = make_thumbnail(&sample_png(1000, 500, false), 256).unwrap();
        let out = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((out.width(), out.height()), (256, 128));
    }

    #[test]
    fn never_enlarges_small_images() {
        let jpeg = make_thumbnail(&sample_png(40, 30, false), 256).unwrap();
        let out = image::load_from_memory(&jpeg).unwrap();
        assert!(out.width() <= 256 && out.height() <= 256);
    }

    #[test]
    fn transparency_is_flattened_on_white() {
        let jpeg = make_thumbnail(&sample_png(64, 64, true), 128).unwrap();
        let px = image::load_from_memory(&jpeg).unwrap().to_rgb8().get_pixel(10, 10).0;
        // vermelho a 50% sobre branco: R alto, G e B no meio-alto (não preto).
        assert!(px[0] > 200 && px[1] > 100 && px[2] > 100, "{px:?}");
    }

    /// Baixa uma imagem de um remote real (rcd isolado, pasta temporária) e gera a miniatura.
    #[tokio::test(flavor = "multi_thread")]
    async fn thumbnail_from_a_real_rclone_remote() {
        use crate::rclone::integration_tests::{rclone_bin, start_rcd};
        let Some(bin) = rclone_bin() else {
            eprintln!("binário do rclone ausente: teste pulado");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("Fotos")).unwrap();
        std::fs::write(data.join("Fotos").join("grande ç.png"), sample_png(1200, 800, false)).unwrap();
        let conf = tmp.path().join("rclone.conf");
        std::fs::write(&conf, format!("[teste]\ntype = alias\nremote = {}\n", data.to_string_lossy().replace('\\', "/"))).unwrap();
        let (mut child, client) = start_rcd(&bin, &conf).await;

        let bytes = client.get_object("teste", "Fotos/grande ç.png", None).await.unwrap().bytes().await.unwrap();
        let jpeg = make_thumbnail(&bytes, 256).unwrap();
        let out = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((out.width(), out.height()), (256, 171));

        let _ = child.kill().await;
    }

    #[test]
    fn rejects_garbage() {
        assert!(make_thumbnail(b"isto nao e uma imagem", 256).is_err());
    }

    #[test]
    fn supported_names() {
        assert!(supports("foto.JPG") && supports("a.b.png") && supports("x.webp"));
        assert!(!supports("doc.pdf") && !supports(".png") && !supports("semextensao"));
    }
}
