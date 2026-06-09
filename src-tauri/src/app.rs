use crate::{commands, logging, tray};
use tauri::{Manager, WindowEvent};
use tracing::error;

pub fn run() {
    let runtime = commands::load_runtime().expect("failed to initialize SUGT runtime");
    logging::init(&runtime.paths.log_file).expect("failed to initialize logging");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(runtime)
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::start_gateway,
            commands::stop_gateway,
            commands::list_providers,
            commands::save_provider,
            commands::delete_provider,
            commands::set_active_provider,
            commands::test_provider,
            commands::set_failover,
            commands::set_autostart,
            commands::read_logs,
            commands::get_config,
            commands::open_config_dir,
        ])
        .setup(|app| {
            tray::setup(app)?;
            if let Some(window) = app.get_webview_window("main") {
                window.hide()?;
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if let Err(err) = window.hide() {
                    error!(error = %err, "failed to hide window");
                }
            }
            WindowEvent::Resized(_) => {}
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running SUGT");
}
