use crate::{commands::{self, AppRuntime}, gateway_watchdog, logging, store, tray};
use tauri::{Emitter, Manager, WindowEvent};

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
            commands::restore_experimental_zen,
            commands::set_active_provider,
            commands::set_provider_enabled,
            commands::test_provider,
            commands::test_provider_draft,
            commands::list_provider_models,
            commands::set_failover,
            commands::update_gateway_settings,
            commands::set_autostart,
            commands::set_autostart_gateway,
            commands::set_auto_takeover,
            commands::set_quit_behavior,
            commands::read_logs,
            commands::get_config,
            commands::get_overlay_config,
            commands::set_overlay_config,
            commands::open_config_dir,
            commands::quick_gateway_chat,
            commands::get_clients_env_status,
            commands::install_clients_env,
            commands::repair_clients_env,
            commands::uninstall_clients_env,
            commands::list_takeover_profiles,
            commands::apply_takeover_profile,
            commands::release_takeover_profile,
            commands::delete_takeover_profile,
            commands::set_takeover_settings_path,
            commands::can_ai_parse_takeover,
            commands::parse_takeover_config_path,
            commands::save_takeover_profile,
            commands::discover_agents,
            commands::add_discovered_agents,
            store::commands::store_list_repos,
            store::commands::store_parse_repo_url,
            store::commands::store_test_repo,
            store::commands::store_add_repo,
            store::commands::store_update_repo,
            store::commands::store_remove_repo,
            store::commands::store_refresh_repo,
            store::commands::store_refresh_all_repos,
            store::commands::store_ensure_preferred_repo,
            store::commands::store_list_catalog,
            store::commands::store_install_skill,
            store::commands::store_uninstall_skill,
            store::commands::store_mount_skill,
            store::commands::store_unmount_skill,
            store::commands::store_get_plugin_panel,
            store::commands::store_get_client_paths,
            store::commands::store_test_github_proxy,
            store::commands::store_launch_client,
            store::commands::store_get_settings,
            store::commands::store_set_settings,
            store::commands::store_open_skill,
            store::commands::store_open_staging_dir,
            store::commands::store_list_local_skills,
            store::commands::store_list_skill_agents,
            store::commands::store_mount_skills,
            store::commands::store_unmount_skills,
            store::commands::store_create_local_skill,
            store::commands::get_agent_advanced,
            store::commands::import_agent_skill,
            store::commands::save_agent_advanced_path,
        ])
        .setup(|app| {
            tray::setup(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(runtime) = handle.try_state::<AppRuntime>() {
                    let watchdog_runtime = runtime.inner().clone();
                    gateway_watchdog::spawn(std::sync::Arc::new(watchdog_runtime));
                    commands::maybe_autostart_gateway(&runtime).await;
                    // 启动后恢复流量悬浮窗（若上次开启过）
                    let mut overlay_cfg = runtime.config.read().await.overlay.clone();
                    if let Err(err) = crate::overlay::apply(&handle, &mut overlay_cfg).await {
                        tracing::warn!(error = %err, "恢复流量悬浮窗失败");
                    }
                    if let Some(rt) = handle.try_state::<AppRuntime>() {
                        let mut guard = rt.config.write().await;
                        guard.overlay = overlay_cfg;
                    }
                    let _ = handle.emit("sugt://status-changed", ());

                    // 启动后后台预热推荐技能仓（Gitee），不阻塞 UI
                    let store_paths = store::paths::StorePaths::from_app(&runtime.paths);
                    let warm_handle = handle.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        match store::repos::ensure_preferred_repo_cached(&store_paths) {
                            Ok(result) if result.warmed => {
                                tracing::info!(message = %result.message, "preferred skill repo warmed");
                                let _ = warm_handle.emit("sugt://skills-catalog-changed", ());
                            }
                            Ok(_) => {}
                            Err(err) => {
                                tracing::warn!(error = %err, "preferred skill repo warm failed");
                            }
                        }
                    });
                }
            });
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
