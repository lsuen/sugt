use crate::{autostart, config::{self, AppPaths}, gateway::{self, GatewayState}, model::{AppConfig, ProviderConfig, ProviderInput, ProviderStatus, ProviderView, RuntimeStatus}};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppRuntime {
    pub paths: AppPaths,
    pub gateway: GatewayState,
    pub config: Arc<RwLock<AppConfig>>,
}

impl AppRuntime {
    pub async fn persist(&self) -> Result<()> {
        let config = self.config.read().await.clone();
        gateway::save_runtime_config(&self.paths, &config)?;
        self.gateway.replace_config(config).await;
        Ok(())
    }
}

#[tauri::command]
pub async fn get_status(runtime: State<'_, AppRuntime>) -> Result<RuntimeStatus, String> {
    let config = runtime.config.read().await.clone();
    let provider = runtime.gateway.active_provider().await;
    Ok(RuntimeStatus {
        running: runtime.gateway.is_running().await,
        listen_url: format!("http://{}:{}", config.host, config.port),
        active_model: provider.as_ref().map(|provider| provider.model_name.clone()),
        active_provider: provider.map(|provider| provider.name),
        config_dir: runtime.paths.config_dir.display().to_string(),
        log_file: runtime.paths.log_file.display().to_string(),
    })
}

#[tauri::command]
pub async fn start_gateway(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    runtime.gateway.start().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn stop_gateway(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    runtime.gateway.stop().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn list_providers(runtime: State<'_, AppRuntime>) -> Result<Vec<ProviderView>, String> {
    let config = runtime.config.read().await;
    Ok(config.providers.iter().map(ProviderView::from).collect())
}

#[tauri::command]
pub async fn save_provider(runtime: State<'_, AppRuntime>, input: ProviderInput) -> Result<Vec<ProviderView>, String> {
    validate_provider_input(&input)?;
    let mut config = runtime.config.write().await;
    let id = input.id.clone().unwrap_or_default();

    if let Some(provider) = config.providers.iter_mut().find(|provider| provider.id == id) {
        provider.name = input.name;
        provider.provider = input.provider;
        provider.base_url = crate::model::trim_base_url(input.base_url);
        if !input.api_key.trim().is_empty() && !input.api_key.contains("****") {
            provider.api_key = input.api_key;
        }
        provider.model_name = input.model_name;
        provider.enabled = input.enabled;
    } else {
        let provider = ProviderConfig::new(input.name, input.provider, input.base_url, input.api_key, input.model_name);
        if config.active_provider_id.is_none() {
            config.active_provider_id = Some(provider.id.clone());
        }
        config.providers.push(provider);
    }

    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers(runtime).await
}

#[tauri::command]
pub async fn delete_provider(runtime: State<'_, AppRuntime>, id: String) -> Result<Vec<ProviderView>, String> {
    let mut config = runtime.config.write().await;
    config.providers.retain(|provider| provider.id != id);
    if config.active_provider_id.as_deref() == Some(&id) {
        config.active_provider_id = config.providers.iter().find(|provider| provider.enabled).map(|provider| provider.id.clone());
    }
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers(runtime).await
}

#[tauri::command]
pub async fn set_active_provider(runtime: State<'_, AppRuntime>, id: String) -> Result<(), String> {
    let mut config = runtime.config.write().await;
    if !config.providers.iter().any(|provider| provider.id == id) {
        return Err("模型配置不存在".to_string());
    }
    config.active_provider_id = Some(id);
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn test_provider(runtime: State<'_, AppRuntime>, id: String) -> Result<ProviderStatus, String> {
    let status = runtime.gateway.test_provider(&id).await.map_err(|err| err.to_string())?;
    let mut config = runtime.config.write().await;
    if let Some(provider) = config.providers.iter_mut().find(|provider| provider.id == id) {
        provider.status = status.clone();
        provider.last_checked_at = Some(chrono::Utc::now());
    }
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn set_failover(runtime: State<'_, AppRuntime>, enabled: bool) -> Result<(), String> {
    runtime.config.write().await.failover_enabled = enabled;
    runtime.persist().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, runtime: State<'_, AppRuntime>, enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    autostart::set_enabled(enabled, &exe).map_err(|err| err.to_string())?;
    runtime.config.write().await.autostart = enabled;
    runtime.persist().await.map_err(|err| err.to_string())?;
    let _ = app.emit("sugt://status-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn read_logs(runtime: State<'_, AppRuntime>, lines: usize) -> Result<Vec<String>, String> {
    crate::logging::tail(&runtime.paths.log_file, lines.min(500)).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn get_config(runtime: State<'_, AppRuntime>) -> Result<AppConfig, String> {
    Ok(runtime.config.read().await.clone())
}

#[tauri::command]
pub async fn open_config_dir(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    open_path(&runtime.paths.config_dir).map_err(|err| err.to_string())
}

fn validate_provider_input(input: &ProviderInput) -> Result<(), String> {
    if input.name.trim().is_empty() || input.provider.trim().is_empty() || input.base_url.trim().is_empty() || input.model_name.trim().is_empty() {
        return Err("模型名称、服务商、Base URL 和 Model Name 不能为空".to_string());
    }
    if input.id.is_none() && input.api_key.trim().is_empty() {
        return Err("新增模型配置必须填写 API Key".to_string());
    }
    if !input.base_url.starts_with("http://") && !input.base_url.starts_with("https://") {
        return Err("Base URL 必须以 http:// 或 https:// 开头".to_string());
    }
    Ok(())
}

fn open_path(path: &std::path::Path) -> Result<()> {
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
    Err(anyhow!("当前平台不支持打开目录"))
}

pub fn load_runtime() -> Result<AppRuntime> {
    let (paths, config) = config::load_or_init_config()?;
    let gateway = GatewayState::new(config.clone())?;
    Ok(AppRuntime {
        paths,
        gateway,
        config: Arc::new(RwLock::new(config)),
    })
}
