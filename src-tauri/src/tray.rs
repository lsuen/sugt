use crate::{clients, commands::AppRuntime, trial};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Emitter, Manager,
};

pub fn setup(app: &mut App) -> tauri::Result<()> {
    let service = MenuItem::with_id(app, "service_status", "服务：已停止", false, None::<&str>)?;
    let provider = MenuItem::with_id(app, "provider_status", "模型：未配置", false, None::<&str>)?;
    let takeover = MenuItem::with_id(app, "takeover_status", "接管：未接管", false, None::<&str>)?;
    let trial = MenuItem::with_id(app, "trial_status", "版本：开发模式", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动网关", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止网关", true, None::<&str>)?;
    let install_clients = MenuItem::with_id(
        app,
        "install_clients",
        "开启 Claude/Codex 接管",
        true,
        None::<&str>,
    )?;
    let uninstall_clients = MenuItem::with_id(
        app,
        "uninstall_clients",
        "关闭 Claude/Codex 接管",
        true,
        None::<&str>,
    )?;
    let config_dir = MenuItem::with_id(app, "config_dir", "打开配置目录", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新状态", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &service,
            &provider,
            &takeover,
            &trial,
            &PredefinedMenuItem::separator(app)?,
            &show,
            &start,
            &stop,
            &PredefinedMenuItem::separator(app)?,
            &install_clients,
            &uninstall_clients,
            &config_dir,
            &refresh,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("sugt-tray").tooltip("SUGT - su gateway");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(&tray.app_handle());
            }
        })
        .on_menu_event(|app, event| {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                match event.id.as_ref() {
                    "show" => show_main_window(&app),
                    "start" => {
                        if let Some(runtime) = app.try_state::<AppRuntime>() {
                            if trial::ensure_allowed(&runtime.paths).is_ok() {
                                let _ = runtime.gateway.start().await;
                            }
                        }
                        let _ = app.emit("sugt://status-changed", ());
                    }
                    "stop" => {
                        if let Some(runtime) = app.try_state::<AppRuntime>() {
                            let _ = runtime.gateway.stop().await;
                        }
                        let _ = app.emit("sugt://status-changed", ());
                    }
                    "install_clients" => {
                        if let Some(runtime) = app.try_state::<AppRuntime>() {
                            let config = runtime.config.read().await.clone();
                            let _ = clients::write_launch_scripts(&runtime.paths, &config);
                            let _ = clients::install(&config);
                        }
                        let _ = app.emit("sugt://status-changed", ());
                    }
                    "uninstall_clients" => {
                        if let Some(runtime) = app.try_state::<AppRuntime>() {
                            let config = runtime.config.read().await.clone();
                            let _ = clients::uninstall(&config);
                        }
                        let _ = app.emit("sugt://status-changed", ());
                    }
                    "config_dir" => {
                        if let Some(runtime) = app.try_state::<AppRuntime>() {
                            let _ = open_path(&runtime.paths.config_dir);
                        }
                    }
                    "refresh" => {
                        let _ = app.emit("sugt://status-changed", ());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                }
                let _ = update_menu(&app).await;
            });
        })
        .build(app)?;

    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let _ = update_menu(&handle).await;
    });

    Ok(())
}

async fn update_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(runtime) = app.try_state::<AppRuntime>() {
        let running = runtime.gateway.is_running().await;
        let config = runtime.config.read().await.clone();
        let env_status = clients::status(&config);
        let trial_status = trial::status(&runtime.paths);
        let active = config
            .providers
            .iter()
            .find(|provider| config.active_provider_id.as_deref() == Some(provider.id.as_str()));

        if let Some(tray) = app.tray_by_id("sugt-tray") {
            let service_text = if running {
                "服务：运行中"
            } else {
                "服务：已停止"
            };
            let provider_text = active
                .map(|provider| format!("模型：{}", provider.name))
                .unwrap_or_else(|| "模型：未配置".to_string());
            let takeover_text = match (env_status.claude.configured, env_status.codex.configured) {
                (true, true) => "接管：Claude / Codex 已接管",
                (true, false) => "接管：Claude 已接管",
                (false, true) => "接管：Codex 已接管",
                (false, false) => "接管：未接管",
            };
            let trial_text = format!("版本：{}", trial_status.message);

            let service =
                MenuItem::with_id(app, "service_status", service_text, false, None::<&str>)?;
            let provider =
                MenuItem::with_id(app, "provider_status", provider_text, false, None::<&str>)?;
            let takeover =
                MenuItem::with_id(app, "takeover_status", takeover_text, false, None::<&str>)?;
            let trial = MenuItem::with_id(app, "trial_status", trial_text, false, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
            let start = MenuItem::with_id(
                app,
                "start",
                "启动网关",
                !running && trial_status.valid,
                None::<&str>,
            )?;
            let stop = MenuItem::with_id(app, "stop", "停止网关", running, None::<&str>)?;
            let install_clients = MenuItem::with_id(
                app,
                "install_clients",
                "开启 Claude/Codex 接管",
                true,
                None::<&str>,
            )?;
            let uninstall_clients = MenuItem::with_id(
                app,
                "uninstall_clients",
                "关闭 Claude/Codex 接管",
                true,
                None::<&str>,
            )?;
            let config_dir =
                MenuItem::with_id(app, "config_dir", "打开配置目录", true, None::<&str>)?;
            let refresh = MenuItem::with_id(app, "refresh", "刷新状态", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &service,
                    &provider,
                    &takeover,
                    &trial,
                    &PredefinedMenuItem::separator(app)?,
                    &show,
                    &start,
                    &stop,
                    &PredefinedMenuItem::separator(app)?,
                    &install_clients,
                    &uninstall_clients,
                    &config_dir,
                    &refresh,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;
            tray.set_menu(Some(menu))?;
        }
    }
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn open_path(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}
