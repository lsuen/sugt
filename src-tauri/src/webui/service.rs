//! WebUI service 层：把 RPC cmd 映射到底层模块调用。
//!
//! 设计约定：
//! - commands.rs 一行不改。这里复刻 command 体中「仅依赖 AppRuntime」的部分，
//!   直接调用底层模块（config/store/client_keys/...）这些真相源，不经过 Tauri State。
//! - 只覆盖 P0+P1；窗口控制、overlay、宠物、系统文件打开等桌面专属能力不暴露。
//! - 入参统一 `serde_json::Value`，helper 反序列化为命令原本的入参类型，
//!   保证与 Tauri 通道的 JSON 语义一致（字段名、可选值、大小写）。

use crate::commands::{
    AddDiscoveredAgentInput, AppRuntime, CreateClientKeyInput, DeleteApiKeyPoolInput,
    DeleteClientKeyInput, DeleteRequestRuleInput, GatewaySettingsInput, ListModelRegistryInput,
    ListModelsInput, MaintenanceSettingsInput, ProbeModelsInput, ResetClientKeyUsageInput,
    ResetKeyUsageInput, SaveApiKeyPoolInput, SaveRequestRuleInput, SaveStrategyBoxInput,
    SetModelEnabledInput, StrategyBoxIdInput, StrategyMemberStatus, TakeoverProfileInput,
    UpdateClientKeyInput,
};
use crate::model::{AppConfig, ProviderInput};
use crate::store::paths::StorePaths;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Instant;

fn parse<T: DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|e| format!("参数格式错误: {}", e))
}

/// 单参数命令：前端传 `{"arg": <值>}` 或裸值。
/// 兼容 Tauri 约定：前端按字段名传单个标量（如 `{"lines":200}` / `{"id":"..."}`）。
fn single<T: DeserializeOwned>(args: &Value) -> Result<T, String> {
    if let Some(v) = args.get("arg").or_else(|| args.get("value")) {
        return parse(v);
    }
    if let Some(obj) = args.as_object() {
        if obj.len() == 1 {
            if let Some(v) = obj.values().next() {
                return parse(v);
            }
        }
    }
    parse(args)
}

/// 多参数命令：按字段名取参。`keys` 可同时给出 camelCase / snake_case 兼容别名。
fn field<T: DeserializeOwned>(args: &Value, keys: &[&str]) -> Result<T, String> {
    for key in keys {
        if let Some(v) = args.get(key) {
            return parse::<T>(v);
        }
    }
    Err(format!("缺少参数: {}", keys.join("/")))
}

/// 可选字段：缺失或显式 null 都返回 None。
fn field_opt<T: DeserializeOwned>(args: &Value, keys: &[&str]) -> Result<Option<T>, String> {
    for key in keys {
        if let Some(v) = args.get(key) {
            if v.is_null() {
                return Ok(None);
            }
            return parse::<T>(v).map(Some);
        }
    }
    Ok(None)
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("序列化失败: {}", e))
}

// ============================================================================
// 以下为 commands.rs 中「私有 helper」的等价实现（commands.rs 一行不改）。
// 只复刻确实被 P0+P1 命令依赖的部分。能直接调用的 pub(crate) helper 不重复实现。
// ============================================================================

async fn gateway_is_reachable(config: &AppConfig) -> bool {
    crate::gateway_daemon::is_reachable(&config.client_host(), config.port).await
}

async fn clients_env_with_reachability(
    config_dir: &std::path::Path,
    config: &AppConfig,
) -> crate::clients::ClientsEnvStatus {
    let mut status = crate::clients::status(config_dir, config);
    status.gateway_reachable = gateway_is_reachable(config).await;
    status
}

async fn sync_gateway_port_if_changed(runtime: &AppRuntime) -> Result<(), String> {
    let gw_config = runtime.gateway.config().await;
    let mut cfg = runtime.config.write().await;
    if cfg.port == gw_config.port {
        return Ok(());
    }
    cfg.port = gw_config.port;
    runtime.persist().await.map_err(|e| e.to_string())?;
    crate::clients::write_launch_scripts(&runtime.paths, &cfg).map_err(|e| e.to_string())?;
    crate::clients::repair(&runtime.paths.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(())
}

async fn ensure_gateway_listening(runtime: &AppRuntime, auto_start: bool) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    if gateway_is_reachable(&config).await {
        return Ok(());
    }
    if !auto_start {
        return Err(
            "gateway_not_running:本地网关未运行，请先启动网关或使用「启动网关并接管」".to_string(),
        );
    }
    crate::trial::ensure_allowed(&runtime.paths).map_err(|err| err.to_string())?;

    if !runtime.gateway.is_running().await {
        if let Err(err) = runtime.gateway.start().await {
            tracing::warn!(error = %err, "embedded gateway start failed, trying detached serve");
        }
    }
    if crate::gateway_daemon::wait_reachable(&config.client_host(), config.port, 8).await {
        return Ok(());
    }

    if let Err(err) = crate::gateway_daemon::spawn_detached(&runtime.paths.config_dir) {
        tracing::warn!(error = %err, "detached gateway spawn failed");
    }
    if crate::gateway_daemon::wait_reachable(&config.client_host(), config.port, 20).await {
        sync_gateway_port_if_changed(runtime).await?;
        return Ok(());
    }

    Err(
        "启动网关失败：端口可能被占用或系统拒绝监听。请检查配置端口、关闭占用程序，或改用 9878 等端口后点击「修复接管」"
            .to_string(),
    )
}

