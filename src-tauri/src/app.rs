use crate::{commands::{self, AppRuntime}, gateway_watchdog, logging, me_agent, pet, store, tray, update};
use tauri::{Emitter, Manager, WindowEvent};

pub fn run() {
    let runtime = commands::load_runtime().expect("failed to initialize SUGT runtime");
    logging::init(&runtime.paths.log_file).expect("failed to initialize logging");
    spawn_log_retention(&runtime);

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
            commands::restore_experimental_free,
            commands::probe_experimental_free_models,
            commands::list_experimental_free_candidates,
            commands::set_active_provider,
            commands::set_provider_enabled,
            commands::test_provider,
            commands::test_provider_draft,
            commands::list_provider_models,
            commands::list_model_registry,
            commands::set_model_enabled,
            commands::probe_models,
            commands::list_strategy_boxes,
            commands::save_strategy_box,
            commands::delete_strategy_box,
            commands::get_strategy_box_status,
            commands::set_failover,
            commands::update_gateway_settings,
            commands::get_lan_share_info,
            commands::get_local_access_info,
            commands::set_autostart,
            commands::set_autostart_gateway,
            commands::set_auto_takeover,
            commands::set_quit_behavior,
            commands::read_logs,
            commands::clear_cache,
            commands::get_maintenance_settings,
            commands::set_maintenance_settings,
            commands::get_config,
            commands::get_overlay_config,
            commands::set_overlay_config,
            commands::patch_overlay_config,
            commands::set_workspace_rules,
            commands::set_workspace_inject_template,
            commands::default_workspace_inject_template,
            commands::preview_workspace_rules,
            commands::test_workspace_notify,
            commands::overlay_start_drag,
            commands::focus_main_window,
            pet::pet_finish_drag,
            pet::pet_global_cursor,
            pet::pet_roam_to,
            pet::pet_open_menu,
            pet::pet_close_menu,
            pet::pet_set_clickable,
            pet::open_settings_tab,
            commands::open_config_dir,
            commands::quick_gateway_chat,
            me_agent::me_status,
            me_agent::me_set_place,
            me_agent::me_create_project,
            me_agent::me_send,
            me_agent::me_confirm,
            me_agent::me_dismiss,
            me_agent::me_stop,
            me_agent::me_clear,
            me_agent::me_open_dir,
            commands::get_quick_chat_history,
            commands::save_quick_chat_history,
            commands::clear_quick_chat_history,
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
            commands::preview_takeover,
            commands::list_agent_capabilities,
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
            store::commands::store_install_git_skills,
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
            update::commands::check_app_update,
            update::commands::apply_app_update,
            update::commands::open_app_release_page,
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
                // 先恢复上次窗口尺寸，再显示，避免用户看到尺寸跳动。
                if let Some(runtime) = app.handle().try_state::<AppRuntime>() {
                    crate::window_state::restore(&window, &runtime.paths.config_dir);
                }
                let _ = window.set_skip_taskbar(false);
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                // 只有主窗口关到托盘。菜单和悬浮窗是临时窗，关闭请求必须放行。
                if window.label() != "main" {
                    return;
                }
                api.prevent_close();
                tray::hide_main_window(window.app_handle());
            }
            WindowEvent::Resized(size) => {
                // 尺寸记忆只属于主窗口。菜单开合也会触发 Resized，不能写进主窗口记录。
                if window.label() != "main" {
                    return;
                }
                // 记住用户手动调整后的尺寸；物理像素换算回逻辑像素再落盘。
                if let Some(runtime) = window.app_handle().try_state::<AppRuntime>() {
                    let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
                    crate::window_state::schedule_save(
                        size.width as f64 / scale,
                        size.height as f64 / scale,
                        runtime.paths.config_dir.clone(),
                    );
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running SUGT");
}

/// 日志保留期清理：启动立即执行一次，之后每 24 小时一次。
/// 保留期每次从实时配置读取，设置页改完即时生效；失败只告警。
fn spawn_log_retention(runtime: &AppRuntime) {
    let runtime = runtime.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let retention_days = runtime.config.read().await.log_retention_days;
            if let Err(err) = logging::prune_logs(&runtime.paths.log_file, retention_days) {
                tracing::warn!(error = %err, "清理过期日志失败");
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}
