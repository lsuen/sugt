use crate::{commands, logging, tray};
use tauri::{Manager, WindowEvent};

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
            commands::set_provider_enabled,
            commands::test_provider,
            commands::set_failover,
            commands::set_autostart,
            commands::read_logs,
            commands::get_config,
            commands::open_config_dir,
            commands::get_clients_env_status,
            commands::install_clients_env,
            commands::repair_clients_env,
            commands::uninstall_clients_env,
        ])
        .setup(|app| {
            tray::setup(app)?;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                tray::hide_main_window(window.app_handle());
            }
            WindowEvent::Resized(_) => {}
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running SUGT");
}
