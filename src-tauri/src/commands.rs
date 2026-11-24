use crate::{
    autostart,
    clients::{self, ClientsEnvStatus},
    config::{self, AppPaths},
    error_hint,
    gateway::{self, GatewayState},
    gateway_daemon,
    maintenance::CacheReport,
    model::{
        AnthropicAccessMode, AppConfig, OverlayConfig, OverlayConfigPatch, ProviderConfig,
        ProviderInput, ProviderProtocol, ProviderStatus, ProviderView, QuitBehavior,
        RuntimeStatus, WorkspaceRules, CLIENT_MODEL_ID, MAX_LOG_RETENTION_DAYS,
    },
    model_registry::{self, ModelEntry, ModelProbe},
    presets::{self, ConfigPreset},
    probe,
    quick_chat::{self, ChatHistory, ChatReply, HistoryMsg},
    request_rules::{self, RequestRule},
    strategy_box::{self, StrategyBox},
    trial,
    takeover_profiles::{self, TakeoverProfile, TakeoverProfileView},
};
use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

/// `read_logs` 单次返回上限。期望一次拉取量有硬上限，因为整份日志经 IPC 回传会拖垮前端。
const READ_LOGS_MAX_LINES: usize = 500;

/// 备选监听口默认值。期望与 `model.rs::default_port_fallback_ports()` 一致，因为
/// 那个函数是 serde 默认值来源且为私有，跨模块无法直接引用；改一处必须同步另一处。
pub(crate) const DEFAULT_PORT_FALLBACK_PORTS: [u16; 3] = [9878, 18887, 28887];

async fn gateway_is_reachable(config: &AppConfig) -> bool {
    gateway_daemon::is_reachable(&config.client_host(), config.port).await
}

/// 运行时有效配置：`config.port` 始终是用户首选口；若嵌入式网关已绑定到备选口，
/// 则把返回值里的 `port` 覆盖为实际监听口，供健康检查 / 接入信息 / 脚本使用。
/// 不写回磁盘，避免首选口被「漂」成临时口。
pub(crate) async fn config_with_listen_port(runtime: &AppRuntime) -> AppConfig {
    let mut cfg = runtime.config.read().await.clone();
    if runtime.gateway.is_running().await {
        let listen = runtime.gateway.config().await.port;
        cfg.port = listen;
        return cfg;
    }
    // 独立/detached 进程可能绑在备选口：扫候选表找可达口，仍不改 preferred。
    if let Some(port) = find_reachable_listen_port(&cfg).await {
        cfg.port = port;
    }
    cfg
}

async fn find_reachable_listen_port(config: &AppConfig) -> Option<u16> {
    for port in config.port_candidates() {
        if gateway_daemon::is_reachable(&config.client_host(), port).await {
            return Some(port);
        }
    }
    None
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
    let effective = config_with_listen_port(runtime).await;
    if gateway_is_reachable(&effective).await {
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
    let after_embed = config_with_listen_port(runtime).await;
    if gateway_daemon::wait_reachable(&after_embed.client_host(), after_embed.port, 8).await {
        let _ = sync_client_endpoints_to_listen_port(runtime).await;
        return Ok(());
    }

    if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
        warn!(error = %err, "detached gateway spawn failed");
    }
    if find_reachable_listen_port(&runtime.config.read().await.clone())
        .await
        .is_some()
    {
        sync_client_endpoints_to_listen_port(runtime).await?;
        return Ok(());
    }

    Err(format!(
        "启动网关失败：端口可能被占用或系统拒绝监听。请检查配置端口、关闭占用程序，或改用 {} 等端口后点击「修复接管」",
        DEFAULT_PORT_FALLBACK_PORTS[0]
    ))
}

/// 用实际监听口刷新接管脚本，但**绝不**把临时口写进 `config.toml` 的首选 `port`。
pub(crate) async fn sync_client_endpoints_to_listen_port(
    runtime: &AppRuntime,
) -> Result<(), String> {
    let preferred = runtime.config.read().await.port;
    let effective = config_with_listen_port(runtime).await;
    if effective.port == preferred {
        return Ok(());
    }
    clients::write_launch_scripts(&runtime.paths, &effective).map_err(|e| e.to_string())?;
    clients::repair(&runtime.paths.config_dir, &effective).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Clone)]
pub struct AppRuntime {
    pub paths: AppPaths,
    pub gateway: GatewayState,
    pub config: Arc<RwLock<AppConfig>>,
}

impl AppRuntime {
    /// 供 CLI 等无 Tauri 场景从磁盘加载运行时。
    pub fn load_from_disk() -> Result<Self, String> {
        let (paths, config) = config::load_or_init_config().map_err(|err| err.to_string())?;
        let gateway = GatewayState::new(config.clone(), Some(paths.config_file.clone()))
            .map_err(|err| err.to_string())?;
        Ok(Self {
            paths,
            gateway,
            config: Arc::new(RwLock::new(config)),
        })
    }

    pub async fn persist(&self) -> Result<()> {
        let config = self.config.read().await.clone();
        gateway::save_runtime_config(&self.paths, &config)?;
        self.gateway.replace_config(config.clone()).await;
        // 仅在用户显式启用 Codex 接管时写入 ~/.codex；避免每次保存配置误伤 OpenCode
        if config.codex_takeover_enabled {
            // SUGT 自身配置已保存，不因 Codex 端失败回滚；但必须留下可见日志。
            if let Err(err) = crate::codex_config::apply_codex_config(&config) {
                tracing::warn!(error = %err, "Codex 配置写入失败（不影响 SUGT 配置保存）");
            }
        }
        Ok(())
    }
}

/// GUI 在 setup 时 manage 此句柄；command 借它即时 start/stop WebUI，进程退出时 stop。
pub type WebUiHandle = Arc<Mutex<Option<crate::webui::WebUiServer>>>;

pub fn new_webui_handle() -> WebUiHandle {
    Arc::new(Mutex::new(None))
}

fn webui_settings_view(enabled: bool, port: u16, allow_lan: bool) -> WebUiSettingsView {
    // 未开启「局域网访问」时 WebUI 只绑回环，此时列出 LAN 地址等于展示不可达入口。
    let mut lan_addresses = Vec::new();
    if enabled {
        lan_addresses.push(format!("http://127.0.0.1:{port}"));
        if allow_lan {
            lan_addresses.extend(
                crate::lan::lan_ipv4_addrs()
                    .into_iter()
                    .map(|ip| format!("http://{ip}:{port}")),
            );
        }
    }
    WebUiSettingsView {
        enabled,
        port,
        lan_addresses,
    }
}

#[tauri::command]
pub async fn get_status(runtime: State<'_, AppRuntime>) -> Result<RuntimeStatus, String> {
    get_status_core(runtime.inner()).await
}

pub(crate) async fn get_status_core(runtime: &AppRuntime) -> Result<RuntimeStatus, String> {
    let preferred = runtime.config.read().await.clone();
    let config = config_with_listen_port(runtime).await;
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
        last_proxy_workspace_rules,
    ) = if let Some(hit) = local_hit {
        (
            Some(hit.provider_name),
            Some(hit.path),
            Some(hit.client),
            Some(hit.mode),
            hit.failover,
            Some(hit.at),
            Some(hit.workspace_rules_injected),
        )
    } else if let Some(client) = traffic.recent_clients.first() {
        (
            Some(client.provider_name.clone()),
            Some(client.last_path.clone()),
            Some(client.label.clone()),
            None,
            false,
            Some(client.last_at),
            None,
        )
    } else {
        (None, None, None, None, false, None, None)
    };
    let public_model_id = provider.as_ref().map(|p| p.public_model_id());
    Ok(RuntimeStatus {
        running: gateway_reachable,
        listen_url: config.listen_url(),
        preferred_port: preferred.port,
        listen_port: config.port,
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
        last_proxy_workspace_rules,
        config_dir: runtime.paths.config_dir.display().to_string(),
        log_file: runtime.paths.log_file.display().to_string(),
        trial: trial::status(&runtime.paths),
        traffic,
        anthropic_access_mode: config.anthropic_access_mode,
    })
}

pub(crate) async fn ensure_gateway_running(runtime: &AppRuntime) -> Result<String, String> {
    trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;
    let effective = config_with_listen_port(runtime).await;
    if gateway_is_reachable(&effective).await {
        return Ok(effective.listen_url());
    }
    runtime
        .gateway
        .start()
        .await
        .map_err(|err| error_hint::format_gateway_start_error(&err))?;
    sync_client_endpoints_to_listen_port(runtime).await?;
    let after_embed = config_with_listen_port(runtime).await;
    if gateway_daemon::wait_reachable(&after_embed.client_host(), after_embed.port, 8).await {
        sync_client_endpoints_to_listen_port(runtime).await?;
        return Ok(after_embed.listen_url());
    }

    if let Err(err) = gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
        return Err(err.to_string());
    }
    // detached 可能走了备选口：扫候选表，而不是死盯首选口。
    if let Some(port) = find_reachable_listen_port(&runtime.config.read().await.clone()).await {
        sync_client_endpoints_to_listen_port(runtime).await?;
        let mut cfg = runtime.config.read().await.clone();
        cfg.port = port;
        Ok(cfg.listen_url())
    } else {
        Err("启动网关失败：健康检查未通过，请查看日志或更换监听端口".to_string())
    }
}

#[tauri::command]
pub async fn start_gateway(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    ensure_gateway_running(&runtime).await
}

#[tauri::command]
pub async fn stop_gateway(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    stop_gateway_core(runtime.inner()).await
}

