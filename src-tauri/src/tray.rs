//! Ícone da bandeja, exibição da janela e fluxo de saída (o único que encerra o rclone).
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

const TRAY_ID: &str = "main";
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Ícone da bandeja (32×32) e quadros da animação "sincronizando" (o mesmo ícone com um giro no canto).
const BASE_ICON: &[u8] = include_bytes!("../icons/32x32.png");
const FRAMES: usize = 12;
const FRAME_EVERY: Duration = Duration::from_millis(110);

struct TrayItems {
    pause: CheckMenuItem<Wry>,
}

pub fn create(app: &AppHandle, paused: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir HSync", true, None::<&str>)?;
    let run_all = MenuItem::with_id(app, "run_all", "Executar todas agora", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pausar agendamentos", true, paused, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &run_all, &pause, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("HSync")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "run_all" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<AppState>().jobs.run_all_now().await;
                });
            }
            "pause" => {
                if let Some(items) = app.try_state::<TrayItems>() {
                    let paused = items.pause.is_checked().unwrap_or(false);
                    let _ = app.state::<AppState>().jobs.set_paused(paused);
                }
            }
            "quit" => request_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = icons().map(|i| &i.synced).or_else(base_icon) {
        builder = builder.icon(icon.clone());
    } else if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(TrayItems { pause });
    Ok(())
}

fn base_icon() -> Option<&'static Image<'static>> {
    static BASE: OnceLock<Option<Image<'static>>> = OnceLock::new();
    BASE.get_or_init(|| Image::from_bytes(BASE_ICON).ok()).as_ref()
}

/// Estado mostrado no ícone da bandeja (e no botão da barra de tarefas).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayStatus {
    /// Tudo em dia: selo verde com um visto.
    Synced = 0,
    /// Sincronizando: selo com um anel girando.
    Syncing = 1,
    /// Última execução de alguma tarefa falhou, ou o rclone está parado: selo vermelho com "!".
    Error = 2,
}

static STATUS: AtomicU8 = AtomicU8::new(TrayStatus::Synced as u8);

#[derive(Clone, Copy)]
enum Glyph {
    Check,
    Exclaim,
    Spinner(f32),
}

const GREEN: [f32; 3] = [46.0, 190.0, 120.0];
const RED: [f32; 3] = [232.0, 70.0, 70.0];
const ACCENT: [f32; 3] = [100.0, 200.0, 245.0];
const DARK: [f32; 3] = [34.0, 42.0, 50.0];

/// Distância de um ponto a um segmento (para desenhar traços com bordas suaves).
fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    ((p.0 - (a.0 + t * abx)).powi(2) + (p.1 - (a.1 + t * aby)).powi(2)).sqrt()
}

/// Cópia do ícone com um selo redondo no canto inferior direito.
fn badge_icon(base: &Image<'_>, bg: [f32; 3], glyph: Glyph) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut px = base.rgba().to_vec();
    let scale = w as f32 / 32.0;
    let (cx, cy) = (w as f32 - 9.0 * scale, h as f32 - 9.0 * scale);
    let outer = 8.5 * scale;
    let tau = std::f32::consts::TAU;
    for y in 0..h {
        for x in 0..w {
            let q = (x as f32 + 0.5, y as f32 + 0.5);
            let (dx, dy) = (q.0 - cx, q.1 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            // 1 dentro do disco, caindo a 0 na borda (suaviza o serrilhado).
            let disc = (outer + 0.5 - d).clamp(0.0, 1.0);
            if disc <= 0.0 {
                continue;
            }
            // Quanto do desenho (branco, ou ciano no anel) cobre este pixel.
            let (ink, color): (f32, [f32; 3]) = match glyph {
                Glyph::Check => {
                    let a = (cx - 4.2 * scale, cy + 0.2 * scale);
                    let b = (cx - 1.2 * scale, cy + 3.2 * scale);
                    let c = (cx + 4.4 * scale, cy - 3.2 * scale);
                    let dist = seg_dist(q, a, b).min(seg_dist(q, b, c));
                    ((1.15 * scale + 0.5 - dist).clamp(0.0, 1.0), [255.0; 3])
                }
                Glyph::Exclaim => {
                    let bar = seg_dist(q, (cx, cy - 4.6 * scale), (cx, cy + 0.9 * scale));
                    let dot = (dx.powi(2) + (dy - 3.8 * scale).powi(2)).sqrt();
                    let cover = (1.25 * scale + 0.5 - bar).clamp(0.0, 1.0).max((1.4 * scale + 0.5 - dot).clamp(0.0, 1.0));
                    (cover, [255.0; 3])
                }
                Glyph::Spinner(phase) => {
                    let inner = 4.2 * scale;
                    let ring = (d - inner).clamp(0.0, 1.0) * (outer - 0.5 - d).clamp(0.0, 1.0);
                    let ang = (dy.atan2(dx) - phase * tau).rem_euclid(tau);
                    let span = tau * 0.62;
                    // Trilho fraco + arco mais forte na ponta da frente, que some na cauda.
                    (ring * (0.3 + 0.7 * if ang < span { ang / span } else { 0.0 }), ACCENT)
                }
            };
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                let badge = bg[c] * (1.0 - ink) + color[c] * ink;
                px[i + c] = (px[i + c] as f32 * (1.0 - disc) + badge * disc) as u8;
            }
            px[i + 3] = px[i + 3].max((disc * 255.0) as u8);
        }
    }
    Image::new_owned(px, w, h)
}

