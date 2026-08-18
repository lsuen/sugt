use crate::{
    autostart,
    clients::{self, ClientsEnvStatus},
    config::{self, AppPaths},
    error_hint,
    gateway::{self, GatewayState},
    gateway_daemon,
    model::{
        AnthropicAccessMode, AppConfig, OverlayConfig, ProviderConfig, ProviderInput,
        ProviderProtocol, ProviderStatus, ProviderView, QuitBehavior, RuntimeStatus,
    },
    trial,
    takeover_profiles::{self, TakeoverProfile, TakeoverProfileView},
};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use tracing::warn;

async fn gateway_is_reachable(config: &AppConfig) -> bool {
    gateway_daemon::is_reachable(&config.client_host(), config.port).await
}

async fn fetch_live_traffic(config: &AppConfig) -> Option<crate::gateway_stats::TrafficStatsView> {
    let url = format!(
        "{}/v1/_sugt/traffic",
        config.listen_url().trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1200))
        .build()
        .ok()?;
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

async fn clients_env_with_reachability(
    config_dir: &std::path::Path,
    config: &AppConfig,
) -> ClientsEnvStatus {
    let mut status = clients::status(config_dir, config);
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
    clients::repair(&runtime.paths.config_dir, &updated).map_err(|e| e.to_string())?;
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
        self.gateway.replace_config(config.clone()).await;
        // 仅在用户显式启用 Codex 接管时写入 ~/.codex；避免每次保存配置误伤 OpenCode
        if config.codex_takeover_enabled {
            let _ = crate::codex_config::apply_codex_config(&config);
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn get_status(runtime: State<'_, AppRuntime>) -> Result<RuntimeStatus, String> {
    let config = runtime.config.read().await.clone();
    let provider = runtime.gateway.active_provider().await;
    let local_hit = runtime.gateway.last_proxy_hit().await;
    let gateway_reachable = gateway_is_reachable(&config).await;
    let traffic = if gateway_reachable {
        fetch_live_traffic(&config)
            .await
            .unwrap_or(runtime.gateway.traffic_stats().await)
    } else {
        runtime.gateway.traffic_stats().await
    };
    // 独立/detached 网关进程的命中在远程；本地 GatewayState 可能为空，用统计里的最近来源兜底
    // （统计视图不含协议模式，fallback 时 mode 置 None，前端回退显示用户配置）
    let (
        last_proxy_provider,
        last_proxy_path,
        last_proxy_client,
        last_proxy_mode,
        last_proxy_failover,
        last_proxy_at,
    ) = if let Some(hit) = local_hit {
        (
            Some(hit.provider_name),
            Some(hit.path),
            Some(hit.client),
            Some(hit.mode),
            hit.failover,
            Some(hit.at),
        )
    } else if let Some(client) = traffic.recent_clients.first() {
        (
            Some(client.provider_name.clone()),
            Some(client.last_path.clone()),
            Some(client.label.clone()),
            None,
            false,
            Some(client.last_at),
        )
    } else {
        (None, None, None, None, false, None)
    };
    let public_model_id = provider.as_ref().map(|p| p.public_model_id());
    Ok(RuntimeStatus {
        running: gateway_reachable,
        listen_url: config.listen_url(),
        openai_base_url: format!("{}/v1", config.listen_url()),
        anthropic_base_url: config.listen_url(),
        gateway_client_api_key: config.gateway_client_api_key.clone(),
        gateway_client_api_key_masked: crate::model::mask_secret(&config.gateway_client_api_key),
        public_model_id,
        allow_lan_access: config.allow_lan_access,
        active_model: provider
            .as_ref()
            .map(|provider| provider.model_name.clone()),
        active_provider: provider.as_ref().map(|provider| provider.name.clone()),
        active_provider_id: provider.as_ref().map(|provider| provider.id.clone()),
        last_proxy_provider,
        last_proxy_path,
        last_proxy_client,
        last_proxy_mode,
        last_proxy_failover,
        last_proxy_at,
        config_dir: runtime.paths.config_dir.display().to_string(),
        log_file: runtime.paths.log_file.display().to_string(),
        trial: trial::status(&runtime.paths),
        traffic,
        anthropic_access_mode: config.anthropic_access_mode,
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
    gateway_daemon::kill_sugt_cli_processes();
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
        provider.base_url = crate::model::resolve_provider_base_url(
            input.base_url.clone(),
            &input.protocol,
            input.auto_adapt_base_url,
        );
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
        provider.auto_adapt_base_url = input.auto_adapt_base_url;
    } else {
        let mut provider = ProviderConfig::new(
            input.name,
            input.provider,
            input.base_url.clone(),
            input.api_key,
            input.model_name,
        );
        provider.protocol = input.protocol.clone();
        provider.base_url = crate::model::resolve_provider_base_url(
            input.base_url,
            &input.protocol,
            input.auto_adapt_base_url,
        );
        provider.model_alias = input
            .model_alias
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        provider.auto_adapt_base_url = input.auto_adapt_base_url;
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
    if crate::experimental_zen::is_experimental_zen_id(&id) {
        config.experimental_zen_dismissed = true;
    }
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
pub async fn restore_experimental_zen(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<ProviderView>, String> {
    {
        let mut config = runtime.config.write().await;
        crate::experimental_zen::restore_experimental_zen(&mut config);
    }
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
    #[serde(default = "default_auto_adapt_base_url")]
    pub auto_adapt_base_url: bool,
}

fn default_auto_adapt_base_url() -> bool {
    true
}

#[tauri::command]
pub async fn test_provider_draft(
    runtime: State<'_, AppRuntime>,
    input: ListModelsInput,
    model_name: String,
) -> Result<String, String> {
    if input.api_key.trim().is_empty() {
        return Err("请先填写 API Key".to_string());
    }
    if input.base_url.trim().is_empty() {
        return Err("请先填写 Base URL".to_string());
    }
    if model_name.trim().is_empty() {
        return Err("请先填写 Model Name".to_string());
    }
    let base_url = crate::model::resolve_provider_base_url(
        input.base_url.clone(),
        &input.protocol,
        input.auto_adapt_base_url,
    );
    let mut provider = ProviderConfig::new(
        "draft".to_string(),
        input.vendor_id.clone().unwrap_or_else(|| "custom".to_string()),
        base_url,
        input.api_key.trim().to_string(),
        model_name.trim().to_string(),
    );
    provider.protocol = input.protocol;
    provider.auto_adapt_base_url = input.auto_adapt_base_url;
    gateway::test_provider_connection(runtime.gateway.http_client(), &provider)
        .await
        .map_err(|err| err.to_string())?;
    Ok("连接成功".to_string())
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
    let base_url = crate::model::resolve_provider_base_url(
        input.base_url.clone(),
        &input.protocol,
        input.auto_adapt_base_url,
    );
    tracing::info!(
        raw = %input.base_url.trim(),
        resolved = %base_url,
        auto_adapt = input.auto_adapt_base_url,
        "list_provider_models"
    );
    runtime
        .gateway
        .list_provider_models(
            base_url.trim(),
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
    #[serde(rename = "anthropicAccessMode")]
    pub anthropic_access_mode: Option<AnthropicAccessMode>,
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
    if let Some(mode) = input.anthropic_access_mode {
        config.anthropic_access_mode = mode;
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
pub async fn set_auto_takeover(
    runtime: State<'_, AppRuntime>,
    enabled: bool,
) -> Result<ClientsEnvStatus, String> {
    if enabled {
        ensure_gateway_for_takeover(&runtime, true).await?;
        {
            let mut config = runtime.config.write().await;
            config.auto_takeover_enabled = true;
            config.claude_takeover_enabled = true;
        }
        runtime.persist().await.map_err(|e| e.to_string())?;
        let config = runtime.config.read().await.clone();
        clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
        clients::install_for(&runtime.paths.config_dir, &config, "claude")
            .map_err(|e| e.to_string())?;
        if config.codex_takeover_enabled {
            clients::install_for(&runtime.paths.config_dir, &config, "codex")
                .map_err(|e| e.to_string())?;
        }
        Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
    } else {
        {
            let mut config = runtime.config.write().await;
            config.auto_takeover_enabled = false;
            config.claude_takeover_enabled = false;
            config.codex_takeover_enabled = false;
        }
        runtime.persist().await.map_err(|e| e.to_string())?;
        let config = runtime.config.read().await.clone();
        clients::uninstall(&runtime.paths.config_dir, &config).map_err(|e| e.to_string())?;
        Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
    }
}

#[tauri::command]
pub async fn set_quit_behavior(
    runtime: State<'_, AppRuntime>,
    behavior: QuitBehavior,
) -> Result<(), String> {
    runtime.config.write().await.quit_behavior = behavior;
    runtime.persist().await.map_err(|err| err.to_string())
}

async fn clear_takeover_writes(runtime: &AppRuntime) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    // 方案 B：只清系统写入，保留 active_takeover_ids 偏好
    takeover_profiles::clear_active_env_writes(&runtime.paths.config_dir, &config)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn apply_quit_behavior(runtime: &AppRuntime) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    let _ = clear_takeover_writes(runtime).await;
    match config.quit_behavior {
        QuitBehavior::ExitOnly => {
            let _ = runtime.gateway.stop().await;
            gateway_daemon::stop_detached(&runtime.paths.config_dir)
                .map_err(|err| err.to_string())?;
            gateway_daemon::kill_sugt_cli_processes();
        }
        QuitBehavior::StopGateway | QuitBehavior::StopAll => {
            runtime.gateway.stop().await.map_err(|err| err.to_string())?;
            gateway_daemon::stop_detached(&runtime.paths.config_dir)
                .map_err(|err| err.to_string())?;
            gateway_daemon::kill_sugt_cli_processes();
        }
    }
    Ok(())
}

/// 旧字段 → active_takeover_ids（仅启动迁移用；日常增删不要再反向合并）
pub(crate) fn migrate_legacy_flags_into_ids(config: &mut AppConfig) {
    let mut ids = config.active_takeover_ids.clone();
    if config.claude_takeover_enabled
        && !ids.iter().any(|id| id.eq_ignore_ascii_case("claude-code"))
    {
        ids.push("claude-code".into());
    }
    if config.codex_takeover_enabled && !ids.iter().any(|id| id.eq_ignore_ascii_case("codex")) {
        ids.push("codex".into());
    }
    if config.auto_takeover_enabled
        && !ids.iter().any(|id| id.eq_ignore_ascii_case("claude-code"))
    {
        ids.push("claude-code".into());
    }
    config.active_takeover_ids = ids;
    sync_legacy_takeover_flags(config);
}

/// active_takeover_ids → 旧布尔字段（单向，以 ids 为准）
pub(crate) fn sync_legacy_takeover_flags(config: &mut AppConfig) {
    config.claude_takeover_enabled = config
        .active_takeover_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case("claude-code"));
    config.codex_takeover_enabled = config
        .active_takeover_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case("codex"));
    config.auto_takeover_enabled = false;
}

pub(crate) fn add_active_takeover_id(config: &mut AppConfig, id: &str) {
    if !config
        .active_takeover_ids
        .iter()
        .any(|x| x.eq_ignore_ascii_case(id))
    {
        config.active_takeover_ids.push(id.to_string());
    }
    sync_legacy_takeover_flags(config);
}

pub(crate) fn remove_active_takeover_id(config: &mut AppConfig, id: &str) {
    config
        .active_takeover_ids
        .retain(|x| !x.eq_ignore_ascii_case(id));
    sync_legacy_takeover_flags(config);
}

pub async fn maybe_autostart_gateway(runtime: &AppRuntime) {
    if let Err(err) = migrate_claude_only_defaults(runtime).await {
        warn!(error = %err, "claude-only migration failed");
    }
    {
        let mut config = runtime.config.write().await;
        migrate_legacy_flags_into_ids(&mut config);
    }
    let _ = runtime.persist().await;
    let config = runtime.config.read().await.clone();

    // 方案 B：按偏好恢复接管环境
    if !config.active_takeover_ids.is_empty() {
        if let Err(err) = ensure_gateway_for_takeover(runtime, true).await {
            warn!(error = %err, "restore takeover gateway failed");
        } else {
            let cfg = runtime.config.read().await.clone();
            let _ = clients::write_launch_scripts(&runtime.paths, &cfg);
            if let Err(err) =
                takeover_profiles::restore_active_profiles(&runtime.paths.config_dir, &cfg)
            {
                warn!(error = %err, "restore active takeover profiles failed");
            }
        }
    }

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

/// 一次性迁移：默认只保 Claude；清掉误写的 OPENAI/Codex；退出默认改为停网关。
async fn migrate_claude_only_defaults(runtime: &AppRuntime) -> Result<(), String> {
    let marker = runtime.paths.config_dir.join("migrated-claude-only-v1");
    if marker.exists() {
        return Ok(());
    }
    {
        let mut config = runtime.config.write().await;
        if !config.codex_takeover_enabled {
            let _ = clients::uninstall_for(&runtime.paths.config_dir, &config, "codex");
        }
        if config.quit_behavior == QuitBehavior::ExitOnly {
            config.quit_behavior = QuitBehavior::StopAll;
        }
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let _ = std::fs::write(&marker, b"1");
    Ok(())
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
        .map_err(|e| e.to_string())?;
    {
        let mut cfg = runtime.config.write().await;
        add_active_takeover_id(&mut cfg, &profile_id);
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
        .map(|p| takeover_profiles::profile_view_with_config_dir(Some(&runtime.paths.config_dir), &config, &p))
        .ok_or_else(|| format!("未找到模板：{}", profile_id))
}

#[tauri::command]
pub async fn release_takeover_profile(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
) -> Result<TakeoverProfileView, String> {
    let config = runtime.config.read().await.clone();
    takeover_profiles::release_profile(&runtime.paths.config_dir, &config, &profile_id)
        .map_err(|e| e.to_string())?;
    {
        let mut cfg = runtime.config.write().await;
        remove_active_takeover_id(&mut cfg, &profile_id);
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
        .map(|p| takeover_profiles::profile_view_with_config_dir(Some(&runtime.paths.config_dir), &config, &p))
        .ok_or_else(|| format!("未找到模板：{}", profile_id))
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeoverProfileInput {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub protocol: String,
    pub env_vars: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub clear_vars: Vec<String>,
    #[serde(default)]
    pub settings_dirs: Vec<String>,
    #[serde(default)]
    pub settings_path: Option<String>,
    #[serde(default)]
    pub skill_dirs: Vec<String>,
    #[serde(default)]
    pub launch_command: Option<String>,
}

#[tauri::command]
pub async fn discover_agents(
    runtime: State<'_, AppRuntime>,
    query: String,
) -> Result<Vec<crate::agent_discover::DiscoveredAgent>, String> {
    Ok(crate::agent_discover::discover_agents(
        &runtime.paths.config_dir,
        &query,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDiscoveredAgentInput {
    pub id: String,
    #[serde(default)]
    pub extra_path: Option<String>,
    /// 前端回传的发现快照（避免二次探测不一致）
    pub name: String,
    pub vendor: String,
    pub protocol: String,
    #[serde(default)]
    pub cli_path: Option<String>,
    #[serde(default)]
    pub settings_path: Option<String>,
    #[serde(default)]
    pub skill_dirs: Vec<String>,
    #[serde(default)]
    pub launch_command: Option<String>,
    #[serde(default)]
    pub hit_reasons: Vec<String>,
    #[serde(default)]
    pub env_vars: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub clear_vars: Vec<String>,
    #[serde(default)]
    pub requires_path: bool,
}

#[tauri::command]
pub async fn add_discovered_agents(
    runtime: State<'_, AppRuntime>,
    items: Vec<AddDiscoveredAgentInput>,
) -> Result<Vec<TakeoverProfileView>, String> {
    if items.is_empty() {
        return Err("请先选择要添加的 Agent".into());
    }
    let config = runtime.config.read().await.clone();
    let mut added = Vec::new();
    for item in items {
        if item.requires_path {
            let path = item
                .extra_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| format!("「{}」需补充配置或技能目录", item.name))?;
            let expanded = takeover_profiles::expand_home(path)
                .ok_or_else(|| format!("无法解析路径：{}", path))?;
            if !expanded.is_dir() {
                return Err(format!("目录不存在：{}", expanded.display()));
            }
        }

        let snapshot = crate::agent_discover::DiscoveredAgent {
            id: item.id.clone(),
            name: item.name.clone(),
            vendor: item.vendor.clone(),
            protocol: item.protocol.clone(),
            confidence: if item.requires_path {
                "low".into()
            } else {
                "high".into()
            },
            already_added: false,
            cli_path: item.cli_path.clone(),
            settings_path: item.settings_path.clone(),
            skill_dirs: item.skill_dirs.clone(),
            launch_command: item.launch_command.clone(),
            hit_reasons: item.hit_reasons.clone(),
            env_vars: item.env_vars.clone(),
            clear_vars: item.clear_vars.clone(),
            warning: None,
            requires_path: item.requires_path,
        };
        let profile = crate::agent_discover::discovered_to_profile(
            &snapshot,
            item.extra_path.as_deref(),
        );
        if takeover_profiles::find_profile(&runtime.paths.config_dir, &profile.id).is_some() {
            continue;
        }
        takeover_profiles::upsert_profile(&runtime.paths.config_dir, profile)
            .map_err(|e| e.to_string())?;
        if let Some(view) = takeover_profiles::find_profile(&runtime.paths.config_dir, &item.id)
            .map(|p| {
                takeover_profiles::profile_view_with_config_dir(
                    Some(&runtime.paths.config_dir),
                    &config,
                    &p,
                )
            })
        {
            added.push(view);
        }
    }
    if added.is_empty() {
        return Err("所选 Agent 均已在列表中".into());
    }
    Ok(added)
}

#[tauri::command]
pub async fn save_takeover_profile(
    runtime: State<'_, AppRuntime>,
    input: TakeoverProfileInput,
) -> Result<TakeoverProfileView, String> {
    if input.id.trim().is_empty() || input.name.trim().is_empty() {
        return Err("客户端 ID 与名称不能为空".to_string());
    }
    let id = input.id.trim().to_string();
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("客户端 ID 仅允许英文、数字、- 和 _".to_string());
    }
    let is_seed = takeover_profiles::builtin_profiles()
        .iter()
        .any(|p| p.id.eq_ignore_ascii_case(&id));
    let profile = TakeoverProfile {
        id: id.clone(),
        name: input.name.trim().to_string(),
        vendor: input.vendor.trim().to_string(),
        description: input.description.trim().to_string(),
        protocol: input.protocol.trim().to_string(),
        env_vars: input.env_vars,
        clear_vars: input.clear_vars,
        settings_dirs: input.settings_dirs,
        settings_path: input
            .settings_path
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        skill_dirs: input.skill_dirs,
        launch_command: input.launch_command,
        builtin: is_seed,
    };
    takeover_profiles::upsert_profile(&runtime.paths.config_dir, profile)
        .map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    takeover_profiles::find_profile(&runtime.paths.config_dir, &id)
        .map(|p| takeover_profiles::profile_view_with_config_dir(Some(&runtime.paths.config_dir), &config, &p))
        .ok_or_else(|| format!("保存后未找到模板: {}", id))
}

#[tauri::command]
pub async fn delete_takeover_profile(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
) -> Result<(), String> {
    // 若正在接管，先释放
    let config = runtime.config.read().await.clone();
    if config
        .active_takeover_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(&profile_id))
    {
        let _ = takeover_profiles::release_profile(
            &runtime.paths.config_dir,
            &config,
            &profile_id,
        );
        {
            let mut cfg = runtime.config.write().await;
            remove_active_takeover_id(&mut cfg, &profile_id);
        }
        let _ = runtime.persist().await;
    }
    takeover_profiles::delete_profile(&runtime.paths.config_dir, &profile_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_takeover_settings_path(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
    path: Option<String>,
) -> Result<TakeoverProfileView, String> {
    takeover_profiles::set_settings_path(&runtime.paths.config_dir, &profile_id, path)
        .map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
        .map(|p| takeover_profiles::profile_view_with_config_dir(Some(&runtime.paths.config_dir), &config, &p))
        .ok_or_else(|| format!("未找到模板：{}", profile_id))
}

#[tauri::command]
pub async fn get_config(runtime: State<'_, AppRuntime>) -> Result<AppConfig, String> {
    Ok(runtime.config.read().await.clone())
}

/// 流量悬浮窗配置（悬浮窗页面初始化时读取）。
#[tauri::command]
pub async fn get_overlay_config(runtime: State<'_, AppRuntime>) -> Result<OverlayConfig, String> {
    Ok(runtime.config.read().await.overlay.clone())
}

/// 悬浮窗开始拖动：Rust 直调 window.start_dragging()。
/// 前端 IPC 的 startDragging 受 ACL（core:window:allow-start-dragging）限制，后端直调不受影响。
#[tauri::command]
pub async fn overlay_start_drag(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    window
        .start_dragging()
        .map_err(|e| format!("开始拖动失败: {e}"))
}

/// 更新流量悬浮窗配置：持久化并即时应用（创建/更新/关闭窗口）。
#[tauri::command]
pub async fn set_overlay_config(
    runtime: State<'_, AppRuntime>,
    app: tauri::AppHandle,
    mut cfg: OverlayConfig,
) -> Result<(), String> {
    if !(0.1..=1.0).contains(&cfg.opacity) {
        return Err("悬浮窗不透明度需在 0.1~1.0 之间".to_string());
    }
    if !matches!(cfg.layout.as_str(), "column" | "row") {
        return Err("悬浮窗布局方向需为 column 或 row".to_string());
    }
    if !(120.0..=800.0).contains(&cfg.width) || !(40.0..=300.0).contains(&cfg.height) {
        return Err("悬浮窗尺寸超出合理范围".to_string());
    }
    crate::overlay::apply(&app, &mut cfg).await?;
    {
        let mut config = runtime.config.write().await;
        config.overlay = cfg.clone();
    }
    // 通知悬浮窗页面刷新编辑态/显示项（无需轮询 get_config）
    let _ = app.emit("sugt://overlay-config-changed", &cfg);
    runtime.persist().await.map_err(|err| err.to_string())
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
    Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
}

#[tauri::command]
pub async fn repair_clients_env(
    runtime: State<'_, AppRuntime>,
) -> Result<ClientsEnvStatus, String> {
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|err| err.to_string())?;
    clients::repair(&runtime.paths.config_dir, &config).map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
}

#[tauri::command]
pub async fn install_clients_env(
    runtime: State<'_, AppRuntime>,
    auto_start: Option<bool>,
    client: Option<String>,
) -> Result<ClientsEnvStatus, String> {
    ensure_gateway_for_takeover(&runtime, auto_start.unwrap_or(false)).await?;
    let target = client
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude".to_string());
    {
        let mut config = runtime.config.write().await;
        if target == "codex" || target == "all" {
            add_active_takeover_id(&mut config, "codex");
        }
        if target == "claude" || target == "all" {
            add_active_takeover_id(&mut config, "claude-code");
        }
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|err| err.to_string())?;
    clients::install_for(&runtime.paths.config_dir, &config, &target)
        .map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
}

#[tauri::command]
pub async fn uninstall_clients_env(
    runtime: State<'_, AppRuntime>,
    client: Option<String>,
) -> Result<ClientsEnvStatus, String> {
    let target = client
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "all".to_string());
    {
        let mut config = runtime.config.write().await;
        if target == "codex" || target == "all" {
            remove_active_takeover_id(&mut config, "codex");
        }
        if target == "claude" || target == "all" {
            remove_active_takeover_id(&mut config, "claude-code");
        }
        if target == "all" {
            config.active_takeover_ids.clear();
            sync_legacy_takeover_flags(&mut config);
        }
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let config = runtime.config.read().await.clone();
    clients::uninstall_for(&runtime.paths.config_dir, &config, &target)
        .map_err(|err| err.to_string())?;
    Ok(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
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
        crate::process_util::hidden_command("explorer")
            .arg(path)
            .spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        crate::process_util::hidden_command("open")
            .arg(path)
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        crate::process_util::hidden_command("xdg-open")
            .arg(path)
            .spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow!("当前平台不支持打开目录"))
}

#[tauri::command]
pub async fn quick_gateway_chat(
    runtime: State<'_, AppRuntime>,
    message: String,
    provider_id: Option<String>,
) -> Result<String, String> {
    trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("消息不能为空".to_string());
    }
    let config = runtime.config.read().await.clone();
    let provider = if let Some(id) = provider_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        config
            .providers
            .iter()
            .find(|p| p.id == id && p.enabled)
            .cloned()
            .ok_or_else(|| "所选模型不存在或未启用".to_string())?
    } else {
        runtime
            .gateway
            .active_provider()
            .await
            .or_else(|| config.providers.iter().find(|p| p.enabled).cloned())
            .ok_or_else(|| "未配置可用上游模型，请先在模型页添加并启用".to_string())?
    };

    let client = gateway::build_upstream_client(std::time::Duration::from_secs(60))
        .map_err(|e| e.to_string())?;

    let anthropic_native = matches!(provider.protocol, ProviderProtocol::Anthropic)
        && provider.base_url.contains("anthropic.com");

    let response = if anthropic_native {
        let url = gateway::build_chat_url_for_provider(&provider);
        client
            .post(&url)
            .header("x-api-key", provider.api_key.trim())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": provider.model_name,
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": message}]
            }))
            .send()
            .await
    } else {
        let url = gateway::build_chat_url_for_provider(&provider);
        let mut req = client
            .post(&url)
            .bearer_auth(provider.api_key.trim())
            .json(&serde_json::json!({
                "model": provider.model_name,
                "messages": [{"role": "user", "content": message}],
                "stream": false,
                "max_tokens": 1024
            }));
        if crate::experimental_zen::is_zen_upstream(&provider) {
            // 复用网关侧 Zen 头，避免直连探测路径不一致
            req = req
                .header(
                    reqwest::header::USER_AGENT,
                    "opencode/1.15.0 ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.13",
                )
                .header("x-opencode-client", "cli")
                .header("x-opencode-project", "global");
        }
        req.send().await
    }
    .map_err(|err| error_hint::classify_request_error(&err))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    extract_chat_text(status, &text)
}

fn extract_chat_text(status: reqwest::StatusCode, text: &str) -> Result<String, String> {
    if !status.is_success() {
        return Err(error_hint::classify_http_error(status, text));
    }
    let body: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    for pointer in [
        "/choices/0/message/content",
        "/choices/0/text",
        "/content/0/text",
    ] {
        if let Some(content) = body
            .pointer(pointer)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(content.to_string());
        }
    }
    Err(format!(
        "模型未返回文本：{}",
        text.chars().take(240).collect::<String>()
    ))
}

pub fn load_runtime() -> Result<AppRuntime> {
    let (paths, config) = config::load_or_init_config()?;
    let gateway = GatewayState::new(config.clone(), Some(paths.config_file.clone()))?;
    Ok(AppRuntime {
        paths,
        gateway,
        config: Arc::new(RwLock::new(config)),
    })
}