pub(crate) async fn stop_gateway_core(runtime: &AppRuntime) -> Result<(), String> {
    runtime.gateway.stop().await.map_err(|err| err.to_string())?;
    gateway_daemon::stop_detached(&runtime.paths.config_dir).map_err(|err| err.to_string())?;
    gateway_daemon::kill_sugt_cli_processes();
    Ok(())
}

pub(crate) async fn list_providers_core(
    runtime: &AppRuntime,
) -> Result<Vec<ProviderView>, String> {
    let config = runtime.config.read().await;
    Ok(config.providers.iter().map(ProviderView::from).collect())
}

#[tauri::command]
pub async fn list_providers(runtime: State<'_, AppRuntime>) -> Result<Vec<ProviderView>, String> {
    list_providers_core(runtime.inner()).await
}

pub(crate) async fn save_provider_core(
    runtime: &AppRuntime,
    input: ProviderInput,
) -> Result<Vec<ProviderView>, String> {
    validate_provider_input(&input)?;
    let (retry_on, retry_extra) = normalize_tool_text_retry(&input);
    let mut config = runtime.config.write().await;
    let id = input.id.clone().unwrap_or_default();

    // 实验通道只读：仅允许改 model_name / enabled
    if crate::experimental_free::is_experimental_free_id(&id) {
        let provider = config
            .providers
            .iter_mut()
            .find(|provider| provider.id == id)
            .ok_or_else(|| "实验免费通道不存在".to_string())?;
        if input.model_name.trim().is_empty() {
            return Err("请选择模型".to_string());
        }
        if crate::experimental_free::route_for_model(&input.model_name).is_none() {
            return Err("该模型不在免费实验通道候选列表中".to_string());
        }
        provider.model_name = input.model_name.trim().to_string();
        provider.enabled = input.enabled;
        crate::experimental_free::apply_route(provider);
        drop(config);
        runtime.persist().await.map_err(|err| err.to_string())?;
        return list_providers_core(runtime).await;
    }

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
        provider.tool_text_retry_enabled = retry_on;
        provider.tool_text_retry_extra = retry_extra;
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
        provider.tool_text_retry_enabled = retry_on;
        provider.tool_text_retry_extra = retry_extra;
        if config.active_provider_id.is_none() {
            config.active_provider_id = Some(provider.id.clone());
        }
        config.providers.push(provider);
    }

    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers_core(runtime).await
}

#[tauri::command]
pub async fn save_provider(
    runtime: State<'_, AppRuntime>,
    input: ProviderInput,
) -> Result<Vec<ProviderView>, String> {
    save_provider_core(runtime.inner(), input).await
}

