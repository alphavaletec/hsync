//! Ícone da bandeja, exibição da janela e fluxo de saída (o único que encerra o rclone).
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

const TRAY_ID: &str = "main";
static QUITTING: AtomicBool = AtomicBool::new(false);

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
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    app.manage(TrayItems { pause });
    Ok(())
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
