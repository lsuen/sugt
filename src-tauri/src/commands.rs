use crate::{
    autostart,
    clients::{self, ClientsEnvStatus},
    config::{self, AppPaths},
    error_hint,
    gateway::{self, GatewayState},
    gateway_daemon,
    model::{
        AppConfig, ProviderConfig, ProviderInput, ProviderProtocol, ProviderStatus, ProviderView,
        QuitBehavior, RuntimeStatus,
    },
    trial,
    takeover_profiles::{self, TakeoverProfile, TakeoverProfileView},
};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;
use tracing::warn;

async fn gateway_is_reachable(config: &AppConfig) -> bool {
    gateway_daemon::is_reachable(&config.client_host(), config.port).await
}

async fn clients_env_with_reachability(config: &AppConfig) -> ClientsEnvStatus {
    let mut status = clients::status(config);
    status.gateway_reachable = gateway_is_reachable(config).await;
    status
}

async fn ensure_gateway_listening(
    runtime: &AppRuntime,
    auto_start: bool,
) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    if gateway_is_reachable(&config).await {
        return Ok(());
    }
    if !auto_start {
        return Err(
            "gateway_not_running:本地网关未运行，请先启动网关或使用「启动网关并接管」".to_string(),
        );
    }
    trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;

    if !runtime.gateway.is_running().await {
        if let Err(err) = runtime.gateway.start().await {
            warn!(error = %err, "embedded gateway start failed, trying detached serve");
        }
    }
    if gateway_daemon::wait_reachable(&config.client_host(), config.port, 8).await {
        return Ok(());
    }

    if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
        warn!(error = %err, "detached gateway spawn failed");
    }
    if gateway_daemon::wait_reachable(&config.client_host(), config.port, 20).await {
        sync_gateway_port_if_changed(runtime).await?;
        return Ok(());
    }

    Err(
        "启动网关失败：端口可能被占用或系统拒绝监听。请检查配置端口、关闭占用程序，或改用 9878 等端口后点击「修复接管」"
            .to_string(),
    )
}

async fn sync_gateway_port_if_changed(runtime: &AppRuntime) -> Result<(), String> {
    let gw_config = runtime.gateway.config().await;
    let mut cfg = runtime.config.write().await;
    if cfg.port == gw_config.port {
        return Ok(());
    }
    cfg.port = gw_config.port;
    let updated = cfg.clone();
    runtime.persist().await.map_err(|e| e.to_string())?;
    clients::write_launch_scripts(&runtime.paths, &updated).map_err(|e| e.to_string())?;
    clients::repair(&updated).map_err(|e| e.to_string())?;
    Ok(())
}

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
    let last_hit = runtime.gateway.last_proxy_hit().await;
    let traffic = runtime.gateway.traffic_stats().await;
    let gateway_reachable = gateway_is_reachable(&config).await;
    let public_model_id = provider.as_ref().map(|p| p.public_model_id());
    Ok(RuntimeStatus {
        running: gateway_reachable,
        listen_url: config.listen_url(),
        openai_base_url: format!("{}/v1", config.listen_url()),
        anthropic_base_url: config.listen_url(),
        gateway_client_api_key_masked: crate::model::mask_secret(&config.gateway_client_api_key),
        public_model_id,
        allow_lan_access: config.allow_lan_access,
        active_model: provider
            .as_ref()
            .map(|provider| provider.model_name.clone()),
        active_provider: provider.map(|provider| provider.name),
        last_proxy_provider: last_hit.as_ref().map(|hit| hit.provider_name.clone()),
        last_proxy_path: last_hit.as_ref().map(|hit| hit.path.clone()),
        last_proxy_failover: last_hit.as_ref().map(|hit| hit.failover).unwrap_or(false),
        last_proxy_at: last_hit.map(|hit| hit.at),
        config_dir: runtime.paths.config_dir.display().to_string(),
        log_file: runtime.paths.log_file.display().to_string(),
        trial: trial::status(&runtime.paths),
        traffic,
    })
}