pub(crate) async fn delete_provider_core(
    runtime: &AppRuntime,
    id: String,
) -> Result<Vec<ProviderView>, String> {
    let mut config = runtime.config.write().await;
    if crate::experimental_free::is_experimental_free_id(&id) {
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
    list_providers_core(runtime).await
}

#[tauri::command]
pub async fn delete_provider(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<Vec<ProviderView>, String> {
    delete_provider_core(runtime.inner(), id).await
}

pub(crate) async fn restore_experimental_free_core(
    runtime: &AppRuntime,
) -> Result<Vec<ProviderView>, String> {
    {
        let mut config = runtime.config.write().await;
        crate::experimental_free::restore_experimental_free(&mut config);
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers_core(runtime).await
}

#[tauri::command]
pub async fn restore_experimental_free(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<ProviderView>, String> {
    restore_experimental_free_core(runtime.inner()).await
}

pub(crate) fn list_experimental_free_candidates_core(
) -> Vec<crate::experimental_free::FreeRouteChoice> {
    crate::experimental_free::route_choices()
}

#[tauri::command]
pub fn list_experimental_free_candidates() -> Vec<crate::experimental_free::FreeRouteChoice> {
    list_experimental_free_candidates_core()
}

pub(crate) async fn probe_experimental_free_models_core(
    runtime: &AppRuntime,
) -> Result<Vec<crate::experimental_free::FreeModelProbe>, String> {
    let probes = crate::experimental_free::test_free_models(runtime.gateway.http_client()).await;
    {
        let mut config = runtime.config.write().await;
        let auto_rotate = config.experimental_free_auto_rotate;
        if let Some(provider) = config
            .providers
            .iter_mut()
            .find(|p| crate::experimental_free::is_experimental_free(p))
        {
            let available: Vec<&str> = probes
                .iter()
                .filter(|p| p.available)
                .map(|p| p.name.as_str())
                .collect();
            if available.is_empty() {
                provider.last_error =
                    Some("免费实验通道暂无可用模型，请稍后重试".into());
                provider.status = ProviderStatus::Unavailable;
            } else {
                provider.last_error = None;
                provider.status = ProviderStatus::Available;
                // 默认不擅自改首选；开启 auto_rotate 时按 t/s（其次延迟）切最优路由。
                if auto_rotate {
                    let _ = crate::experimental_free::apply_best_from_probes(provider, &probes);
                }
            }
            provider.last_checked_at = Some(chrono::Utc::now());
        }
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(probes)
}

#[tauri::command]
pub async fn probe_experimental_free_models(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<crate::experimental_free::FreeModelProbe>, String> {
    probe_experimental_free_models_core(runtime.inner()).await
}

pub(crate) async fn set_active_provider_core(
    runtime: &AppRuntime,
    id: String,
) -> Result<(), String> {
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
pub async fn set_active_provider(runtime: State<'_, AppRuntime>, id: String) -> Result<(), String> {
    set_active_provider_core(runtime.inner(), id).await
}

pub(crate) async fn set_provider_enabled_core(
    runtime: &AppRuntime,
    id: String,
    enabled: bool,
) -> Result<Vec<ProviderView>, String> {
    let is_free = crate::experimental_free::is_experimental_free_id(&id);
    if enabled && is_free {
        let accepted = runtime.config.read().await.experimental_free_disclaimer_accepted;
        if !accepted {
            return Err(
                "free_disclaimer_required:请先在模型配置 → 实验·免费通道「设置」中确认免责声明"
                    .to_string(),
            );
        }
    }
    {
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
            config.active_provider_id = Some(id.clone());
        }
    }

    // 仅实验免费通道：启用时并发探测（不碰用户自定义源）
    if enabled && is_free {
        let probes =
            crate::experimental_free::probe_free_models(runtime.gateway.http_client()).await;
        let mut config = runtime.config.write().await;
        if let Some(provider) = config
            .providers
            .iter_mut()
            .find(|p| crate::experimental_free::is_experimental_free_id(&p.id))
        {
            let available: Vec<&str> = probes
                .iter()
                .filter(|p| p.available)
                .map(|p| p.name.as_str())
                .collect();
            if available.is_empty() {
                provider.last_error =
                    Some("免费实验通道暂无可用模型，请稍后重试".into());
                provider.status = ProviderStatus::Unavailable;
            } else {
                if !available
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(provider.model_name.trim()))
                {
                    if let Some(preferred) =
                        crate::experimental_free::pick_preferred_model(&available)
                    {
                        provider.model_name = preferred.to_string();
                    }
                }
                crate::experimental_free::apply_route(provider);
                provider.last_error = None;
                provider.status = ProviderStatus::Available;
            }
            provider.last_checked_at = Some(chrono::Utc::now());
        }
    }

    runtime.persist().await.map_err(|err| err.to_string())?;
    list_providers_core(runtime).await
}

#[tauri::command]
pub async fn set_provider_enabled(
    runtime: State<'_, AppRuntime>,
    id: String,
    enabled: bool,
) -> Result<Vec<ProviderView>, String> {
    set_provider_enabled_core(runtime.inner(), id, enabled).await
}

pub(crate) async fn test_provider_core(
    runtime: &AppRuntime,
    id: String,
) -> Result<gateway::ProviderTestOutcome, String> {
    let outcome = runtime
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
        provider.status = outcome.status.clone();
        provider.last_checked_at = Some(chrono::Utc::now());
    }
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(outcome)
}

#[tauri::command]
pub async fn test_provider(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<gateway::ProviderTestOutcome, String> {
    test_provider_core(runtime.inner(), id).await
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

pub(crate) async fn test_provider_draft_core(
    runtime: &AppRuntime,
    input: ListModelsInput,
    model_name: String,
) -> Result<probe::ProbeReport, String> {
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
    // 失败也返回报告（ok=false + error），前端统一按 ProbeReport 渲染延迟与原因
    Ok(probe::probe_llm_connectivity(runtime.gateway.http_client(), &provider).await)
}

#[tauri::command]
pub async fn test_provider_draft(
    runtime: State<'_, AppRuntime>,
    input: ListModelsInput,
    model_name: String,
) -> Result<probe::ProbeReport, String> {
    test_provider_draft_core(runtime.inner(), input, model_name).await
}

pub(crate) async fn list_provider_models_core(
    runtime: &AppRuntime,
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
pub async fn list_provider_models(
    runtime: State<'_, AppRuntime>,
    input: ListModelsInput,
) -> Result<Vec<String>, String> {
    list_provider_models_core(runtime.inner(), input).await
}

#[derive(Deserialize)]
pub struct ListModelRegistryInput {
    pub provider_id: String,
}

#[derive(Deserialize)]
pub struct SetModelEnabledInput {
    pub provider_id: String,
    pub model_name: String,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct ProbeModelsInput {
    pub provider_id: String,
    pub model_names: Vec<String>,
}

fn find_provider(config: &AppConfig, provider_id: &str) -> Result<ProviderConfig, String> {
    config
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
        .ok_or_else(|| format!("未找到服务商: {provider_id}"))
}

/// 拉取上游模型列表并与注册表合并；普通服务商走网关现成的 models 接口，
/// 实验免费通道直接用内置候选，不发网络请求。
pub(crate) async fn list_model_registry_core(
    runtime: &AppRuntime,
    input: ListModelRegistryInput,
) -> Result<Vec<ModelEntry>, String> {
    let config = runtime.config.read().await.clone();
    let provider = find_provider(&config, &input.provider_id)?;
    let upstream_models: Vec<String> = if crate::experimental_free::is_experimental_free(&provider) {
        crate::experimental_free::candidate_models()
            .into_iter()
            .map(|model| model.to_string())
            .collect()
    } else {
        let base_url = crate::model::resolve_provider_base_url(
            provider.base_url.clone(),
            &provider.protocol,
            provider.auto_adapt_base_url,
        );
        runtime
            .gateway
            .list_provider_models(base_url.trim(), provider.api_key.trim(), None)
            .await
            .map_err(|err| err.to_string())?
    };

    let config_dir = runtime.paths.config_dir.clone();
    let existing = model_registry::load(&config_dir);
    let merged = model_registry::merge_entries(&existing, &provider.id, &upstream_models);
    model_registry::save(&config_dir, &merged).map_err(|err| err.to_string())?;
    Ok(model_registry::entries_for_provider(
        &merged,
        &provider.id,
        &upstream_models,
    ))
}

#[tauri::command]
pub async fn list_model_registry(
    runtime: State<'_, AppRuntime>,
    input: ListModelRegistryInput,
) -> Result<Vec<ModelEntry>, String> {
    list_model_registry_core(runtime.inner(), input).await
}

/// 启用/禁用单个模型（禁用后控制台不再展示；模型列表 tab 仍保留）。
pub(crate) async fn set_model_enabled_core(
    runtime: &AppRuntime,
    input: SetModelEnabledInput,
) -> Result<(), String> {
    let config_dir = runtime.paths.config_dir.clone();
    let mut registry = model_registry::load(&config_dir);
    model_registry::set_enabled(
        &mut registry,
        &input.provider_id,
        &input.model_name,
        input.enabled,
    );
    model_registry::save(&config_dir, &registry).map_err(|err| err.to_string())?;
    // 禁用状态立即对策略盒子选路生效：同步网关内存副本。
    runtime
        .gateway
        .set_model_registry_snapshot(registry.entries.clone())
        .await;
    Ok(())
}

#[tauri::command]
pub async fn set_model_enabled(
    runtime: State<'_, AppRuntime>,
    input: SetModelEnabledInput,
) -> Result<(), String> {
    set_model_enabled_core(runtime.inner(), input).await
}

/// 批量探测指定模型（限并发 3），结果一次性回写注册表并返回最新条目。
pub(crate) async fn probe_models_core(
    runtime: &AppRuntime,
    input: ProbeModelsInput,
) -> Result<Vec<ModelEntry>, String> {
    if input.model_names.is_empty() {
        return Ok(vec![]);
    }
    let config = runtime.config.read().await.clone();
    let provider = find_provider(&config, &input.provider_id)?;

    // 统一走 probe_llm_connectivity：免费通道在 probe_openai 内部经
    // prepare_upstream_provider 绑定路由，同样得到真实延迟/速度；
    // 旧的免费专用分支只回 available 且硬编码 0 值，已废弃。
    let client = runtime.gateway.http_client().clone();
    let provider = provider.clone();
    let results: Vec<(String, ModelProbe)> =
        futures_util::stream::iter(input.model_names.iter().cloned().map(|model_name| {
            let client = client.clone();
            let mut target = provider.clone();
            target.model_name = model_name.clone();
            async move {
                let report = probe::probe_llm_connectivity(&client, &target).await;
                (
                    model_name,
                    ModelProbe {
                        ok: report.ok,
                        latency_ms: report.latency_ms,
                        tokens_per_sec: report.tokens_per_sec,
                        completion_tokens: report.completion_tokens,
                        estimated: report.estimated,
                        probed_at: chrono::Utc::now(),
                        error: report.error,
                    },
                )
            }
        }))
        .buffer_unordered(3)
        .collect::<Vec<_>>()
        .await;

    let config_dir = runtime.paths.config_dir.clone();
    let mut registry = model_registry::load(&config_dir);
    for (model_name, probe) in results {
        model_registry::apply_probe(&mut registry, &provider.id, &model_name, probe);
    }
    model_registry::save(&config_dir, &registry).map_err(|err| err.to_string())?;
    runtime
        .gateway
        .set_model_registry_snapshot(registry.entries.clone())
        .await;
    Ok(model_registry::entries_for_provider(
        &registry,
        &provider.id,
        &input.model_names,
    ))
}

#[tauri::command]
pub async fn probe_models(
    runtime: State<'_, AppRuntime>,
    input: ProbeModelsInput,
) -> Result<Vec<ModelEntry>, String> {
    probe_models_core(runtime.inner(), input).await
}

/// 自定义通道逐模型真实测试结果，前端按 camelCase 读取（latencyMs / tokensPerSec）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTestResult {
    pub model_name: String,
    pub available: bool,
    pub latency_ms: Option<u64>,
    pub tokens_per_sec: Option<f64>,
}

/// 逐模型真实 LLM 测试（自定义通道）：取模型注册表里启用的模型，
/// 逐个 clone provider 覆盖 model_name 后走 probe_llm_connectivity，并发上限 4。
pub(crate) async fn test_provider_models_core(
    runtime: &AppRuntime,
    id: String,
) -> Result<Vec<ModelTestResult>, String> {
    let config = runtime.config.read().await.clone();
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .cloned()
        .ok_or_else(|| "模型配置不存在".to_string())?;
    if crate::experimental_free::is_experimental_free_id(&id) {
        return Err("实验免费通道请使用 probe_experimental_free_models".to_string());
    }
    // 与 list_model_registry 同一数据源：model-registry.json，只测启用项。
    let registry = model_registry::load(&runtime.paths.config_dir);
    let model_names: Vec<String> = registry
        .entries
        .iter()
        .filter(|entry| entry.provider_id == provider.id && entry.enabled)
        .map(|entry| entry.model_name.clone())
        .collect();
    let client = runtime.gateway.http_client().clone();
    let results = futures_util::stream::iter(model_names.into_iter().map(|model_name| {
        let client = client.clone();
        let mut target = provider.clone();
        target.model_name = model_name.clone();
        async move {
            let report = probe::probe_llm_connectivity(&client, &target).await;
            ModelTestResult {
                model_name,
                available: report.ok,
                latency_ms: Some(report.latency_ms),
                tokens_per_sec: report.ok.then_some(report.tokens_per_sec),
            }
        }
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;
    Ok(results)
}

#[tauri::command]
pub async fn test_provider_models(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<Vec<ModelTestResult>, String> {
    test_provider_models_core(runtime.inner(), id).await
}

#[derive(Debug, Deserialize)]
pub struct SaveStrategyBoxInput {
    pub spec: StrategyBox,
}

#[derive(Debug, Deserialize)]
pub struct StrategyBoxIdInput {
    pub box_id: String,
}

/// 一个成员的运行态健康度，前端只读展示。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyMemberStatus {
    pub provider_id: String,
    pub model_name: String,
    pub last_latency_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

/// 策略盒子列表（磁盘为准；启动时网关也已加载同一份）。
#[tauri::command]
pub async fn list_strategy_boxes(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<StrategyBox>, String> {
    Ok(strategy_box::load_boxes(&runtime.paths.config_dir))
}

/// 保存（新建或整体替换）一个策略盒子：读盘 → 校验 → upsert → 落盘 → 刷新网关内存副本。
#[tauri::command]
pub async fn save_strategy_box(
    runtime: State<'_, AppRuntime>,
    input: SaveStrategyBoxInput,
) -> Result<StrategyBox, String> {
    let providers = runtime.config.read().await.providers.clone();
    let config_dir = runtime.paths.config_dir.clone();
    let mut boxes = strategy_box::load_boxes(&config_dir);
    strategy_box::validate(&input.spec, &providers)?;
    strategy_box::check_public_id_conflict(&boxes, &input.spec)?;
    let saved = strategy_box::upsert(&mut boxes, input.spec);
    strategy_box::save_boxes(&config_dir, &boxes).map_err(|err| err.to_string())?;
    runtime.gateway.set_strategy_boxes(boxes).await;
    Ok(saved)
}

/// 删除一个策略盒子（写盘后同步刷新网关内存副本）。
#[tauri::command]
pub async fn delete_strategy_box(
    runtime: State<'_, AppRuntime>,
    input: StrategyBoxIdInput,
) -> Result<(), String> {
    let config_dir = runtime.paths.config_dir.clone();
    let mut boxes = strategy_box::load_boxes(&config_dir);
    let before = boxes.len();
    boxes.retain(|item| item.id != input.box_id);
    if boxes.len() == before {
        return Err(format!("未找到策略盒子: {}", input.box_id));
    }
    strategy_box::save_boxes(&config_dir, &boxes).map_err(|err| err.to_string())?;
    runtime.gateway.set_strategy_boxes(boxes).await;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct SaveRequestRuleInput {
    pub spec: RequestRule,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequestRuleInput {
    pub rule_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveApiKeyPoolInput {
    pub spec: crate::api_key_pool::ApiKeyPool,
}

#[derive(Debug, Deserialize)]
pub struct DeleteApiKeyPoolInput {
    pub pool_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetKeyUsageInput {
    pub pool_id: String,
    pub key_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClientKeyInput {
    pub name: String,
    pub key: Option<String>,
    /// 可选创建第二把主密钥；默认 `false`，且不允许出现第二个 primary。
    #[serde(default)]
    pub primary: Option<bool>,
    #[serde(default)]
    pub quota: Option<ClientQuotaInput>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClientKeyInput {
    pub id: String,
    pub name: Option<String>,
    pub key: Option<String>,
    pub enabled: Option<bool>,
    /// 传 `Some` 才覆盖配额；各限值用 `Option<Option<u64>>` 区分「不动」与「清除」。
    #[serde(default)]
    pub quota: Option<ClientQuotaInput>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteClientKeyInput {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetClientKeyUsageInput {
    pub id: String,
}

/// 配额输入：字段缺失 = 保持原值，显式 `null` = 清除该限值。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientQuotaInput {
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    pub request_limit: Option<Option<u64>>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    pub token_limit: Option<Option<u64>>,
}

fn deserialize_optional_limit<'de, D>(deserializer: D) -> Result<Option<Option<u64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<u64>::deserialize(deserializer)?))
}

fn quota_from_input(input: Option<ClientQuotaInput>) -> crate::client_keys::ClientQuota {
    let input = input.unwrap_or_default();
    crate::client_keys::ClientQuota {
        request_limit: input.request_limit.flatten(),
        token_limit: input.token_limit.flatten(),
    }
}

/// 请求改写规则列表（磁盘为准；文件缺失时返回内置默认规则）。
#[tauri::command]
pub async fn list_request_rules(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<RequestRule>, String> {
    Ok(request_rules::load_rules(&runtime.paths.config_dir))
}

/// 保存（新建或整体替换）一条规则：读盘 → 校验 → upsert → 落盘。
#[tauri::command]
pub async fn save_request_rule(
    runtime: State<'_, AppRuntime>,
    input: SaveRequestRuleInput,
) -> Result<RequestRule, String> {
    let config_dir = runtime.paths.config_dir.clone();
    let mut rules = request_rules::load_rules(&config_dir);
    request_rules::validate(&input.spec, &rules)?;
    let saved = request_rules::upsert(&mut rules, input.spec);
    request_rules::save_rules(&config_dir, &rules).map_err(|err| err.to_string())?;
    // 网关内存副本同步：新规则立即对入站请求生效。
    runtime.gateway.set_request_rules(rules).await;
    Ok(saved)
}

/// 删除一条规则；内置规则改为禁用而不是移除，保留「显式关闭」的语义。
#[tauri::command]
pub async fn delete_request_rule(
    runtime: State<'_, AppRuntime>,
    input: DeleteRequestRuleInput,
) -> Result<(), String> {
    let config_dir = runtime.paths.config_dir.clone();
    let mut rules = request_rules::load_rules(&config_dir);
    if request_rules::is_builtin_rule_id(&input.rule_id) {
        let rule = rules
            .iter_mut()
            .find(|rule| rule.id == input.rule_id)
            .ok_or_else(|| format!("未找到请求改写规则: {}", input.rule_id))?;
        rule.enabled = false;
    } else {
        let before = rules.len();
        rules.retain(|rule| rule.id != input.rule_id);
        if rules.len() == before {
            return Err(format!("未找到请求改写规则: {}", input.rule_id));
        }
    }
    request_rules::save_rules(&config_dir, &rules).map_err(|err| err.to_string())?;
    // 网关内存副本同步：删除/禁用立即生效。
    runtime.gateway.set_request_rules(rules).await;
    Ok(())
}

/// API Key 池列表（内存为准，含最新累计用量）。
#[tauri::command]
pub async fn list_api_key_pools(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<crate::api_key_pool::ApiKeyPoolView>, String> {
    let snapshot = runtime.gateway.key_pools_snapshot().await;
    Ok(snapshot
        .pools
        .iter()
        .map(crate::api_key_pool::ApiKeyPoolView::from)
        .collect())
}

/// 保存（新建或整体替换）一个池：内存快照 → 校验 → upsert → 落盘 → 换内存。
#[tauri::command]
pub async fn save_api_key_pool(
    runtime: State<'_, AppRuntime>,
    input: SaveApiKeyPoolInput,
) -> Result<crate::api_key_pool::ApiKeyPool, String> {
    let mut file = runtime.gateway.key_pools_snapshot().await;
    crate::api_key_pool::validate(&input.spec, &file.pools)?;
    let saved = crate::api_key_pool::upsert(&mut file.pools, input.spec);
    runtime.gateway.persist_key_pools(file).await?;
    Ok(saved)
}

/// 删除一个池。
#[tauri::command]
pub async fn delete_api_key_pool(
    runtime: State<'_, AppRuntime>,
    input: DeleteApiKeyPoolInput,
) -> Result<(), String> {
    let mut file = runtime.gateway.key_pools_snapshot().await;
    if !crate::api_key_pool::remove_pool(&mut file.pools, &input.pool_id) {
        return Err(format!("未找到 API Key 池: {}", input.pool_id));
    }
    runtime.gateway.persist_key_pools(file).await
}

/// 清零用量：`key_id` 为空时清整池。
#[tauri::command]
pub async fn reset_key_usage(
    runtime: State<'_, AppRuntime>,
    input: ResetKeyUsageInput,
) -> Result<(), String> {
    let mut file = runtime.gateway.key_pools_snapshot().await;
    let pool = file
        .pools
        .iter_mut()
        .find(|pool| pool.id == input.pool_id)
        .ok_or_else(|| format!("未找到 API Key 池: {}", input.pool_id))?;
    crate::api_key_pool::reset_usage(pool, input.key_id.as_deref());
    runtime.gateway.persist_key_pools(file).await
}

/// 入站密钥列表（内存为准，含最新累计用量）。
#[tauri::command]
pub async fn list_client_keys(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<crate::client_keys::ClientKeyView>, String> {
    list_client_keys_core(runtime.inner()).await
}

pub(crate) async fn list_client_keys_core(
    runtime: &AppRuntime,
) -> Result<Vec<crate::client_keys::ClientKeyView>, String> {
    let file = runtime.gateway.client_keys_snapshot().await;
    Ok(file
        .keys
        .iter()
        .map(crate::client_keys::ClientKeyView::from)
        .collect())
}

/// 新建一把入站密钥：`key` 为空时自动生成。
#[tauri::command]
pub async fn create_client_key(
    runtime: State<'_, AppRuntime>,
    input: CreateClientKeyInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    create_client_key_core(runtime.inner(), input).await
}

pub(crate) async fn create_client_key_core(
    runtime: &AppRuntime,
    input: CreateClientKeyInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    crate::client_keys::validate_name(&input.name)?;
    let name = input.name.trim().to_string();
    let mut file = runtime.gateway.client_keys_snapshot().await;
    if file
        .keys
        .iter()
        .any(|key| key.name.trim() == name.as_str())
    {
        return Err(format!("密钥名称已存在: {name}"));
    }
    let primary = input.primary.unwrap_or(false);
    if primary && file.keys.iter().any(|key| key.primary) {
        return Err("已存在主密钥".to_string());
    }
    let key = match input
        .key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        Some(key) => key,
        None => crate::client_keys::generate_key(),
    };
    // 与既有 key（含 config 里的 gateway_client_api_key）重复会造成归属歧义。
    // 主密钥本身就要占用 config 里的 key，因此只在非主密钥时检查配置项。
    if let Some(existing) = duplicate_key_source(runtime, &file.keys, &key, !primary).await {
        return Err(format!("密钥已存在: {existing}"));
    }
    let created = crate::client_keys::upsert(
        &mut file.keys,
        crate::client_keys::ClientKey {
            id: String::new(),
            name,
            key,
            enabled: true,
            primary,
            quota: quota_from_input(input.quota),
            usage: crate::client_keys::ClientKeyUsage::default(),
        },
    );
    runtime.gateway.persist_client_keys(file).await?;
    Ok(crate::client_keys::ClientKeyView::from(&created))
}

/// 修改一把入站密钥：未传的字段保持原值，`usage` 一直保留。
#[tauri::command]
pub async fn update_client_key(
    runtime: State<'_, AppRuntime>,
    input: UpdateClientKeyInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    update_client_key_core(runtime.inner(), input).await
}

pub(crate) async fn update_client_key_core(
    runtime: &AppRuntime,
    input: UpdateClientKeyInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    let mut file = runtime.gateway.client_keys_snapshot().await;
    let position = file
        .keys
        .iter()
        .position(|key| key.id == input.id)
        .ok_or_else(|| format!("未找到入站密钥: {}", input.id))?;
    let is_primary = file.keys[position].primary;
    crate::client_keys::validate_update(&file.keys[position], input.enabled)?;

    if let Some(name) = input.name.as_deref() {
        crate::client_keys::validate_name(name)?;
        let name = name.trim().to_string();
        if file
            .keys
            .iter()
            .enumerate()
            .any(|(index, key)| index != position && key.name.trim() == name.as_str())
        {
            return Err(format!("密钥名称已存在: {name}"));
        }
        file.keys[position].name = name;
    }

    let mut key_changed = false;
    if let Some(key) = input.key.as_deref() {
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err("密钥不能为空".to_string());
        }
        let others: Vec<crate::client_keys::ClientKey> = file
            .keys
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != position)
            .map(|(_, key)| key.clone())
            .collect();
        if let Some(existing) =
            duplicate_key_source(runtime, &others, &key, !is_primary).await
        {
            return Err(format!("密钥已存在: {existing}"));
        }
        key_changed = file.keys[position].key != key;
        file.keys[position].key = key;
    }

    if let Some(quota) = input.quota {
        let target = &mut file.keys[position].quota;
        if let Some(limit) = quota.request_limit {
            target.request_limit = limit;
        }
        if let Some(limit) = quota.token_limit {
            target.token_limit = limit;
        }
    }

    if let Some(enabled) = input.enabled {
        file.keys[position].enabled = enabled;
    }

    let updated = file.keys[position].clone();
    runtime.gateway.persist_client_keys(file).await?;

    // 主密钥的 key 就是网关入站 key：改完立即同步配置，接入信息卡与 Codex 环境变量同步更新。
    if is_primary && key_changed {
        {
            let mut config = runtime.config.write().await;
            config.gateway_client_api_key = updated.key.clone();
        }
        runtime.persist().await.map_err(|err| err.to_string())?;
    }

    Ok(crate::client_keys::ClientKeyView::from(&updated))
}

/// 删除一把入站密钥；主密钥不可删除。
#[tauri::command]
pub async fn delete_client_key(
    runtime: State<'_, AppRuntime>,
    input: DeleteClientKeyInput,
) -> Result<(), String> {
    delete_client_key_core(runtime.inner(), input).await
}

pub(crate) async fn delete_client_key_core(
    runtime: &AppRuntime,
    input: DeleteClientKeyInput,
) -> Result<(), String> {
    let mut file = runtime.gateway.client_keys_snapshot().await;
    let existing = file
        .keys
        .iter()
        .find(|key| key.id == input.id)
        .ok_or_else(|| format!("未找到入站密钥: {}", input.id))?;
    crate::client_keys::validate_delete(existing)?;
    crate::client_keys::remove_key(&mut file.keys, &input.id);
    runtime.gateway.persist_client_keys(file).await
}

/// 清空一把入站密钥的用量，保留 key 与配额。
#[tauri::command]
pub async fn reset_client_key_usage(
    runtime: State<'_, AppRuntime>,
    input: ResetClientKeyUsageInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    reset_client_key_usage_core(runtime.inner(), input).await
}

pub(crate) async fn reset_client_key_usage_core(
    runtime: &AppRuntime,
    input: ResetClientKeyUsageInput,
) -> Result<crate::client_keys::ClientKeyView, String> {
    let mut file = runtime.gateway.client_keys_snapshot().await;
    let entry = file
        .keys
        .iter_mut()
        .find(|key| key.id == input.id)
        .ok_or_else(|| format!("未找到入站密钥: {}", input.id))?;
    crate::client_keys::reset_usage(entry);
    let view = crate::client_keys::ClientKeyView::from(&*entry);
    runtime.gateway.persist_client_keys(file).await?;
    Ok(view)
}

/// 检查 key 是否已被占用（池内其它 key 或网关配置里的 key）。
/// `check_configured=false` 时只看池内其它 key（主密钥改 key 时用）。
/// 返回脱敏后的重复值，便于直接拼进错误信息。
async fn duplicate_key_source(
    runtime: &AppRuntime,
    others: &[crate::client_keys::ClientKey],
    key: &str,
    check_configured: bool,
) -> Option<String> {
    if check_configured {
        let configured = runtime
            .gateway
            .config()
            .await
            .gateway_client_api_key
            .trim()
            .to_string();
        if !configured.is_empty() && configured == key {
            return Some(crate::model::mask_secret(key));
        }
    }
    others
        .iter()
        .find(|other| other.key.trim() == key)
        .map(|_| crate::model::mask_secret(key))
}

/// 读取成员健康度（只读，不发探测请求）。
#[tauri::command]
pub async fn get_strategy_box_status(
    runtime: State<'_, AppRuntime>,
    input: StrategyBoxIdInput,
) -> Result<Vec<StrategyMemberStatus>, String> {
    let state = runtime.gateway.strategy_state();
    let guard = state.read().await;
    let box_spec = guard
        .boxes
        .iter()
        .find(|item| item.id == input.box_id)
        .ok_or_else(|| format!("未找到策略盒子: {}", input.box_id))?;
    let now = Instant::now();
    let statuses = box_spec
        .members
        .iter()
        .map(|member| {
            let health = guard
                .runtime
                .status(&strategy_box::member_key(&box_spec.id, member));
            // Instant 不能序列化，换算成「现在 + 剩余时长」的 UTC 时间点。
            let cooldown_until = health.cooldown_until.and_then(|deadline| {
                deadline
                    .checked_duration_since(now)
                    .and_then(|left| chrono::Duration::from_std(left).ok())
                    .map(|left| chrono::Utc::now() + left)
            });
            StrategyMemberStatus {
                provider_id: member.provider_id.clone(),
                model_name: member.model_name.clone(),
                last_latency_ms: health.last_latency_ms,
                consecutive_failures: health.consecutive_failures,
                cooldown_until,
                last_error: health.last_error,
            }
        })
        .collect();
    Ok(statuses)
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
    /// 备选监听口列表；空数组表示恢复默认 `9878/18887/28887`。
    pub port_fallback_ports: Option<Vec<u16>>,
    pub gateway_client_api_key: Option<String>,
    #[serde(rename = "anthropicAccessMode")]
    pub anthropic_access_mode: Option<AnthropicAccessMode>,
    /// 免费通道：获取模型列表后逐路由真实测速
    pub free_models_fetch_and_test: Option<bool>,
    /// 自定义通道：获取模型后逐模型真实测试
    pub custom_models_fetch_and_test: Option<bool>,
    /// 单次请求内上游重试总预算（毫秒）；`null` 表示恢复默认 8 分钟。
    pub upstream_retry_budget_ms: Option<Option<u64>>,
    /// 单次等待上限（毫秒）；`null` 表示恢复默认 60 秒。
    pub upstream_retry_single_ms: Option<Option<u64>>,
    /// 实验·免费通道免责声明已确认
    pub experimental_free_disclaimer_accepted: Option<bool>,
    /// 闲时自动测速
    pub experimental_free_idle_probe_enabled: Option<bool>,
    /// 闲时测速间隔（秒）
    pub experimental_free_idle_probe_interval_secs: Option<u64>,
    /// 测速后按 t/s 自动切最优路由
    pub experimental_free_auto_rotate: Option<bool>,
}

#[tauri::command]
pub async fn update_gateway_settings(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    input: GatewaySettingsInput,
) -> Result<AppConfig, String> {
    update_gateway_settings_core(runtime.inner(), input, Some(&app)).await
}

pub async fn update_gateway_settings_core(
    runtime: &AppRuntime,
    input: GatewaySettingsInput,
    app: Option<&AppHandle>,
) -> Result<AppConfig, String> {
    let mut lan_changed = false;
    let mut port_changed = false;
    let mut config = runtime.config.write().await;
    if let Some(enabled) = input.port_fallback_enabled {
        config.port_fallback_enabled = enabled;
    }
    if let Some(enabled) = input.gateway_watchdog_enabled {
        config.gateway_watchdog_enabled = enabled;
    }
    if let Some(enabled) = input.allow_lan_access {
        if config.allow_lan_access != enabled {
            lan_changed = true;
        }
        config.allow_lan_access = enabled;
    }
    if let Some(port) = input.port {
        if port == 0 {
            return Err("端口不能为 0".to_string());
        }
        if config.port != port {
            port_changed = true;
        }
        config.port = port;
    }
    if let Some(ports) = input.port_fallback_ports {
        let cleaned: Vec<u16> = ports
            .into_iter()
            .filter(|p| *p != 0)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        config.port_fallback_ports = if cleaned.is_empty() {
            DEFAULT_PORT_FALLBACK_PORTS.to_vec()
        } else {
            cleaned
        };
    }
    if let Some(key) = input.gateway_client_api_key {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err("网关 API Key 不能为空".to_string());
        }
        let new_key = trimmed.to_string();
        // WB-003：程序化改主 key 必须与 client-keys 池同步，否则鉴权仍用旧 key、
        // 接入卡却显示新 key。复用 update_client_key 的 primary 分支：改池内 primary
        // + 一次持久化；ensure_primary 保证空池/无 primary 时补种，已有 primary 时幂等。
        let mut keys_file = runtime.gateway.client_keys_snapshot().await;
        let mut keys_changed = crate::client_keys::ensure_primary(&mut keys_file, &new_key);
        if let Some(primary) = keys_file.keys.iter_mut().find(|key| key.primary) {
            if primary.key != new_key {
                primary.key = new_key.clone();
                keys_changed = true;
            }
        }
        if keys_changed {
            runtime.gateway.persist_client_keys(keys_file).await?;
        }
        config.gateway_client_api_key = new_key;
    }
    if let Some(mode) = input.anthropic_access_mode {
        config.anthropic_access_mode = mode;
    }
    if let Some(enabled) = input.free_models_fetch_and_test {
        config.free_models_fetch_and_test = enabled;
    }
    if let Some(enabled) = input.custom_models_fetch_and_test {
        config.custom_models_fetch_and_test = enabled;
    }
    if let Some(budget) = input.upstream_retry_budget_ms {
        config.upstream_retry_budget_ms = budget.filter(|v| *v > 0);
    }
    if let Some(single) = input.upstream_retry_single_ms {
        config.upstream_retry_single_ms = single.filter(|v| *v > 0);
    }
    if let Some(accepted) = input.experimental_free_disclaimer_accepted {
        config.experimental_free_disclaimer_accepted = accepted;
    }
    if let Some(enabled) = input.experimental_free_idle_probe_enabled {
        config.experimental_free_idle_probe_enabled = enabled;
    }
    if let Some(secs) = input.experimental_free_idle_probe_interval_secs {
        config.experimental_free_idle_probe_interval_secs = secs
            .max(crate::model::MIN_EXPERIMENTAL_FREE_IDLE_PROBE_INTERVAL_SECS);
    }
    if let Some(enabled) = input.experimental_free_auto_rotate {
        config.experimental_free_auto_rotate = enabled;
    }
    let snapshot = config.clone();
    drop(config);
    runtime.persist().await.map_err(|err| err.to_string())?;
    let need_rebind = lan_changed || port_changed;
    // 绑定地址 / 首选口变化后需重启监听，否则仍停在旧 socket
    if need_rebind && runtime.gateway.is_running().await {
        let _ = runtime.gateway.stop().await;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        runtime
            .gateway
            .replace_config(snapshot.clone())
            .await;
        if let Err(err) = runtime.gateway.start().await {
            warn!(error = %err, "restart gateway after bind settings change failed");
            return Err(error_hint::format_gateway_start_error(&err));
        }
        let _ = sync_client_endpoints_to_listen_port(runtime).await;
    } else if need_rebind {
        runtime.gateway.replace_config(snapshot.clone()).await;
    }
    // WebUI 的绑定地址同样跟随 allow_lan_access，必须一起重启，
    // 否则开关已开但管理面仍只监听回环（设置页会显示不可达地址）。
    if lan_changed {
        if let Some(app) = app {
            restart_webui_after_lan_change(app, runtime, &snapshot).await;
        }
    }
    Ok(snapshot)
}

async fn restart_webui_after_lan_change(
    app: &AppHandle,
    runtime: &AppRuntime,
    config: &AppConfig,
) {
    let Some(slot) = app.try_state::<WebUiHandle>().map(|state| state.inner().clone()) else {
        return;
    };
    if let Some(server) = slot.lock().await.take() {
        server.stop().await;
    }
    if !config.webui_enabled {
        return;
    }
    match crate::webui::start_on_port(runtime, config.webui_port).await {
        Ok(server) => {
            tracing::info!(addr = %server.addr, "WebUI 已按新的局域网设置重启");
            *slot.lock().await = Some(server);
        }
        Err(err) => warn!(error = %err, "局域网开关切换后重启 WebUI 失败"),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebUiSettingsInput {
    pub enabled: bool,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebUiSettingsView {
    pub enabled: bool,
    pub port: u16,
    /// 同网段可访问的 http://IP:port 地址列表。
    pub lan_addresses: Vec<String>,
}

#[tauri::command]
pub async fn get_webui_settings(
    runtime: State<'_, AppRuntime>,
) -> Result<WebUiSettingsView, String> {
    let config = runtime.config.read().await;
    Ok(webui_settings_view(
        config.webui_enabled,
        config.webui_port,
        config.allow_lan_access,
    ))
}

/// GUI 设置页开关持久化入口：写 config 并即时 start/stop，不改 CLI `--webui` 的本次运行语义。
#[tauri::command]
pub async fn set_webui_settings(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    input: WebUiSettingsInput,
) -> Result<WebUiSettingsView, String> {
    let (gateway_port, old_enabled, old_port) = {
        let config = runtime.config.read().await;
        (config.port, config.webui_enabled, config.webui_port)
    };
    if input.port == 0 {
        return Err("WebUI 端口不能为 0".to_string());
    }
    if input.port == gateway_port {
        return Err(format!("WebUI 端口 {} 与网关端口冲突，请更换", input.port));
    }

    // 先停旧实例释放端口；启动失败则尽量恢复旧实例，且配置保持不变。
    let slot = app
        .try_state::<WebUiHandle>()
        .map(|state| state.inner().clone());
    if let Some(slot) = &slot {
        if let Some(server) = slot.lock().await.take() {
            server.stop().await;
        }
    }

    let new_server = if input.enabled {
        match crate::webui::start_on_port(runtime.inner(), input.port).await {
            Ok(server) => {
                tracing::info!(addr = %server.addr, "WebUI 已即时启用");
                Some(server)
            }
            Err(err) => {
                if old_enabled && old_port != 0 && old_port != input.port {
                    match crate::webui::start_on_port(runtime.inner(), old_port).await {
                        Ok(server) => {
                            if let Some(slot) = &slot {
                                *slot.lock().await = Some(server);
                            }
                        }
                        Err(restore_err) => {
                            warn!(error = %restore_err, "恢复旧 WebUI 实例失败");
                        }
                    }
                }
                return Err(format!("WebUI 启动失败：{err}"));
            }
        }
    } else {
        None
    };

    {
        let mut config = runtime.config.write().await;
        config.webui_enabled = input.enabled;
        config.webui_port = input.port;
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    let allow_lan = runtime.config.read().await.allow_lan_access;

    if let Some(slot) = slot {
        *slot.lock().await = new_server;
    }
    Ok(webui_settings_view(input.enabled, input.port, allow_lan))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LanShareInfo {
    pub enabled: bool,
    pub port: u16,
    pub api_key: String,
    pub model_id: String,
    pub hosts: Vec<String>,
    pub openai_bases: Vec<String>,
    pub anthropic_bases: Vec<String>,
    /// 一键复制给局域网对端的完整接入信息
    pub copy_text: String,
}

#[tauri::command]
pub async fn get_lan_share_info(runtime: State<'_, AppRuntime>) -> Result<LanShareInfo, String> {
    let config = config_with_listen_port(runtime.inner()).await;
    Ok(lan_share_info_from_config(&config))
}

/// 纯逻辑：从配置生成局域网分享信息。WebUI service 复用此函数，避免与 Tauri 通道漂移。
pub fn lan_share_info_from_config(config: &AppConfig) -> LanShareInfo {
    let port = config.port;
    let api_key = config.gateway_client_api_key.clone();
    // 期望模型 id 取当前激活 provider 的公开别名，因为写死 "sutai" 会在用户改别名后
    // 把错误入口复制给对端；取不到激活 provider 就留空，绝不编造默认值。
    let model_id = config
        .active_provider_id
        .as_deref()
        .and_then(|id| config.providers.iter().find(|provider| provider.id == id))
        .map(|provider| provider.public_model_id())
        .unwrap_or_default();
    if !config.allow_lan_access {
        return LanShareInfo {
            enabled: false,
            port,
            api_key,
            model_id,
            hosts: Vec::new(),
            openai_bases: Vec::new(),
            anthropic_bases: Vec::new(),
            copy_text: String::new(),
        };
    }
    let hosts = crate::lan::lan_ipv4_addrs();
    let openai_bases: Vec<String> = hosts
        .iter()
        .map(|h| format!("http://{h}:{port}/v1"))
        .collect();
    let anthropic_bases: Vec<String> = hosts
        .iter()
        .map(|h| format!("http://{h}:{port}"))
        .collect();
    let copy_text = if hosts.is_empty() {
        String::new()
    } else {
        let mut lines = Vec::new();
        for host in &hosts {
            lines.push(format!("OpenAI Base: http://{host}:{port}/v1"));
            lines.push(format!("Anthropic Base: http://{host}:{port}"));
        }
        lines.push(format!("API Key: {api_key}"));
        lines.push(format!("Model: {model_id}"));
        lines.join("\n")
    };
    LanShareInfo {
        enabled: true,
        port,
        api_key,
        model_id,
        hosts,
        openai_bases,
        anthropic_bases,
        copy_text,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccessEntry {
    /// 统一入口固定为 "sutai"，其余为 provider.name（中文文案由前端 i18n 补）
    pub label: String,
    pub base_url: String,
    pub model_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccessInfo {
    /// 当前监听 host（可能是 0.0.0.0，仅供展示）
    pub host: String,
    pub port: u16,
    /// 首个非 loopback IPv4；取不到回退 127.0.0.1
    pub lan_host: String,
    pub models_url: String,
    pub entries: Vec<LocalAccessEntry>,
}

/// 对外的接入信息一律用可达地址，避免把 0.0.0.0 直接给用户复制。
/// 网关绑定回环（host=127.0.0.1 且未开 LAN）时，探测到的 LAN/VPN 隧道候选
/// 一律不采用：此时网关只对本机可达，展示 LAN IP 是「显示可达实际不可达」的
/// 双重误导（2026-11-20 utun7=28.0.0.1 排查结论）。
fn build_local_access_info(
    host: &str,
    port: u16,
    providers: &[ProviderConfig],
    lan_hosts: &[String],
) -> LocalAccessInfo {
    let lan_host = if crate::model::is_loopback_bind_host(host) {
        "127.0.0.1".to_string()
    } else {
        crate::lan::preferred_lan_host(lan_hosts).unwrap_or_else(|| "127.0.0.1".to_string())
    };
    let base_url = format!("http://{lan_host}:{port}/v1");
    // 与 /v1/models 口径一致：启用中的 provider，按 public_model_id 去重
    let mut seen = std::collections::HashSet::new();
    seen.insert(CLIENT_MODEL_ID.to_string());
    let mut entries = vec![LocalAccessEntry {
        label: CLIENT_MODEL_ID.to_string(),
        base_url: base_url.clone(),
        model_id: CLIENT_MODEL_ID.to_string(),
    }];
    for provider in providers.iter().filter(|provider| provider.enabled) {
        let model_id = provider.public_model_id();
        if !seen.insert(model_id.clone()) {
            continue;
        }
        entries.push(LocalAccessEntry {
            label: provider.name.clone(),
            base_url: base_url.clone(),
            model_id,
        });
    }
    LocalAccessInfo {
        host: host.to_string(),
        port,
        lan_host: lan_host.clone(),
        models_url: format!("http://{lan_host}:{port}/v1/models"),
        entries,
    }
}

#[tauri::command]
pub async fn get_local_access_info(
    runtime: State<'_, AppRuntime>,
) -> Result<LocalAccessInfo, String> {
    let config = config_with_listen_port(runtime.inner()).await;
    Ok(local_access_info_from_config(&config))
}

/// 纯逻辑：从配置生成本地接入信息。WebUI service 复用此函数。
/// 传 bind_host()（含 allow_lan_access 展开）而不是 host 原值，
/// 保证展示地址与网关实际监听范围一致。
pub fn local_access_info_from_config(config: &AppConfig) -> LocalAccessInfo {
    build_local_access_info(
        &config.bind_host(),
        config.port,
        &config.providers,
        &crate::lan::lan_ipv4_addrs(),
    )
}

/// 真实请求本机网关 /v1/models 的报告；
/// 前端按 ok / model_ids / error 组合出成功、空、失败、加载四态。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelsFetchReport {
    pub ok: bool,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub model_ids: Vec<String>,
    pub error: Option<String>,
}

/// 真实 GET 本机网关 /v1/models。固定走 127.0.0.1：这是本机自测，
/// 网关绑定回环或 0.0.0.0 时回环都可达，与卡片展示的 LAN 地址无关。
pub async fn fetch_local_models_report(
    client: &reqwest::Client,
    config: &AppConfig,
) -> LocalModelsFetchReport {
    let url = format!("http://127.0.0.1:{}/v1/models", config.port);
    let api_key = config.gateway_client_api_key.trim().to_string();
    let started = std::time::Instant::now();

    let mut req = client.get(&url).timeout(std::time::Duration::from_secs(10));
    if !api_key.is_empty() {
        req = req.bearer_auth(&api_key);
    }
    let response = match req.send().await {
        Ok(resp) => resp,
        Err(err) => {
            return LocalModelsFetchReport {
                ok: false,
                status: None,
                elapsed_ms: started.elapsed().as_millis() as u64,
                model_ids: vec![],
                error: Some(error_hint::classify_request_error(&err)),
            };
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if !status.is_success() {
        tracing::warn!(%url, %status, "fetch_local_models http error");
        return LocalModelsFetchReport {
            ok: false,
            status: Some(status.as_u16()),
            elapsed_ms,
            model_ids: vec![],
            error: Some(error_hint::classify_http_error(status, &text)),
        };
    }

    let payload: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            return LocalModelsFetchReport {
                ok: false,
                status: Some(status.as_u16()),
                elapsed_ms,
                model_ids: vec![],
                error: Some(format!("响应不是合法 JSON：{err}")),
            };
        }
    };

    let model_ids = payload
        .get("data")
        .and_then(|data| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default();

    LocalModelsFetchReport {
        ok: true,
        status: Some(status.as_u16()),
        elapsed_ms,
        model_ids,
        error: None,
    }
}

/// 本机接入卡片的「请求」按钮：现场验证 /v1/models 可达并返回真实模型列表。
#[tauri::command]
pub async fn fetch_local_models(
    runtime: State<'_, AppRuntime>,
) -> Result<LocalModelsFetchReport, String> {
    let config = config_with_listen_port(runtime.inner()).await;
    Ok(fetch_local_models_report(runtime.gateway.http_client(), &config).await)
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
    if let Err(err) = crate::pet::migrate_pet_full_defaults(runtime).await {
        warn!(error = %err, "pet full-defaults migration failed");
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
    crate::logging::tail(&runtime.paths.log_file, lines.min(READ_LOGS_MAX_LINES))
        .map_err(|err| err.to_string())
}

/// 维护设置（PRD v1.1.11 R7）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceSettings {
    pub log_retention_days: u32,
}

/// 入参沿用项目约定：字段 snake_case，由前端原样传。
#[derive(Debug, Deserialize)]
pub struct MaintenanceSettingsInput {
    pub log_retention_days: u32,
}

#[tauri::command]
pub async fn clear_cache(runtime: State<'_, AppRuntime>) -> Result<CacheReport, String> {
    crate::maintenance::clear_cache(&runtime.paths.config_dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn get_maintenance_settings(
    runtime: State<'_, AppRuntime>,
) -> Result<MaintenanceSettings, String> {
    let log_retention_days = runtime.config.read().await.log_retention_days;
    Ok(MaintenanceSettings { log_retention_days })
}

#[tauri::command]
pub async fn set_maintenance_settings(
    runtime: State<'_, AppRuntime>,
    input: MaintenanceSettingsInput,
) -> Result<u32, String> {
    let log_retention_days = input.log_retention_days.min(MAX_LOG_RETENTION_DAYS);
    {
        let mut config = runtime.config.write().await;
        config.log_retention_days = log_retention_days;
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(log_retention_days)
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

/// 接入影响预览（只读）：列出将写入的变量名、配置字段、技能目录与风险。
#[tauri::command]
pub async fn preview_takeover(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
) -> Result<crate::takeover_preview::TakeoverPreview, String> {
    let config = runtime.config.read().await.clone();
    crate::takeover_preview::build_preview(&runtime.paths.config_dir, &config, &profile_id)
        .map_err(|e| e.to_string())
}

/// Agent 能力清单（只读）：设置页与 SOUL 共用同一份能力矩阵。
#[tauri::command]
pub async fn list_agent_capabilities() -> Result<Vec<crate::takeover_preview::AgentCapabilityView>, String> {
    Ok(crate::takeover_preview::list_capabilities())
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
            writable_fields: crate::agent_adapters::config_capability(&item.id)
                .writable_fields
                .iter()
                .map(|s| s.to_string())
                .collect(),
            config_note: crate::agent_adapters::config_capability(&item.id)
                .note
                .to_string(),
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

// ===== 配置预设：业务逻辑在 presets.rs，这里只做 Tauri State 适配 =====

#[tauri::command]
pub async fn list_config_presets(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<ConfigPreset>, String> {
    presets::list_config_presets_core(&runtime.paths.config_dir)
}

#[tauri::command]
pub async fn apply_config_preset(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<ConfigPreset, String> {
    presets::apply_config_preset_core(runtime.inner(), &id).await
}

#[tauri::command]
pub async fn save_config_preset(
    runtime: State<'_, AppRuntime>,
    name: String,
) -> Result<ConfigPreset, String> {
    presets::save_config_preset_core(runtime.inner(), &name).await
}

#[tauri::command]
pub async fn delete_config_preset(runtime: State<'_, AppRuntime>, id: String) -> Result<(), String> {
    presets::delete_config_preset_core(&runtime.paths.config_dir, &id)
}

#[tauri::command]
pub async fn export_config_preset(
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<String, String> {
    presets::export_config_preset_core(&runtime.paths.config_dir, &id)
}

#[tauri::command]
pub async fn import_config_preset(
    runtime: State<'_, AppRuntime>,
    json: String,
) -> Result<ConfigPreset, String> {
    presets::import_config_preset_core(&runtime.paths.config_dir, &json)
}

/// 流量悬浮窗配置（悬浮窗页面初始化时读取）。
#[tauri::command]
pub async fn get_overlay_config(runtime: State<'_, AppRuntime>) -> Result<OverlayConfig, String> {
    Ok(runtime.config.read().await.overlay.clone())
}

/// 桌面精灵只读交互：把主窗口带到前台。不修改任何配置、不执行写操作。
#[tauri::command]
pub fn focus_main_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    window.show().map_err(|e| format!("显示主窗口失败: {e}"))?;
    window.set_focus().map_err(|e| format!("聚焦主窗口失败: {e}"))?;
    Ok(())
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

/// 更新全局工作区规则：持久化并热更新到网关。
#[tauri::command]
pub async fn set_workspace_rules(
    runtime: State<'_, AppRuntime>,
    mut rules: WorkspaceRules,
) -> Result<AppConfig, String> {
    rules.rules_text = rules.rules_text.trim().to_string();
    rules.normalize();
    if rules.rules_text.chars().count() > 8000 {
        return Err("全局规则过长（最多约 8000 字）".to_string());
    }
    if rules.doc_habits.len() > 20 {
        return Err("文档习惯最多 20 条".to_string());
    }
    {
        let mut config = runtime.config.write().await;
        config.workspace_rules = rules;
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(runtime.config.read().await.clone())
}

/// 只更新文档习惯注入文案。与默认相同则存空，表示跟随内置文案。
#[tauri::command]
pub async fn set_workspace_inject_template(
    runtime: State<'_, AppRuntime>,
    template: String,
) -> Result<AppConfig, String> {
    let template = template.trim().to_string();
    if template.chars().count() > 8000 {
        return Err("注入文案过长（最多约 8000 字）".to_string());
    }
    let stored = if template == crate::workspace_rules::default_inject_template() {
        String::new()
    } else {
        template
    };
    {
        let mut config = runtime.config.write().await;
        config.workspace_rules.inject_template = stored;
    }
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok(runtime.config.read().await.clone())
}

#[tauri::command]
pub fn default_workspace_inject_template() -> String {
    crate::workspace_rules::default_inject_template().to_string()
}

/// 用当前配置的 webhook 发一条测试消息（不经模型）。
#[tauri::command]
pub async fn test_workspace_notify(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    let config = runtime.config.read().await.clone();
    let rules = &config.workspace_rules;
    if !rules.notify_enabled {
        return Err("请先启用 Webhook 通知".to_string());
    }
    let webhook = rules.notify_webhook.trim();
    if webhook.is_empty() {
        return Err("请先填写 Webhook".to_string());
    }
    gateway::send_webhook_markdown(
        runtime.gateway.http_client(),
        webhook,
        "SUGT 测试",
        "### SUGT 全局规则\nWebhook 通知已连通。",
    )
    .await
    .map_err(|err| err.to_string())?;
    Ok("已发送测试消息".to_string())
}

/// 校验精灵窗口配置；两个写入入口（全量 / 增量）共用同一套判据。
fn validate_overlay_config(cfg: &OverlayConfig) -> Result<(), String> {
    if !(0.1..=1.0).contains(&cfg.opacity) {
        return Err("立绘不透明度需在 0.1~1.0 之间".to_string());
    }
    // v1.4 三档 160×136 / 198×168 / 240×204；下限 120 保留给自定义小窗，上限留给自定义窗口
    if !(120.0..=800.0).contains(&cfg.width) || !(40.0..=420.0).contains(&cfg.height) {
        return Err("悬浮窗尺寸超出合理范围".to_string());
    }
    Ok(())
}

/// 校验 → 应用窗口 → 写回运行时配置 → 广播 → 持久化。
async fn persist_overlay_config(
    runtime: &AppRuntime,
    app: &AppHandle,
    mut cfg: OverlayConfig,
) -> Result<(), String> {
    validate_overlay_config(&cfg)?;
    crate::overlay::apply(app, &mut cfg).await?;
    {
        let mut config = runtime.config.write().await;
        config.overlay = cfg.clone();
    }
    // 通知精灵透明窗与右键菜单刷新（无需轮询 get_config）
    let _ = app.emit("sugt://overlay-config-changed", &cfg);
    runtime.persist().await.map_err(|err| err.to_string())
}

/// 更新桌面精灵配置（全量）：保留给旧调用方与干净升级路径。
#[tauri::command]
pub async fn set_overlay_config(
    runtime: State<'_, AppRuntime>,
    app: tauri::AppHandle,
    cfg: OverlayConfig,
) -> Result<(), String> {
    persist_overlay_config(&runtime, &app, cfg).await
}

/// 更新桌面精灵配置（按域增量）：设置页 / 菜单 / 透明窗各自只发自己改的字段，
/// 后端以当前配置为基准 merge，避免三个窗口的全量快照互相覆盖。
#[tauri::command]
pub async fn patch_overlay_config(
    runtime: State<'_, AppRuntime>,
    app: tauri::AppHandle,
    patch: OverlayConfigPatch,
) -> Result<(), String> {
    let current = runtime.config.read().await.overlay.clone();
    let mut cfg = current;
    patch.apply_to(&mut cfg);
    persist_overlay_config(&runtime, &app, cfg).await
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
    if input.tool_text_retry_enabled
        && !(1..=5).contains(&input.tool_text_retry_extra)
    {
        return Err("重试次数请填 1 到 5".to_string());
    }
    Ok(())
}

fn normalize_tool_text_retry(input: &crate::model::ProviderInput) -> (bool, u32) {
    if input.tool_text_retry_enabled {
        (true, input.tool_text_retry_extra)
    } else {
        (false, 0)
    }
}

fn open_path(path: &std::path::Path) -> Result<()> {
    crate::platform::open_path(path).map_err(|e| anyhow::anyhow!("{}", e))
}

#[tauri::command]
pub async fn get_quick_chat_history(
    runtime: State<'_, AppRuntime>,
) -> Result<ChatHistory, String> {
    Ok(quick_chat::load_history(&runtime.paths))
}

#[tauri::command]
pub async fn save_quick_chat_history(
    runtime: State<'_, AppRuntime>,
    history: ChatHistory,
) -> Result<(), String> {
    quick_chat::save_history(&runtime.paths, &history).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_quick_chat_history(
    runtime: State<'_, AppRuntime>,
) -> Result<(), String> {
    quick_chat::clear_history(&runtime.paths).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn quick_gateway_chat(
    runtime: State<'_, AppRuntime>,
    message: String,
    provider_id: Option<String>,
    history: Option<Vec<HistoryMsg>>,
) -> Result<ChatReply, String> {
    trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("消息不能为空".to_string());
    }
    let config = runtime.config.read().await.clone();
    let mut provider = if let Some(id) = provider_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
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
    if crate::experimental_free::is_experimental_free(&provider) {
        crate::experimental_free::apply_route(&mut provider);
    }

    let client = gateway::build_upstream_client_with_timeout(std::time::Duration::from_secs(60))
        .map_err(|e| e.to_string())?;

    let prior = history.unwrap_or_default();
    let messages = quick_chat::build_upstream_messages(&prior, &message);

    let anthropic_native = matches!(provider.protocol, ProviderProtocol::Anthropic)
        && provider.base_url.contains("anthropic.com");

    let started = Instant::now();
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
                "messages": messages
            }))
            .send()
            .await
    } else {
        let url = gateway::build_chat_url_for_provider(&provider);
        let mut req = client.post(&url).json(&serde_json::json!({
            "model": provider.model_name,
            "messages": messages,
            "stream": false,
            "max_tokens": 1024
        }));
        if !provider.api_key.trim().is_empty() {
            req = req.bearer_auth(provider.api_key.trim());
        }
        req.send().await
    }
    .map_err(|err| error_hint::classify_request_error(&err))?;

    let latency_ms = started.elapsed().as_millis() as u64;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(error_hint::classify_http_error(status, &text));
    }
    let body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("响应不是 JSON：{}", e))?;
    let content = quick_chat::extract_reply_text(&body).ok_or_else(|| {
        format!(
            "模型未返回文本：{}",
            text.chars().take(240).collect::<String>()
        )
    })?;

    let (input_tokens, output_tokens, tokens_estimated) =
        if let Some((input, output)) = quick_chat::extract_usage(&body) {
            (input, output, false)
        } else {
            (0, quick_chat::estimate_output_tokens(&content), true)
        };
    let tps = quick_chat::tokens_per_sec(output_tokens, latency_ms);

    Ok(ChatReply {
        content,
        latency_ms,
        input_tokens,
        output_tokens,
        tokens_per_sec: tps,
        tokens_estimated,
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        model_name: provider.model_name.clone(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str, alias: Option<&str>, enabled: bool) -> ProviderConfig {
        let mut config = ProviderConfig::new(
            name,
            "openai",
            "https://example.com/v1",
            "sk-test",
            "upstream-model",
        );
        config.enabled = enabled;
        config.model_alias = alias.map(|alias| alias.to_string());
        config
    }

    #[test]
    fn local_access_falls_back_to_loopback_without_lan_address() {
        let info = build_local_access_info("0.0.0.0", 8899, &[], &[]);
        assert_eq!(info.lan_host, "127.0.0.1");
        assert_eq!(info.host, "0.0.0.0");
        assert_eq!(info.port, 8899);
        assert!(info.models_url.ends_with("/v1/models"));
        assert_eq!(info.models_url, "http://127.0.0.1:8899/v1/models");
        assert_eq!(info.entries[0].base_url, "http://127.0.0.1:8899/v1");
    }

    #[test]
    fn loopback_bind_never_shows_lan_or_tunnel_candidates() {
        // 回环绑定 + 未开 LAN 时，探测到的 VPN 隧道 IP 一律不给用户
        let info = build_local_access_info("127.0.0.1", 18998, &[], &["28.0.0.1".to_string()]);
        assert_eq!(info.lan_host, "127.0.0.1");
        assert_eq!(info.models_url, "http://127.0.0.1:18998/v1/models");
    }

    #[test]
    fn non_loopback_prefers_private_lan_over_tunnel_ip() {
        let lan_hosts = vec!["28.0.0.1".to_string(), "192.168.31.134".to_string()];
        let info = build_local_access_info("0.0.0.0", 8899, &[], &lan_hosts);
        assert_eq!(info.lan_host, "192.168.31.134");
        assert_eq!(info.models_url, "http://192.168.31.134:8899/v1/models");
    }

    #[test]
    fn unified_entry_is_always_first() {
        let providers = vec![provider("厂商 A", Some("vendor-a"), true)];
        let lan_hosts = vec!["192.168.1.20".to_string()];
        let info = build_local_access_info("0.0.0.0", 8899, &providers, &lan_hosts);
        assert_eq!(info.lan_host, "192.168.1.20");
        assert_eq!(info.models_url, "http://192.168.1.20:8899/v1/models");
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[0].label, CLIENT_MODEL_ID);
        assert_eq!(info.entries[0].model_id, CLIENT_MODEL_ID);
        assert_eq!(info.entries[0].base_url, "http://192.168.1.20:8899/v1");
        assert_eq!(info.entries[1].label, "厂商 A");
        assert_eq!(info.entries[1].model_id, "vendor-a");
    }

    #[test]
    fn duplicate_public_model_id_is_deduplicated() {
        let providers = vec![
            provider("厂商 A", Some("shared"), true),
            provider("厂商 B", Some("shared"), true),
        ];
        let info = build_local_access_info("127.0.0.1", 8710, &providers, &[]);
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[1].label, "厂商 A");
        assert_eq!(info.entries[1].model_id, "shared");
        assert!(info
            .entries
            .iter()
            .all(|entry| entry.model_id != "厂商 B"));
    }

    #[test]
    fn disabled_provider_is_skipped() {
        let providers = vec![
            provider("厂商 A", Some("vendor-a"), true),
            provider("厂商 B", Some("vendor-b"), false),
        ];
        let info = build_local_access_info("127.0.0.1", 8710, &providers, &[]);
        assert_eq!(info.entries.len(), 2);
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.model_id == "vendor-a"));
        assert!(!info
            .entries
            .iter()
            .any(|entry| entry.model_id == "vendor-b"));
    }
}