async fn ensure_gateway_for_takeover(runtime: &AppRuntime, auto_start: bool) -> Result<(), String> {
    ensure_gateway_listening(runtime, auto_start).await
}

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

fn quota_from_input(input: Option<crate::commands::ClientQuotaInput>) -> crate::client_keys::ClientQuota {
    let input = input.unwrap_or_default();
    crate::client_keys::ClientQuota {
        request_limit: input.request_limit.flatten(),
        token_limit: input.token_limit.flatten(),
    }
}

fn store_paths(runtime: &AppRuntime) -> StorePaths {
    StorePaths::from_app(&runtime.paths)
}

/// store/commands.rs 的 sorted_repo_views 等价实现（其依赖的 SkillRepoView 构造私有）。
fn skill_repo_views(
    repos: &[crate::store::repos::SkillRepo],
    sp: &StorePaths,
) -> Vec<crate::store::commands::SkillRepoView> {
    use crate::store::commands::SkillRepoView;
    let mut views: Vec<_> = repos
        .iter()
        .map(|repo| SkillRepoView {
            id: repo.id.clone(),
            owner: repo.owner.clone(),
            repo: repo.repo.clone(),
            branch: repo.branch.clone(),
            weight: repo.weight,
            enabled: repo.enabled,
            clone_url: repo.clone_url(),
            label: repo.label(),
            last_refresh_at: repo.last_refresh_at.clone(),
            last_error: repo.last_error.clone(),
            skill_count: crate::store::install::repo_skill_count(sp, &repo.id),
        })
        .collect();
    views.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label.cmp(&b.label)));
    views
}

