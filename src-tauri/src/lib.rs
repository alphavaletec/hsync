mod commands;
mod error;
mod jobs;
mod ondemand;
mod rclone;
mod secret;
mod state;
mod tray;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use jobs::{JobManager, Store};
use rclone::{Daemon, DaemonPaths};
use state::AppState;

/// O Tauri copia o `externalBin` para junto do executável, sem o sufixo do target triple.
fn rclone_path() -> PathBuf {
    let exe = std::env::current_exe().expect("caminho do executável indisponível");
    let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
    dir.join(if cfg!(windows) { "rclone.exe" } else { "rclone" })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Precisa ser o primeiro plugin: uma segunda instância só foca a janela existente.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let local_dir = app.path().app_local_data_dir()?;

            let store = Arc::new(Store::open(&data_dir.join("hsync.db"))?);
            let interrupted = store.mark_running_interrupted()?;
            if interrupted > 0 {
                log::warn!("{interrupted} execução(ões) interrompida(s) na sessão anterior");
            }

            let daemon = Daemon::new(
                app.handle().clone(),
                DaemonPaths {
                    rclone: rclone_path(),
                    config: data_dir.join("rclone.conf"),
                    cache: local_dir.join("cache"),
                },
                store.get_setting("bwlimit")?.unwrap_or_default(),
            );
            // Diretório curto: o bisync monta nomes de arquivo com os dois caminhos (MAX_PATH no Windows).
            let jobs = JobManager::new(app.handle().clone(), daemon.clone(), store.clone(), local_dir.join("bs"));
            let ondemand = ondemand::OnDemand::new(daemon.clone(), store.clone());

            commands::preview::clear_preview_cache(app.handle());
            tray::create(app.handle(), jobs.is_paused())?;
            app.manage(AppState {
                daemon: daemon.clone(),
                jobs: jobs.clone(),
                store,
                pending_remotes: Mutex::new(HashMap::new()),
                ondemand: ondemand.clone(),
            });

            daemon.start();
            jobs.start_background();
            // Contas com arquivos sob demanda voltam a responder ao Explorer.
            ondemand.reconnect_all();

            if std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Fechar a janela só a esconde: as tarefas agendadas continuam na bandeja.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && !tray::is_quitting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings::daemon_status,
            commands::settings::daemon_logs,
            commands::settings::daemon_retry,
            commands::settings::unlock_config,
            commands::settings::get_settings,
            commands::settings::set_bwlimit,
            commands::settings::set_master_password,
            commands::settings::remove_master_password,
            commands::accounts::list_remotes,
            commands::accounts::list_providers,
            commands::accounts::create_remote,
            commands::accounts::answer_config,
            commands::accounts::oauth_progress,
            commands::accounts::cancel_remote_config,
            commands::accounts::reauthorize_remote,
            commands::accounts::rename_remote,
            commands::ondemand::ondemand_status,
            commands::ondemand::enable_ondemand,
            commands::ondemand::disable_ondemand,
            commands::ondemand::ondemand_refresh,
            commands::accounts::test_remote,
            commands::accounts::delete_remote,
            commands::accounts::list_remote_dirs,
            commands::accounts::make_remote_dir,
            commands::files::files_config,
            commands::files::browse,
            commands::files::set_item_state,
            commands::files::set_files_base,
            commands::files::set_files_interval,
            commands::files::sync_account_now,
            commands::files::download_items,
            commands::files::upload_items,
            commands::files::free_space,
            commands::preview::preview_file,
            commands::preview::open_preview_external,
            commands::jobs::list_jobs,
            commands::jobs::create_job,
            commands::jobs::update_job,
            commands::jobs::delete_job,
            commands::jobs::run_job,
            commands::jobs::stop_job,
            commands::jobs::run_all_jobs,
            commands::jobs::list_runs,
            commands::jobs::get_progress,
            commands::jobs::set_schedules_paused,
            commands::jobs::quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("falha ao iniciar o HSync")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                if !tray::is_quitting() {
                    api.prevent_exit();
                }
            }
        });
}
