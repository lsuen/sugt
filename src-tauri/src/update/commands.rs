use crate::commands::AppRuntime;
use crate::update::{self, ApplyResult, UpdateInfo, UpdateProgress};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn check_app_update(runtime: State<'_, AppRuntime>) -> Result<UpdateInfo, String> {
    update::check_for_update(&runtime.paths.config_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_app_update(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    info: UpdateInfo,
) -> Result<ApplyResult, String> {
    let config_dir = runtime.paths.config_dir.clone();
    let app_for_progress = app.clone();
    let result = update::download_and_apply(&config_dir, &info, move |progress: UpdateProgress| {
        let _ = app_for_progress.emit("sugt://update-progress", progress);
    })
    .await
    .map_err(|e| e.to_string())?;

    if result.will_exit {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            handle.exit(0);
        });
    }
    Ok(result)
}

#[tauri::command]
pub async fn open_app_release_page(url: Option<String>) -> Result<(), String> {
    update::open_release_page(url.as_deref()).map_err(|e| e.to_string())
}