/// RPC 分发入口。未知命令明确报错，不静默吞掉。
pub async fn dispatch(runtime: &AppRuntime, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        // ===== P0 状态/配置 =====
        "get_config" => to_value(runtime.config.read().await.clone()),

        // ===== P0 Provider =====
        "list_providers" => to_value(crate::commands::list_providers_core(runtime).await?),
        "list_experimental_free_candidates" => {
            to_value(crate::commands::list_experimental_free_candidates_core())
        }
        "list_model_registry" => {
            let input: ListModelRegistryInput = field(args, &["input"])?;
            to_value(crate::commands::list_model_registry_core(runtime, input).await?)
        }
        "save_provider" => {
            let input: ProviderInput = field(args, &["input"])?;
            to_value(crate::commands::save_provider_core(runtime, input).await?)
        }
        "delete_provider" => {
            let id: String = single(args)?;
            to_value(crate::commands::delete_provider_core(runtime, id).await?)
        }
        "restore_experimental_free" => {
            to_value(crate::commands::restore_experimental_free_core(runtime).await?)
        }
        "probe_experimental_free_models" => {
            to_value(crate::commands::probe_experimental_free_models_core(runtime).await?)
        }
        "set_active_provider" => {
            let id: String = single(args)?;
            to_value(crate::commands::set_active_provider_core(runtime, id).await?)
        }
        "set_provider_enabled" => {
            let id: String = field(args, &["id"])?;
            let enabled: bool = field(args, &["enabled"])?;
            to_value(crate::commands::set_provider_enabled_core(runtime, id, enabled).await?)
        }
        "test_provider" => {
            let id: String = single(args)?;
            to_value(crate::commands::test_provider_core(runtime, id).await?)
        }
        "test_provider_draft" => {
            let input: ListModelsInput = field(args, &["input"])?;
            let model_name: String = field(args, &["modelName", "model_name"])?;
            to_value(crate::commands::test_provider_draft_core(runtime, input, model_name).await?)
        }
        "list_provider_models" => {
            let input: ListModelsInput = field(args, &["input"])?;
            to_value(crate::commands::list_provider_models_core(runtime, input).await?)
        }
        "set_model_enabled" => {
            let input: SetModelEnabledInput = field(args, &["input"])?;
            to_value(crate::commands::set_model_enabled_core(runtime, input).await?)
        }
        "probe_models" => {
            let input: ProbeModelsInput = field(args, &["input"])?;
            to_value(crate::commands::probe_models_core(runtime, input).await?)
        }

        // ===== P0 网关设置 =====
        "get_lan_share_info" => {
            let config = runtime.config.read().await.clone();
            to_value(crate::commands::lan_share_info_from_config(&config))
        }
        "get_local_access_info" => {
            let config = runtime.config.read().await.clone();
            to_value(crate::commands::local_access_info_from_config(&config))
        }
        "fetch_local_models" => {
            let config = runtime.config.read().await.clone();
            to_value(
                crate::commands::fetch_local_models_report(runtime.gateway.http_client(), &config)
                    .await,
            )
        }
        "update_gateway_settings" => {
            let input: GatewaySettingsInput = field(args, &["input"])?;
            let mut lan_changed = false;
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
            if lan_changed && runtime.gateway.is_running().await {
                let _ = runtime.gateway.stop().await;
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                runtime.gateway.replace_config(snapshot.clone()).await;
                if let Err(err) = runtime.gateway.start().await {
                    tracing::warn!(error = %err, "restart gateway after LAN toggle failed");
                    return Err(crate::error_hint::format_gateway_start_error(&err));
                }
            } else if lan_changed {
                runtime.gateway.replace_config(snapshot.clone()).await;
            }
            to_value(snapshot)
        }
        "set_failover" => {
            let enabled: bool = single(args)?;
            runtime.config.write().await.failover_enabled = enabled;
            to_value(runtime.persist().await.map_err(|err| err.to_string())?)
        }
        "start_gateway" => to_value(crate::commands::ensure_gateway_running(runtime).await?),
        "stop_gateway" => {
            runtime.gateway.stop().await.map_err(|err| err.to_string())?;
            crate::gateway_daemon::stop_detached(&runtime.paths.config_dir)
                .map_err(|err| err.to_string())?;
            crate::gateway_daemon::kill_sugt_cli_processes();
            to_value(())
        }

        // ===== P0 客户端接入 =====
        "get_clients_env_status" => {
            let config = runtime.config.read().await.clone();
            to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
        }
        "install_clients_env" => {
            let auto_start: Option<bool> = field_opt(args, &["autoStart", "auto_start"])?;
            let client: Option<String> = field_opt(args, &["client"])?;
            ensure_gateway_for_takeover(runtime, auto_start.unwrap_or(false)).await?;
            let target = client
                .as_deref()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude".to_string());
            {
                let mut config = runtime.config.write().await;
                if target == "codex" || target == "all" {
                    crate::commands::add_active_takeover_id(&mut config, "codex");
                }
                if target == "claude" || target == "all" {
                    crate::commands::add_active_takeover_id(&mut config, "claude-code");
                }
            }
            runtime.persist().await.map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            crate::clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
            crate::clients::install_for(&runtime.paths.config_dir, &config, &target)
                .map_err(|e| e.to_string())?;
            to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
        }
        "repair_clients_env" => {
            let config = runtime.config.read().await.clone();
            crate::clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
            crate::clients::repair(&runtime.paths.config_dir, &config).map_err(|e| e.to_string())?;
            to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
        }
        "uninstall_clients_env" => {
            let client: Option<String> = field_opt(args, &["client"])?;
            let target = client
                .as_deref()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "all".to_string());
            {
                let mut config = runtime.config.write().await;
                if target == "codex" || target == "all" {
                    crate::commands::remove_active_takeover_id(&mut config, "codex");
                }
                if target == "claude" || target == "all" {
                    crate::commands::remove_active_takeover_id(&mut config, "claude-code");
                }
                if target == "all" {
                    config.active_takeover_ids.clear();
                    crate::commands::sync_legacy_takeover_flags(&mut config);
                }
            }
            runtime.persist().await.map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            crate::clients::uninstall_for(&runtime.paths.config_dir, &config, &target)
                .map_err(|e| e.to_string())?;
            to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
        }
        "set_auto_takeover" => {
            let enabled: bool = single(args)?;
            if enabled {
                ensure_gateway_for_takeover(runtime, true).await?;
                {
                    let mut config = runtime.config.write().await;
                    config.auto_takeover_enabled = true;
                    config.claude_takeover_enabled = true;
                }
                runtime.persist().await.map_err(|e| e.to_string())?;
                let config = runtime.config.read().await.clone();
                crate::clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
                crate::clients::install_for(&runtime.paths.config_dir, &config, "claude")
                    .map_err(|e| e.to_string())?;
                if config.codex_takeover_enabled {
                    crate::clients::install_for(&runtime.paths.config_dir, &config, "codex")
                        .map_err(|e| e.to_string())?;
                }
                to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
            } else {
                {
                    let mut config = runtime.config.write().await;
                    config.auto_takeover_enabled = false;
                    config.claude_takeover_enabled = false;
                    config.codex_takeover_enabled = false;
                }
                runtime.persist().await.map_err(|e| e.to_string())?;
                let config = runtime.config.read().await.clone();
                crate::clients::uninstall(&runtime.paths.config_dir, &config).map_err(|e| e.to_string())?;
                to_value(clients_env_with_reachability(&runtime.paths.config_dir, &config).await)
            }
        }

        // ===== P0 key 池 =====
        "list_client_keys" => to_value(
            runtime
                .gateway
                .client_keys_snapshot()
                .await
                .keys
                .iter()
                .map(crate::client_keys::ClientKeyView::from)
                .collect::<Vec<_>>(),
        ),
        "list_api_key_pools" => to_value(
            runtime
                .gateway
                .key_pools_snapshot()
                .await
                .pools
                .iter()
                .map(crate::api_key_pool::ApiKeyPoolView::from)
                .collect::<Vec<_>>(),
        ),
        "create_client_key" => {
            let input: CreateClientKeyInput = field(args, &["input"])?;
            crate::client_keys::validate_name(&input.name)?;
            let name = input.name.trim().to_string();
            let mut file = runtime.gateway.client_keys_snapshot().await;
            if file.keys.iter().any(|key| key.name.trim() == name.as_str()) {
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
            if let Some(existing) =
                duplicate_key_source(runtime, &file.keys, &key, !primary).await
            {
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
            to_value(crate::client_keys::ClientKeyView::from(&created))
        }
        "update_client_key" => {
            let input: UpdateClientKeyInput = field(args, &["input"])?;
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
                if let Some(existing) = duplicate_key_source(runtime, &others, &key, !is_primary).await {
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

            if is_primary && key_changed {
                {
                    let mut config = runtime.config.write().await;
                    config.gateway_client_api_key = updated.key.clone();
                }
                runtime.persist().await.map_err(|err| err.to_string())?;
            }

            to_value(crate::client_keys::ClientKeyView::from(&updated))
        }
        "delete_client_key" => {
            let input: DeleteClientKeyInput = field(args, &["input"])?;
            let mut file = runtime.gateway.client_keys_snapshot().await;
            let existing = file
                .keys
                .iter()
                .find(|key| key.id == input.id)
                .ok_or_else(|| format!("未找到入站密钥: {}", input.id))?;
            crate::client_keys::validate_delete(existing)?;
            crate::client_keys::remove_key(&mut file.keys, &input.id);
            to_value(runtime.gateway.persist_client_keys(file).await?)
        }
        "reset_client_key_usage" => {
            let input: ResetClientKeyUsageInput = field(args, &["input"])?;
            let mut file = runtime.gateway.client_keys_snapshot().await;
            let entry = file
                .keys
                .iter_mut()
                .find(|key| key.id == input.id)
                .ok_or_else(|| format!("未找到入站密钥: {}", input.id))?;
            crate::client_keys::reset_usage(entry);
            let view = crate::client_keys::ClientKeyView::from(&*entry);
            runtime.gateway.persist_client_keys(file).await?;
            to_value(view)
        }

        // ===== P0 技能管理 =====
        "store_list_catalog" => {
            let query: Option<String> = single(args)?;
            let sp = store_paths(runtime);
            let items = crate::store::catalog::build_catalog(&sp).map_err(|e| e.to_string())?;
            to_value(
                items
                    .into_iter()
                    .filter(|item| {
                        query
                            .as_ref()
                            .map(|q| {
                                q.trim().is_empty()
                                    || item.name.to_lowercase().contains(&q.to_lowercase())
                            })
                            .unwrap_or(true)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        "store_list_repos" => to_value(
            crate::store::repos::load_repos(&crate::store::paths::StorePaths::from_app(
                &runtime.paths,
            ))
            .map_err(|e| e.to_string())?,
        ),
        "store_install_skill" => {
            let skill_id: String = field(args, &["skillId", "skill_id"])?;
            to_value(
                crate::store::install::install_skill(&store_paths(runtime), &skill_id)
                    .map_err(|e| e.to_string())?,
            )
        }
        "store_install_git_skills" => {
            let url: String = field(args, &["url"])?;
            let branch: Option<String> = field_opt(args, &["branch"])?;
            to_value(
                crate::store::github_install::install_from_git_url(
                    &store_paths(runtime),
                    &url,
                    branch.as_deref(),
                )
                .map_err(|e| e.to_string())?,
            )
        }
        "store_uninstall_skill" => {
            let skill_id: String = field(args, &["skillId", "skill_id"])?;
            to_value(
                crate::store::install::uninstall_skill(&store_paths(runtime), &skill_id)
                    .map_err(|e| e.to_string())?,
            )
        }
        "store_mount_skill" => {
            let skill_id: String = field(args, &["skillId", "skill_id"])?;
            let target: Option<String> = field_opt(args, &["target"])?;
            let mount_target = crate::store::install::MountTarget::parse(target.as_deref());
            to_value(
                crate::store::install::mount_skill(&store_paths(runtime), &skill_id, mount_target)
                    .map_err(|e| e.to_string())?,
            )
        }
        "store_unmount_skill" => {
            let skill_id: String = field(args, &["skillId", "skill_id"])?;
            let target: Option<String> = field_opt(args, &["target"])?;
            let mount_target = crate::store::install::MountTarget::parse(target.as_deref());
            to_value(
                crate::store::install::unmount_skill(&store_paths(runtime), &skill_id, mount_target)
                    .map_err(|e| e.to_string())?,
            )
        }
        "store_mount_skills" => {
            let skill_ids: Vec<String> = field(args, &["skillIds", "skill_ids"])?;
            let agent_ids: Vec<String> = field(args, &["agentIds", "agent_ids"])?;
            let custom_path: Option<String> = field_opt(args, &["customPath", "custom_path"])?;
            let sp = store_paths(runtime);
            sp.ensure_dirs().map_err(|e| e.to_string())?;
            to_value(
                crate::store::hub::mount_skills_to_agents(
                    &sp,
                    &runtime.paths.config_dir,
                    &skill_ids,
                    &agent_ids,
                    custom_path.as_deref(),
                )
                .map_err(|e| e.to_string())?,
            )
        }
        "store_unmount_skills" => {
            let skill_ids: Vec<String> = field(args, &["skillIds", "skill_ids"])?;
            let agent_ids: Vec<String> = field(args, &["agentIds", "agent_ids"])?;
            let custom_path: Option<String> = field_opt(args, &["customPath", "custom_path"])?;
            to_value(
                crate::store::hub::unmount_skills_from_agents(
                    &store_paths(runtime),
                    &runtime.paths.config_dir,
                    &skill_ids,
                    &agent_ids,
                    custom_path.as_deref(),
                )
                .map_err(|e| e.to_string())?,
            )
        }
        "store_create_local_skill" => {
            let name: String = field(args, &["name"])?;
            let sp = store_paths(runtime);
            sp.ensure_dirs().map_err(|e| e.to_string())?;
            to_value(crate::store::hub::create_local_skill(&sp, &name).map_err(|e| e.to_string())?)
        }
        "store_list_local_skills" => {
            let sp = store_paths(runtime);
            sp.ensure_dirs().map_err(|e| e.to_string())?;
            to_value(
                crate::store::hub::list_local_skills(&sp, &runtime.paths.config_dir)
                    .map_err(|e| e.to_string())?,
            )
        }
        "store_list_skill_agents" => to_value(
            crate::store::hub::list_skill_agents(&runtime.paths.config_dir),
        ),
        "store_add_repo" => {
            let url: String = field(args, &["url"])?;
            let branch: String = field(args, &["branch"])?;
            let weight: Option<i32> = field_opt(args, &["weight"])?;
            let owner: Option<String> = field_opt(args, &["owner"])?;
            let repo: Option<String> = field_opt(args, &["repo"])?;
            let sp = store_paths(runtime);
            let resolved_url = if !url.trim().is_empty() {
                url
            } else if let (Some(owner), Some(repo)) = (owner, repo) {
                format!("{}/{}", owner.trim(), repo.trim())
            } else {
                return Err("请填写完整 git 地址".to_string());
            };
            let repos = crate::store::repos::add_repo(
                &sp,
                &resolved_url,
                &branch,
                weight.unwrap_or(0),
            )
            .map_err(|e| e.to_string())?;
            to_value(skill_repo_views(&repos, &sp))
        }
        "store_update_repo" => {
            let repo_id: String = field(args, &["repoId", "repo_id"])?;
            let branch: Option<String> = field_opt(args, &["branch"])?;
            let weight: Option<i32> = field_opt(args, &["weight"])?;
            let enabled: Option<bool> = field_opt(args, &["enabled"])?;
            let sp = store_paths(runtime);
            let repos = crate::store::repos::update_repo(
                &sp,
                &repo_id,
                branch.as_deref(),
                weight,
                enabled,
            )
            .map_err(|e| e.to_string())?;
            to_value(skill_repo_views(&repos, &sp))
        }
        "store_remove_repo" => {
            let repo_id: String = field(args, &["repoId", "repo_id"])?;
            let sp = store_paths(runtime);
            let repos = crate::store::repos::remove_repo(&sp, &repo_id).map_err(|e| e.to_string())?;
            to_value(skill_repo_views(&repos, &sp))
        }
        "store_refresh_repo" => {
            let repo_id: String = field(args, &["repoId", "repo_id"])?;
            let sp = store_paths(runtime);
            let views: Result<Vec<crate::store::commands::SkillRepoView>, String> =
                tokio::task::spawn_blocking(move || {
                    crate::store::repos::refresh_repo_by_id(&sp, &repo_id)
                        .map_err(|e| e.to_string())?;
                    let repos = crate::store::repos::load_repos(&sp).map_err(|e| e.to_string())?;
                    Ok(skill_repo_views(&repos, &sp))
                })
                .await
                .map_err(|e| e.to_string())?;
            to_value(views)
        }
        "store_refresh_all_repos" => {
            let sp = store_paths(runtime);
            let views: Result<Vec<crate::store::commands::SkillRepoView>, String> =
                tokio::task::spawn_blocking(move || {
                    let repos =
                        crate::store::repos::refresh_all_repos(&sp).map_err(|e| e.to_string())?;
                    Ok(skill_repo_views(&repos, &sp))
                })
                .await
                .map_err(|e| e.to_string())?;
            to_value(views)
        }
        "store_ensure_preferred_repo" => {
            let sp = store_paths(runtime);
            let result = tokio::task::spawn_blocking(move || {
                crate::store::repos::ensure_preferred_repo_cached(&sp).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
            to_value(result)
        }
        "store_parse_repo_url" => {
            let url: String = field(args, &["url"])?;
            to_value(crate::store::repos::parse_git_url(&url).map_err(|e| e.to_string())?)
        }
        "store_test_repo" => {
            let url: String = field(args, &["url"])?;
            let branch: String = field(args, &["branch"])?;
            let result = tokio::task::spawn_blocking(move || {
                crate::store::repos::test_repo_access(&url, &branch).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())?;
            to_value(result)
        }
        "store_get_plugin_panel" => {
            let client: String = field(args, &["client"])?;
            to_value(crate::store::plugins::plugin_panel(&client).map_err(|e| e.to_string())?)
        }
        "store_get_settings" => to_value(
            crate::store::settings::load_settings(&store_paths(runtime)).map_err(|e| e.to_string())?,
        ),
        "store_set_settings" => {
            let settings: crate::store::settings::StoreSettings = field(args, &["settings"])?;
            crate::store::settings::save_settings(&store_paths(runtime), &settings)
                .map_err(|e| e.to_string())?;
            to_value(settings)
        }
        "store_get_client_paths" => {
            let sp = store_paths(runtime);
            sp.ensure_dirs().map_err(|e| e.to_string())?;
            to_value(crate::store::commands::StoreClientPathsView {
                staging_dir: sp.skills_dir.display().to_string(),
                claude_skills: StorePaths::claude_skills_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                codex_skills: StorePaths::codex_skills_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                claude_plugins: StorePaths::claude_plugins_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            })
        }
        "store_test_github_proxy" => {
            let prefix: String = field(args, &["prefix"])?;
            to_value(crate::store::settings::test_github_proxy(&prefix).map_err(|e| e.to_string())?)
        }

        // ===== P1 日志/维护 =====
        "read_logs" => {
            let lines: usize = single(args)?;
            to_value(
                crate::logging::tail(&runtime.paths.log_file, lines.min(500))
                    .map_err(|e| e.to_string())?,
            )
        }
        "get_maintenance_settings" => {
            to_value(serde_json::json!({"logRetentionDays": runtime.config.read().await.log_retention_days}))
        }
        "clear_cache" => to_value(
            crate::maintenance::clear_cache(&runtime.paths.config_dir).map_err(|err| err.to_string())?,
        ),
        "set_maintenance_settings" => {
            let input: MaintenanceSettingsInput = field(args, &["input"])?;
            let log_retention_days = input.log_retention_days.min(crate::model::MAX_LOG_RETENTION_DAYS);
            {
                let mut config = runtime.config.write().await;
                config.log_retention_days = log_retention_days;
            }
            runtime.persist().await.map_err(|err| err.to_string())?;
            to_value(log_retention_days)
        }

        // ===== P1 策略盒/规则/key 池 =====
        "list_strategy_boxes" => to_value(runtime.gateway.strategy_boxes().await),
        "save_strategy_box" => {
            let input: SaveStrategyBoxInput = field(args, &["input"])?;
            let providers = runtime.config.read().await.providers.clone();
            let config_dir = runtime.paths.config_dir.clone();
            let mut boxes = crate::strategy_box::load_boxes(&config_dir);
            crate::strategy_box::validate(&input.spec, &providers)?;
            crate::strategy_box::check_public_id_conflict(&boxes, &input.spec)?;
            let saved = crate::strategy_box::upsert(&mut boxes, input.spec);
            crate::strategy_box::save_boxes(&config_dir, &boxes).map_err(|err| err.to_string())?;
            runtime.gateway.set_strategy_boxes(boxes).await;
            to_value(saved)
        }
        "delete_strategy_box" => {
            let input: StrategyBoxIdInput = field(args, &["input"])?;
            let config_dir = runtime.paths.config_dir.clone();
            let mut boxes = crate::strategy_box::load_boxes(&config_dir);
            let before = boxes.len();
            boxes.retain(|item| item.id != input.box_id);
            if boxes.len() == before {
                return Err(format!("未找到策略盒子: {}", input.box_id));
            }
            crate::strategy_box::save_boxes(&config_dir, &boxes).map_err(|err| err.to_string())?;
            runtime.gateway.set_strategy_boxes(boxes).await;
            to_value(())
        }
        "get_strategy_box_status" => {
            let input: StrategyBoxIdInput = field(args, &["input"])?;
            let state = runtime.gateway.strategy_state();
            let guard = state.read().await;
            let box_spec = guard
                .boxes
                .iter()
                .find(|item| item.id == input.box_id)
                .ok_or_else(|| format!("未找到策略盒子: {}", input.box_id))?;
            let now = Instant::now();
            let statuses: Vec<StrategyMemberStatus> = box_spec
                .members
                .iter()
                .map(|member| {
                    let health = guard
                        .runtime
                        .status(&crate::strategy_box::member_key(&box_spec.id, member));
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
            to_value(statuses)
        }
        "list_request_rules" => to_value(runtime.gateway.request_rules_snapshot().await),
        "save_request_rule" => {
            let input: SaveRequestRuleInput = field(args, &["input"])?;
            let config_dir = runtime.paths.config_dir.clone();
            let mut rules = crate::request_rules::load_rules(&config_dir);
            crate::request_rules::validate(&input.spec, &rules)?;
            let saved = crate::request_rules::upsert(&mut rules, input.spec);
            crate::request_rules::save_rules(&config_dir, &rules).map_err(|err| err.to_string())?;
            runtime.gateway.set_request_rules(rules).await;
            to_value(saved)
        }
        "delete_request_rule" => {
            let input: DeleteRequestRuleInput = field(args, &["input"])?;
            let config_dir = runtime.paths.config_dir.clone();
            let mut rules = crate::request_rules::load_rules(&config_dir);
            if crate::request_rules::is_builtin_rule_id(&input.rule_id) {
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
            crate::request_rules::save_rules(&config_dir, &rules).map_err(|err| err.to_string())?;
            runtime.gateway.set_request_rules(rules).await;
            to_value(())
        }
        "save_api_key_pool" => {
            let input: SaveApiKeyPoolInput = field(args, &["input"])?;
            let mut file = runtime.gateway.key_pools_snapshot().await;
            crate::api_key_pool::validate(&input.spec, &file.pools)?;
            let saved = crate::api_key_pool::upsert(&mut file.pools, input.spec);
            runtime.gateway.persist_key_pools(file).await?;
            to_value(saved)
        }
        "delete_api_key_pool" => {
            let input: DeleteApiKeyPoolInput = field(args, &["input"])?;
            let mut file = runtime.gateway.key_pools_snapshot().await;
            if !crate::api_key_pool::remove_pool(&mut file.pools, &input.pool_id) {
                return Err(format!("未找到 API Key 池: {}", input.pool_id));
            }
            to_value(runtime.gateway.persist_key_pools(file).await?)
        }
        "reset_key_usage" => {
            let input: ResetKeyUsageInput = field(args, &["input"])?;
            let mut file = runtime.gateway.key_pools_snapshot().await;
            let pool = file
                .pools
                .iter_mut()
                .find(|pool| pool.id == input.pool_id)
                .ok_or_else(|| format!("未找到 API Key 池: {}", input.pool_id))?;
            crate::api_key_pool::reset_usage(pool, input.key_id.as_deref());
            to_value(runtime.gateway.persist_key_pools(file).await?)
        }

        // ===== P1 接管配置 =====
        "list_takeover_profiles" => {
            let cfg = runtime.config.read().await;
            to_value(crate::takeover_profiles::list_profile_views(
                &runtime.paths.config_dir,
                &cfg,
            ))
        }
        "list_agent_capabilities" => to_value(crate::takeover_preview::list_capabilities()),
        "apply_takeover_profile" => {
            let profile_id: String = field(args, &["profileId", "profile_id"])?;
            let auto_start: Option<bool> = field_opt(args, &["autoStart", "auto_start"])?;
            ensure_gateway_for_takeover(runtime, auto_start.unwrap_or(true)).await?;
            let config = runtime.config.read().await.clone();
            crate::clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;
            crate::takeover_profiles::apply_profile(&runtime.paths.config_dir, &config, &profile_id)
                .map_err(|e| e.to_string())?;
            {
                let mut cfg = runtime.config.write().await;
                crate::commands::add_active_takeover_id(&mut cfg, &profile_id);
            }
            runtime.persist().await.map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            to_value(
                crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
                    .map(|p| {
                        crate::takeover_profiles::profile_view_with_config_dir(
                            Some(&runtime.paths.config_dir),
                            &config,
                            &p,
                        )
                    })
                    .ok_or_else(|| format!("未找到模板：{}", profile_id))?,
            )
        }
        "release_takeover_profile" => {
            let profile_id: String = field(args, &["profileId", "profile_id"])?;
            let config = runtime.config.read().await.clone();
            crate::takeover_profiles::release_profile(&runtime.paths.config_dir, &config, &profile_id)
                .map_err(|e| e.to_string())?;
            {
                let mut cfg = runtime.config.write().await;
                crate::commands::remove_active_takeover_id(&mut cfg, &profile_id);
            }
            runtime.persist().await.map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            to_value(
                crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
                    .map(|p| {
                        crate::takeover_profiles::profile_view_with_config_dir(
                            Some(&runtime.paths.config_dir),
                            &config,
                            &p,
                        )
                    })
                    .ok_or_else(|| format!("未找到模板：{}", profile_id))?,
            )
        }
        "preview_takeover" => {
            let profile_id: String = field(args, &["profileId", "profile_id"])?;
            let config = runtime.config.read().await.clone();
            to_value(
                crate::takeover_preview::build_preview(
                    &runtime.paths.config_dir,
                    &config,
                    &profile_id,
                )
                .map_err(|e| e.to_string())?,
            )
        }
        "can_ai_parse_takeover" => {
            let config = runtime.config.read().await.clone();
            to_value(config.providers.iter().any(|p| p.enabled))
        }
        "parse_takeover_config_path" => {
            let path: String = field(args, &["path"])?;
            let save: Option<bool> = field_opt(args, &["save"])?;
            let config = runtime.config.read().await.clone();
            let path_buf = std::path::PathBuf::from(path.trim());
            if !path_buf.is_file() {
                return Err(format!("文件不存在：{}", path_buf.display()));
            }
            let profile = crate::takeover_profiles::parse_config_file_heuristic(&path_buf, &config)
                .map_err(|e| e.to_string())?;
            if save.unwrap_or(false) {
                let mut custom =
                    crate::takeover_profiles::load_custom_profiles(&runtime.paths.config_dir);
                custom.retain(|p| p.id != profile.id);
                custom.push(profile.clone());
                crate::takeover_profiles::save_custom_profiles(&runtime.paths.config_dir, &custom)
                    .map_err(|e| e.to_string())?;
            }
            to_value(profile)
        }
        "discover_agents" => {
            let query: String = field(args, &["query"])?;
            to_value(crate::agent_discover::discover_agents(
                &runtime.paths.config_dir,
                &query,
            ))
        }
        "add_discovered_agents" => {
            let items: Vec<AddDiscoveredAgentInput> = field(args, &["items"])?;
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
                    let expanded = crate::takeover_profiles::expand_home(path)
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
                if crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &profile.id)
                    .is_some()
                {
                    continue;
                }
                crate::takeover_profiles::upsert_profile(&runtime.paths.config_dir, profile)
                    .map_err(|e| e.to_string())?;
                if let Some(view) =
                    crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &item.id)
                        .map(|p| {
                            crate::takeover_profiles::profile_view_with_config_dir(
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
            to_value(added)
        }
        "save_takeover_profile" => {
            let input: TakeoverProfileInput = field(args, &["input"])?;
            if input.id.trim().is_empty() || input.name.trim().is_empty() {
                return Err("客户端 ID 与名称不能为空".to_string());
            }
            let id = input.id.trim().to_string();
            if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err("客户端 ID 仅允许英文、数字、- 和 _".to_string());
            }
            let is_seed = crate::takeover_profiles::builtin_profiles()
                .iter()
                .any(|p| p.id.eq_ignore_ascii_case(&id));
            let profile = crate::takeover_profiles::TakeoverProfile {
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
            crate::takeover_profiles::upsert_profile(&runtime.paths.config_dir, profile)
                .map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            to_value(
                crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &id)
                    .map(|p| {
                        crate::takeover_profiles::profile_view_with_config_dir(
                            Some(&runtime.paths.config_dir),
                            &config,
                            &p,
                        )
                    })
                    .ok_or_else(|| format!("保存后未找到模板: {}", id))?,
            )
        }
        "delete_takeover_profile" => {
            let profile_id: String = field(args, &["profileId", "profile_id"])?;
            let config = runtime.config.read().await.clone();
            if config
                .active_takeover_ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(&profile_id))
            {
                let _ = crate::takeover_profiles::release_profile(
                    &runtime.paths.config_dir,
                    &config,
                    &profile_id,
                );
                {
                    let mut cfg = runtime.config.write().await;
                    crate::commands::remove_active_takeover_id(&mut cfg, &profile_id);
                }
                let _ = runtime.persist().await;
            }
            to_value(
                crate::takeover_profiles::delete_profile(&runtime.paths.config_dir, &profile_id)
                    .map_err(|e| e.to_string())?,
            )
        }
        "set_takeover_settings_path" => {
            let profile_id: String = field(args, &["profileId", "profile_id"])?;
            let path: Option<String> = field_opt(args, &["path"])?;
            crate::takeover_profiles::set_settings_path(
                &runtime.paths.config_dir,
                &profile_id,
                path,
            )
            .map_err(|e| e.to_string())?;
            let config = runtime.config.read().await.clone();
            to_value(
                crate::takeover_profiles::find_profile(&runtime.paths.config_dir, &profile_id)
                    .map(|p| {
                        crate::takeover_profiles::profile_view_with_config_dir(
                            Some(&runtime.paths.config_dir),
                            &config,
                            &p,
                        )
                    })
                    .ok_or_else(|| format!("未找到模板：{}", profile_id))?,
            )
        }

        other => Err(format!("WebUI 不支持命令: {}", other)),
    }
}