struct Icons {
    synced: Image<'static>,
    error: Image<'static>,
    spinner: Vec<Image<'static>>,
}

fn icons() -> Option<&'static Icons> {
    static CACHE: OnceLock<Option<Icons>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let base = base_icon()?;
            Some(Icons {
                synced: badge_icon(base, GREEN, Glyph::Check),
                error: badge_icon(base, RED, Glyph::Exclaim),
                spinner: (0..FRAMES).map(|i| badge_icon(base, DARK, Glyph::Spinner(i as f32 / FRAMES as f32))).collect(),
            })
        })
        .as_ref()
}

/// Atualiza o ícone da bandeja e a barra de progresso do botão na barra de tarefas.
/// `percent`: andamento geral (0–100) enquanto sincroniza, ou `None` se o total ainda é desconhecido.
pub fn set_status(app: &AppHandle, status: TrayStatus, percent: Option<u64>) {
    set_taskbar_progress(app, status, percent);
    let previous = STATUS.swap(status as u8, Ordering::SeqCst);
    if previous == status as u8 {
        return;
    }
    let Some(icons) = icons() else { return };
    let set = |icon: &Image<'static>| {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_icon(Some(icon.clone()));
        }
    };
    match status {
        TrayStatus::Synced => set(&icons.synced),
        TrayStatus::Error => set(&icons.error),
        TrayStatus::Syncing => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut i = 0;
                while STATUS.load(Ordering::SeqCst) == TrayStatus::Syncing as u8 {
                    if let Some(tray) = app.tray_by_id(TRAY_ID) {
                        let _ = tray.set_icon(Some(icons.spinner[i % icons.spinner.len()].clone()));
                    }
                    i += 1;
                    tokio::time::sleep(FRAME_EVERY).await;
                }
            });
        }
    }
}

fn set_taskbar_progress(app: &AppHandle, status: TrayStatus, percent: Option<u64>) {
    let Some(window) = app.get_webview_window("main") else { return };
    let state = match (status, percent) {
        (TrayStatus::Syncing, Some(p)) => ProgressBarState { status: Some(ProgressBarStatus::Normal), progress: Some(p.min(100)) },
        (TrayStatus::Syncing, None) => ProgressBarState { status: Some(ProgressBarStatus::Indeterminate), progress: None },
        // Barra vermelha cheia no botão da barra de tarefas enquanto houver erro.
        (TrayStatus::Error, _) => ProgressBarState { status: Some(ProgressBarStatus::Error), progress: Some(100) },
        (TrayStatus::Synced, _) => ProgressBarState { status: Some(ProgressBarStatus::None), progress: None },
    };
    let _ = window.set_progress_bar(state);
}

pub fn set_tooltip(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}

pub fn sync_paused_item(app: &AppHandle, paused: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.pause.set_checked(paused);
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Para as tarefas, encerra o rclone de forma limpa e só então sai do app.
pub fn request_quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        state.jobs.stop_all_and_wait().await;
        state.ondemand.disconnect_all();
        state.daemon.shutdown().await;
        app.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_differ_and_keep_the_rest_of_the_icon() {
        let base = base_icon().expect("ícone base");
        let icons = icons().expect("ícones");
        let (w, h) = (base.width() as usize, base.height() as usize);
        assert_eq!(icons.spinner.len(), FRAMES);
        assert_ne!(icons.synced.rgba(), icons.error.rgba());
        assert_ne!(icons.spinner[0].rgba(), icons.spinner[FRAMES / 2].rgba());
        // O canto superior esquerdo (onde não há selo) é igual ao ícone original em todos os estados.
        let top_left = |img: &Image<'_>| img.rgba()[..(h / 2) * w * 4 / 2].to_vec();
        for icon in [&icons.synced, &icons.error, &icons.spinner[3]] {
            assert_eq!(top_left(icon), top_left(base));
        }
    }

    /// Gera uma folha de contato ampliada para conferir o desenho à vista (HSYNC_DUMP=<arquivo.png>).
    #[test]
    fn dump_contact_sheet() {
        let Ok(out) = std::env::var("HSYNC_DUMP") else { return };
        let icons = icons().unwrap();
        let mut list: Vec<&Image<'static>> = vec![&icons.synced, &icons.error];
        list.extend(icons.spinner.iter());
        let (w, h, zoom) = (32u32, 32u32, 6u32);
        let cols = 7u32;
        let rows = (list.len() as u32).div_ceil(cols);
        let mut sheet = image::RgbaImage::from_pixel(cols * w * zoom, rows * h * zoom, image::Rgba([40, 44, 52, 255]));
        for (n, icon) in list.iter().enumerate() {
            let (ox, oy) = ((n as u32 % cols) * w * zoom, (n as u32 / cols) * h * zoom);
            for y in 0..h * zoom {
                for x in 0..w * zoom {
                    let i = (((y / zoom) * w + x / zoom) * 4) as usize;
                    let p = &icon.rgba()[i..i + 4];
                    let a = p[3] as f32 / 255.0;
                    let bg = [40.0, 44.0, 52.0];
                    let px = image::Rgba([0, 1, 2].map(|c| (p[c] as f32 * a + bg[c] * (1.0 - a)) as u8).into_iter().chain([255]).collect::<Vec<u8>>().try_into().unwrap());
                    sheet.put_pixel(ox + x, oy + y, px);
                }
            }
        }
        sheet.save(out).unwrap();
    }
}