#[tauri::command]
pub async fn start_gateway(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;
    let config = runtime.config.read().await.clone();
    if gateway_is_reachable(&config).await {
        return Ok(config.listen_url());
    }
    runtime
        .gateway
        .start()
        .await
        .map_err(|err| error_hint::format_gateway_start_error(&err))?;
    sync_gateway_port_if_changed(&runtime).await?;
    let config = runtime.config.read().await.clone();
    if gateway_daemon::wait_reachable(&config.client_host(), config.port, 8).await {
        sync_gateway_port_if_changed(&runtime).await?;
        return Ok(runtime.config.read().await.listen_url());
    }

    if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
        return Err(err.to_string());
    }
    if gateway_daemon::wait_reachable(&config.client_host(), config.port, 20).await {
        sync_gateway_port_if_changed(&runtime).await?;
        Ok(runtime.config.read().await.listen_url())
    } else {
        Err("启动网关失败：健康检查未通过，请查看日志或更换监听端口".to_string())
    }
}

#[tauri::command]
pub async fn stop_gateway(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    runtime.gateway.stop().await.map_err(|err| err.to_string())?;
    gateway_daemon::stop_detached(&runtime.paths.config_dir).map_err(|err| err.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn list_providers(runtime: State<'_, AppRuntime>) -> Result<Vec<ProviderView>, String> {
    let config = runtime.config.read().await;
    Ok(config.providers.iter().map(ProviderView::from).collect())
}

#[tauri::command]
pub async fn save_provider(
    runtime: State<'_, AppRuntime>,
    input: ProviderInput,
) -> Result<Vec<ProviderView>, String> {
    validate_provider_input(&input)?;
    let mut config = runtime.config.write().await;
    let id = input.id.clone().unwrap_or_default();

    if let Some(provider) = config
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
    {
        provider.name = input.name;
        provider.provider = input.provider;
        provider.base_url =
            crate::model::normalize_base_url_for_protocol(input.base_url.clone(), &input.protocol);
        if !input.api_key.trim().is_empty() && !input.api_key.contains("****") {
            provider.api_key = input.api_key;
        }
        provider.model_name = input.model_name;
        provider.model_alias = input
            .model_alias
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        provider.protocol = input.protocol.clone();
        provider.enabled = input.enabled;
    } else {
        let mut provider = ProviderConfig::new(
            input.name,
            input.provider,
            input.base_url.clone(),
            input.api_key,
            input.model_name,
        );
        provider.protocol = input.protocol.clone();
        provider.base_url =
            crate::model::normalize_base_url_for_protocol(input.base_url, &input.protocol);
        provider.model_alias = input
            .model_alias
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
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
pub async fn delete_provider(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<Vec<ProviderView>, String> {
    let mut config = runtime.config.write().await;
    config.providers.retain(|provider| provider.id != id);
    if config.active_provider_id.as_deref() == Some(&id) {
        config.active_provider_id = config
            .providers
            .iter()
            .find(|provider| provider.enabled)
            .map(|provider| provider.id.clone());
    }
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers(runtime).await
}

#[tauri::command]
pub async fn set_active_provider(runtime: State<'_, AppRuntime>, id: String) -> Result<(), String> {
    let mut config = runtime.config.write().await;
    if !config
        .providers
        .iter()
        .any(|provider| provider.enabled && provider.id == id)
    {
        return Err("模型配置不存在或未启用".to_string());
    }
    config.active_provider_id = Some(id);
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn set_provider_enabled(
    runtime: State<'_, AppRuntime>,
    id: String,
    enabled: bool,
) -> Result<Vec<ProviderView>, String> {
    let mut config = runtime.config.write().await;
    let provider = config
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .ok_or_else(|| "模型配置不存在".to_string())?;
    provider.enabled = enabled;

    if !enabled && config.active_provider_id.as_deref() == Some(&id) {
        config.active_provider_id = config
            .providers
            .iter()
            .find(|provider| provider.enabled)
            .map(|provider| provider.id.clone());
    } else if enabled && config.active_provider_id.is_none() {
        config.active_provider_id = Some(id);
    }

    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers(runtime).await
}

#[tauri::command]
pub async fn test_provider(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<ProviderStatus, String> {
    let status = runtime
        .gateway
        .test_provider(&id)
        .await
        .map_err(|err| err.to_string())?;
    let gateway_config = runtime.gateway.config().await;
    let mut config = runtime.config.write().await;
    if let Some(source) = gateway_config.providers.iter().find(|provider| provider.id == id) {
        if let Some(provider) = config.providers.iter_mut().find(|provider| provider.id == id) {
            provider.status = source.status.clone();
            provider.last_checked_at = source.last_checked_at;
            provider.last_error = source.last_error.clone();
        }
    } else if let Some(provider) = config.providers.iter_mut().find(|provider| provider.id == id) {
        provider.status = status.clone();
        provider.last_checked_at = Some(chrono::Utc::now());
    }
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(status)
}

#[derive(Deserialize)]
pub struct ListModelsInput {
    pub base_url: String,
    pub api_key: String,
    pub protocol: ProviderProtocol,
    pub vendor_id: Option<String>,
}

#[tauri::command]
pub async fn list_provider_models(
    runtime: State<'_, AppRuntime>,
    input: ListModelsInput,
) -> Result<Vec<String>, String> {
    if input.api_key.trim().is_empty() {
        return Err("请先填写 API Key".to_string());
    }
    if input.base_url.trim().is_empty() {
        return Err("请先填写 Base URL".to_string());
    }
    runtime
        .gateway
        .list_provider_models(
            input.base_url.trim(),
            input.api_key.trim(),
            input.vendor_id.as_deref(),
        )
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn set_failover(runtime: State<'_, AppRuntime>, enabled: bool) -> Result<(), String> {
    runtime.config.write().await.failover_enabled = enabled;
    runtime.persist().await.map_err(|err| err.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySettingsInput {
    pub port_fallback_enabled: Option<bool>,
    pub gateway_watchdog_enabled: Option<bool>,
    pub allow_lan_access: Option<bool>,
    pub port: Option<u16>,
    pub gateway_client_api_key: Option<String>,
}

#[tauri::command]
pub async fn update_gateway_settings(
    runtime: State<'_, AppRuntime>,
    input: GatewaySettingsInput,
) -> Result<AppConfig, String> {
    let mut config = runtime.config.write().await;
    if let Some(enabled) = input.port_fallback_enabled {
        config.port_fallback_enabled = enabled;
    }
    if let Some(enabled) = input.gateway_watchdog_enabled {
        config.gateway_watchdog_enabled = enabled;
    }
    if let Some(enabled) = input.allow_lan_access {
        config.allow_lan_access = enabled;
    }
    if let Some(port) = input.port {
        if port == 0 {
            return Err("端口不能为 0".to_string());
        }
        config.port = port;
    }
    if let Some(key) = input.gateway_client_api_key {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err("网关 API Key 不能为空".to_string());
        }
        config.gateway_client_api_key = trimmed.to_string();
    }
    let snapshot = config.clone();
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn set_autostart(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    enabled: bool,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    autostart::set_enabled(enabled, &exe).map_err(|err| err.to_string())?;
    runtime.config.write().await.autostart = enabled;
    runtime.persist().await.map_err(|err| err.to_string())?;
    let _ = app.emit("sugt://status-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn set_autostart_gateway(
    runtime: State<'_, AppRuntime>,
    enabled: bool,
) -> Result<(), String> {
    runtime.config.write().await.autostart_gateway = enabled;
    runtime.persist().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn set_quit_behavior(
    runtime: State<'_, AppRuntime>,
    behavior: QuitBehavior,
) -> Result<(), String> {
    runtime.config.write().await.quit_behavior = behavior;
    runtime.persist().await.map_err(|err| err.to_string())
}

pub async fn apply_quit_behavior(runtime: &AppRuntime) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    match config.quit_behavior {
        QuitBehavior::ExitOnly => {
            let clients_status = clients::status(&config);
            let takeover_active =
                clients_status.claude.configured || clients_status.codex.configured;
            let embedded = runtime.gateway.is_running().await;
            if takeover_active && embedded {
                if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
                    warn!(error = %err, "failed to spawn detached gateway on quit");
                } else if gateway_daemon::wait_reachable(&config.client_host(), config.port, 20).await {
                    let _ = runtime.gateway.stop().await;
                }
            }
        }
        QuitBehavior::StopGateway => {
            runtime.gateway.stop().await.map_err(|err| err.to_string())?;
            gateway_daemon::stop_detached(&runtime.paths.config_dir)
                .map_err(|err| err.to_string())?;
        }
        QuitBehavior::StopAll => {
            runtime.gateway.stop().await.map_err(|err| err.to_string())?;
            gateway_daemon::stop_detached(&runtime.paths.config_dir)
                .map_err(|err| err.to_string())?;
            clients::uninstall(&config).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

pub async fn maybe_autostart_gateway(runtime: &AppRuntime) {
    let config = runtime.config.read().await.clone();
    if !config.autostart_gateway {
        return;
    }
    if gateway_is_reachable(&config).await || runtime.gateway.is_running().await {
        return;
    }
    if trial::ensure_allowed(&runtime.paths).is_err() {
        return;
    }
    if let Err(err) = runtime.gateway.start().await {
        warn!(error = %err, "gateway autostart embedded failed");
    }
    if !gateway_is_reachable(&config).await {
        if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
            warn!(error = %err, "gateway autostart detached failed");
        }
    }
    if !gateway_daemon::wait_reachable(&config.client_host(), config.port, 20).await {
        warn!("gateway autostart: health check still failing after retries");
    }
}

async fn ensure_gateway_for_takeover(
    runtime: &AppRuntime,
    auto_start: bool,
) -> Result<(), String> {
    ensure_gateway_listening(runtime, auto_start).await
}

#[tauri::command]
pub async fn read_logs(
    runtime: State<'_, AppRuntime>,
    lines: usize,
) -> Result<Vec<String>, String> {
    crate::logging::tail(&runtime.paths.log_file, lines.min(500)).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn list_takeover_profiles(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<TakeoverProfileView>, String> {
    let config = runtime.config.read().await.clone();
    Ok(takeover_profiles::list_profile_views(&runtime.paths.config_dir, &config))
}

#[tauri::command]
pub async fn apply_takeover_profile(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
    auto_start: Option<bool>,
) -> Result<TakeoverProfileView, String> {
    ensure_gateway_for_takeover(&runtime, auto_start.unwrap_or(true)).await?;
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
    takeover_profiles::apply_profile(&runtime.paths.config_dir, &config, &profile_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn can_ai_parse_takeover(runtime: State<'_, AppRuntime>) -> Result<bool, String> {
    let config = runtime.config.read().await.clone();
    Ok(config.providers.iter().any(|p| p.enabled))
}

#[tauri::command]
pub async fn parse_takeover_config_path(
    runtime: State<'_, AppRuntime>,
    path: String,
    save: Option<bool>,
) -> Result<TakeoverProfile, String> {
    let config = runtime.config.read().await.clone();
    let path_buf = std::path::PathBuf::from(path.trim());
    if !path_buf.is_file() {
        return Err(format!("文件不存在：{}", path_buf.display()));
    }
    let profile = takeover_profiles::parse_config_file_heuristic(&path_buf, &config)
        .map_err(|e| e.to_string())?;
    if save.unwrap_or(false) {
        let mut custom = takeover_profiles::load_custom_profiles(&runtime.paths.config_dir);
        custom.retain(|p| p.id != profile.id);
        custom.push(profile.clone());
        takeover_profiles::save_custom_profiles(&runtime.paths.config_dir, &custom)
            .map_err(|e| e.to_string())?;
    }
    Ok(profile)
}

#[tauri::command]
pub async fn get_config(runtime: State<'_, AppRuntime>) -> Result<AppConfig, String> {
    Ok(runtime.config.read().await.clone())
}

#[tauri::command]
pub async fn open_config_dir(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    open_path(&runtime.paths.config_dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn get_clients_env_status(
    runtime: State<'_, AppRuntime>,
) -> Result<ClientsEnvStatus, String> {
    let config = runtime.config.read().await.clone();
    Ok(clients_env_with_reachability(&config).await)
}

#[tauri::command]
pub async fn repair_clients_env(
    runtime: State<'_, AppRuntime>,
) -> Result<ClientsEnvStatus, String> {
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|err| err.to_string())?;
    clients::repair(&config).map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&config).await)
}

#[tauri::command]
pub async fn install_clients_env(
    runtime: State<'_, AppRuntime>,
    auto_start: Option<bool>,
) -> Result<ClientsEnvStatus, String> {
    ensure_gateway_for_takeover(&runtime, auto_start.unwrap_or(false)).await?;
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|err| err.to_string())?;
    clients::install(&config).map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&config).await)
}

#[tauri::command]
pub async fn uninstall_clients_env(
    runtime: State<'_, AppRuntime>,
) -> Result<ClientsEnvStatus, String> {
    let config = runtime.config.read().await.clone();
    clients::uninstall(&config).map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&config).await)
}

fn validate_provider_input(input: &ProviderInput) -> Result<(), String> {
    if input.name.trim().is_empty()
        || input.provider.trim().is_empty()
        || input.base_url.trim().is_empty()
        || input.model_name.trim().is_empty()
    {
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
