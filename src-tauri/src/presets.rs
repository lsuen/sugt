//! 配置预设（Config Presets）领域模块。
//!
//! 用途：提供「稳定工作 / 省钱省流 / 极速响应」三档内置配置预设，并支持用户把当前
//!   配置快照保存为自定义预设、应用、删除、导入与导出。
//! 适用项目：SUGT（本地 AI 网关与桌面精灵）。
//! 创建日期：2026-11-22。
//! 核心职责：
//!   1. 定义 `PresetSettings` / `ConfigPreset` 数据结构与内置三档（代码生成，不落盘）；
//!   2. 以 `config_dir/config-presets.json` 原子持久化自定义预设（tmp + rename）；
//!   3. 提供 6 个 `*_core` 业务函数，供桌面 Tauri 命令与 WebUI dispatch 共用（单一真相源）；
//!   4. 应用预设时只覆盖 `PresetSettings` 声明的字段，其余配置保持原样；
//!   5. 导出前断言不含任何密钥类字段；日志只记录模块/动作/关键参数/结果，绝不打印密钥。

use crate::{commands::AppRuntime, model::AppConfig};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const PRESETS_FILE: &str = "config-presets.json";
/// 内置档由代码生成，`updatedAt` 固定，避免每次列出都变化。
const BUILTIN_UPDATED_AT: &str = "2026-01-01T00:00:00Z";
const BUILTIN_IDS: [&str; 3] = ["builtin-work", "builtin-save", "builtin-turbo"];
/// AGENTS.md：上游韧性默认 blank = 8 min / 60 s => 480000ms / 60000ms。
const DEFAULT_RETRY_BUDGET_MS: u64 = 480_000;
const DEFAULT_RETRY_SINGLE_MS: u64 = 60_000;
/// 导入时对重试毫秒数的合理上限（24 小时），防止误填超大值。
const MAX_RETRY_MS: u64 = 86_400_000;
/// 预设名最大长度（字符数）。
const MAX_PRESET_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSettings {
    /// None = 切换时不改模型（保留当前 activeProviderId）。
    pub active_provider_id: Option<String>,
    pub failover_enabled: bool,
    pub upstream_retry_budget_ms: Option<u64>,
    pub upstream_retry_single_ms: Option<u64>,
    pub experimental_free_idle_probe_enabled: bool,
    pub experimental_free_auto_rotate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreset {
    /// 内置：builtin-work / builtin-save / builtin-turbo；自定义：custom-<毫秒时间戳>。
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub settings: PresetSettings,
    /// RFC3339 时间戳。
    pub updated_at: String,
}

/// 内置三档。参数来源（非硬绑假数据）：
/// - `failoverEnabled` / `activeProviderId` 取自 `AppConfig::default()`；
/// - `upstreamRetryBudgetMs` / `upstreamRetrySingleMs` 的 480000 / 60000 来自
///   AGENTS.md「blank = defaults (8 min / 60 s)」的上游韧性默认值。
/// 三档的 `activeProviderId` 均为 None，表示应用时不动用户当前选择的模型。
pub fn builtin_presets() -> Vec<ConfigPreset> {
    let base = AppConfig::default();
    let make = |id: &str, name: &str, failover: bool, idle_probe: bool, auto_rotate: bool| {
        ConfigPreset {
            id: id.to_string(),
            name: name.to_string(),
            builtin: true,
            settings: PresetSettings {
                active_provider_id: base.active_provider_id.clone(),
                failover_enabled: failover,
                upstream_retry_budget_ms: Some(DEFAULT_RETRY_BUDGET_MS),
                upstream_retry_single_ms: Some(DEFAULT_RETRY_SINGLE_MS),
                experimental_free_idle_probe_enabled: idle_probe,
                experimental_free_auto_rotate: auto_rotate,
            },
            updated_at: BUILTIN_UPDATED_AT.to_string(),
        }
    };
    vec![
        // 稳定工作：保留默认 failover，不启用实验性免费通道优化。
        make("builtin-work", "稳定工作", base.failover_enabled, false, false),
        // 省钱省流：开启免费通道闲时测速并按速度自动轮换。
        make("builtin-save", "省钱省流", base.failover_enabled, true, true),
        // 极速响应：关闭 failover 与实验性优化，减少等待。
        make("builtin-turbo", "极速响应", false, false, false),
    ]
}

fn is_builtin_id(id: &str) -> bool {
    BUILTIN_IDS.contains(&id)
}

fn presets_path(config_dir: &Path) -> PathBuf {
    config_dir.join(PRESETS_FILE)
}

/// 读取自定义预设。文件不存在/为空返回空列表；格式损坏返回明确错误（不吞异常）。
/// 磁盘内容不得伪造内置档、也不得通过同 id 覆盖内置档（模块解耦的安全边界）。
fn load_custom_presets(config_dir: &Path) -> Result<Vec<ConfigPreset>> {
    let path = presets_path(config_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("无法读取配置预设文件 {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut presets: Vec<ConfigPreset> = serde_json::from_str(&raw)
        .with_context(|| format!("配置预设文件格式错误 {}", path.display()))?;
    presets.retain(|p| !is_builtin_id(&p.id));
    for preset in &mut presets {
        preset.builtin = false;
    }
    // 同 id 只保留首个，避免磁盘被手工篡改后出现重复项。
    let mut seen = HashSet::new();
    presets.retain(|p| seen.insert(p.id.clone()));
    Ok(presets)
}

/// 原子写入自定义预设：先写 tmp 再 rename，避免网关/前端读到半截 JSON（复用 config.rs 思路）。
fn save_custom_presets(config_dir: &Path, presets: &[ConfigPreset]) -> Result<()> {
    fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let path = presets_path(config_dir);
    let serialized = serde_json::to_string_pretty(presets).context("配置预设序列化失败")?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serialized)
        .with_context(|| format!("无法写入配置预设临时文件 {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("无法重命名配置预设文件 {}", path.display()))
}

/// 内置三档在前，自定义预设在后。
pub fn all_presets(config_dir: &Path) -> Result<Vec<ConfigPreset>> {
    let mut presets = builtin_presets();
    presets.extend(load_custom_presets(config_dir)?);
    Ok(presets)
}

fn find_preset(config_dir: &Path, id: &str) -> Result<ConfigPreset, String> {
    let presets = all_presets(config_dir).map_err(|e| e.to_string())?;
    presets
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("未找到配置预设：{id}"))
}

/// 生成 custom-<毫秒时间戳>，并确保不与已有预设撞 id（同毫秒连续保存也安全）。
fn next_custom_id(existing: &[ConfigPreset]) -> String {
    let mut millis = Utc::now().timestamp_millis();
    loop {
        let id = format!("custom-{millis}");
        if !existing.iter().any(|p| p.id == id) {
            return id;
        }
        millis += 1;
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("预设名称不能为空".to_string());
    }
    if trimmed.chars().count() > MAX_PRESET_NAME_CHARS {
        return Err(format!("预设名称过长（最多 {MAX_PRESET_NAME_CHARS} 字）"));
    }
    Ok(trimmed.to_string())
}

/// 从运行时配置抽取可被预设覆盖的字段快照。
fn snapshot_from_config(config: &AppConfig) -> PresetSettings {
    PresetSettings {
        active_provider_id: config.active_provider_id.clone(),
        failover_enabled: config.failover_enabled,
        upstream_retry_budget_ms: config.upstream_retry_budget_ms,
        upstream_retry_single_ms: config.upstream_retry_single_ms,
        experimental_free_idle_probe_enabled: config.experimental_free_idle_probe_enabled,
        experimental_free_auto_rotate: config.experimental_free_auto_rotate,
    }
}

// ============================================================================
// 6 个 core 函数：桌面命令与 WebUI dispatch 共用，避免两份业务逻辑。
// 每个函数统一在 wrapper 里记录「模块/动作/关键参数/结果」，且不打印预设内容与密钥。
// ============================================================================

pub fn list_config_presets_core(config_dir: &Path) -> Result<Vec<ConfigPreset>, String> {
    let result = all_presets(config_dir);
    match &result {
        Ok(list) => tracing::info!(
            module = "presets",
            action = "list_config_presets",
            count = list.len(),
            result = "ok",
            "list config presets"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "list_config_presets",
            result = "error",
            "list config presets failed"
        ),
    }
    result.map_err(|e| e.to_string())
}

async fn apply_config_preset_inner(runtime: &AppRuntime, id: &str) -> Result<ConfigPreset, String> {
    let preset = find_preset(&runtime.paths.config_dir, id)?;
    // 安全边界：预设若指定了模型渠道，必须真实存在于 providers，否则拒绝应用。
    if let Some(provider_id) = preset.settings.active_provider_id.as_ref() {
        let exists = {
            let config = runtime.config.read().await;
            config.providers.iter().any(|p| &p.id == provider_id)
        };
        if !exists {
            return Err(format!("预设指定的模型渠道不存在：{provider_id}"));
        }
    }
    {
        let mut config = runtime.config.write().await;
        let s = &preset.settings;
        // 只覆盖 PresetSettings 声明的字段；host/port/providers/日志等其余配置原样保留。
        // activeProviderId=None 表示"切换时不改模型"，此时不动 config.active_provider_id。
        if let Some(provider_id) = s.active_provider_id.as_ref() {
            config.active_provider_id = Some(provider_id.clone());
        }
        config.failover_enabled = s.failover_enabled;
        config.upstream_retry_budget_ms = s.upstream_retry_budget_ms;
        config.upstream_retry_single_ms = s.upstream_retry_single_ms;
        config.experimental_free_idle_probe_enabled = s.experimental_free_idle_probe_enabled;
        config.experimental_free_auto_rotate = s.experimental_free_auto_rotate;
    }
    // persist 会写盘并热更新网关内存配置。这些字段不改变绑定地址/端口，无需重启监听。
    runtime.persist().await.map_err(|e| e.to_string())?;
    Ok(preset)
}

pub async fn apply_config_preset_core(
    runtime: &AppRuntime,
    id: &str,
) -> Result<ConfigPreset, String> {
    let result = apply_config_preset_inner(runtime, id).await;
    match &result {
        Ok(_) => tracing::info!(
            module = "presets",
            action = "apply_config_preset",
            preset_id = %id,
            result = "ok",
            "apply config preset"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "apply_config_preset",
            preset_id = %id,
            result = "error",
            "apply config preset failed"
        ),
    }
    result
}

async fn save_config_preset_inner(runtime: &AppRuntime, name: &str) -> Result<ConfigPreset, String> {
    let name = validate_name(name)?;
    let settings = {
        let config = runtime.config.read().await;
        snapshot_from_config(&config)
    };
    let mut customs = load_custom_presets(&runtime.paths.config_dir).map_err(|e| e.to_string())?;
    // 内置档名称禁止被自定义预设占用：避免出现与内置档同名、却行为不同的"影子档"。
    if builtin_presets()
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("预设名与内置档冲突：{name}"));
    }
    // 同名自定义档 = 覆盖（产品明确要求的功能）：保留原 id，刷新快照与时间戳。
    // 走与新建相同的 save_custom_presets 原子写，不引入第二条持久化路径。
    let preset = if let Some(existing) = customs
        .iter_mut()
        .find(|p| p.name.trim().eq_ignore_ascii_case(&name))
    {
        existing.settings = settings;
        existing.updated_at = now_rfc3339();
        existing.clone()
    } else {
        let created = ConfigPreset {
            id: next_custom_id(&customs),
            name,
            builtin: false,
            settings,
            updated_at: now_rfc3339(),
        };
        customs.push(created.clone());
        created
    };
    save_custom_presets(&runtime.paths.config_dir, &customs).map_err(|e| e.to_string())?;
    Ok(preset)
}

pub async fn save_config_preset_core(
    runtime: &AppRuntime,
    name: &str,
) -> Result<ConfigPreset, String> {
    let result = save_config_preset_inner(runtime, name).await;
    match &result {
        Ok(p) => tracing::info!(
            module = "presets",
            action = "save_config_preset",
            preset_name = %p.name,
            result = "ok",
            "save config preset"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "save_config_preset",
            result = "error",
            "save config preset failed"
        ),
    }
    result
}

fn delete_config_preset_inner(config_dir: &Path, id: &str) -> Result<(), String> {
    if is_builtin_id(id) {
        return Err(format!("内置预设不可删除：{id}"));
    }
    let mut customs = load_custom_presets(config_dir).map_err(|e| e.to_string())?;
    let before = customs.len();
    customs.retain(|p| p.id != id);
    if customs.len() == before {
        return Err(format!("未找到配置预设：{id}"));
    }
    save_custom_presets(config_dir, &customs).map_err(|e| e.to_string())
}

pub fn delete_config_preset_core(config_dir: &Path, id: &str) -> Result<(), String> {
    let result = delete_config_preset_inner(config_dir, id);
    match &result {
        Ok(_) => tracing::info!(
            module = "presets",
            action = "delete_config_preset",
            preset_id = %id,
            result = "ok",
            "delete config preset"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "delete_config_preset",
            preset_id = %id,
            result = "error",
            "delete config preset failed"
        ),
    }
    result
}

/// 导出前的安全闸门。PresetSettings 本身不含任何密钥字段；这里再做一次字符串级断言，
/// 防止未来误往预设里加 apiKey/baseUrl 等字段时悄悄泄漏鉴权信息。
fn assert_no_sensitive_fields(json: &str) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &[
        "api_key",
        "apikey",
        "base_url",
        "baseurl",
        "authorization",
        "secret",
        "password",
        "access_token",
        "refresh_token",
        "client_key",
    ];
    let lower = json.to_ascii_lowercase();
    for marker in FORBIDDEN {
        if lower.contains(marker) {
            return Err(format!("导出内容疑似含敏感字段（{marker}），已拒绝导出"));
        }
    }
    Ok(())
}

fn export_config_preset_inner(config_dir: &Path, id: &str) -> Result<String, String> {
    let preset = find_preset(config_dir, id)?;
    let json = serde_json::to_string_pretty(&preset).map_err(|e| e.to_string())?;
    assert_no_sensitive_fields(&json)?;
    Ok(json)
}

pub fn export_config_preset_core(config_dir: &Path, id: &str) -> Result<String, String> {
    let result = export_config_preset_inner(config_dir, id);
    match &result {
        Ok(_) => tracing::info!(
            module = "presets",
            action = "export_config_preset",
            preset_id = %id,
            result = "ok",
            "export config preset"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "export_config_preset",
            preset_id = %id,
            result = "error",
            "export config preset failed"
        ),
    }
    result
}

/// 兼容导出格式的可选字段：导入时一律忽略并重新生成，防止覆盖内置档或伪造时间戳。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PresetImport {
    name: String,
    settings: PresetSettings,
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    builtin: Option<bool>,
    #[allow(dead_code)]
    updated_at: Option<String>,
}

fn validate_import(import: &PresetImport) -> Result<String, String> {
    let name = validate_name(&import.name)?;
    for (label, value) in [
        ("上游重试总预算", import.settings.upstream_retry_budget_ms),
        ("单次等待上限", import.settings.upstream_retry_single_ms),
    ] {
        if let Some(v) = value {
            if v == 0 || v > MAX_RETRY_MS {
                return Err(format!("{label} 不在合理范围（1..={MAX_RETRY_MS} 毫秒）"));
            }
        }
    }
    if let Some(provider_id) = import.settings.active_provider_id.as_ref() {
        if provider_id.trim().is_empty() {
            return Err("模型渠道 ID 不能为空白".to_string());
        }
    }
    Ok(name)
}

fn import_config_preset_inner(config_dir: &Path, json: &str) -> Result<ConfigPreset, String> {
    // 不打印 serde 错误详情：错误内容可能回显导入 JSON 里的密钥值。
    let import: PresetImport = serde_json::from_str(json)
        .map_err(|_| "预设 JSON 格式错误或缺少必填字段（name / settings）".to_string())?;
    let name = validate_import(&import)?;
    let mut customs = load_custom_presets(config_dir).map_err(|e| e.to_string())?;
    if customs
        .iter()
        .any(|p| p.name.trim().eq_ignore_ascii_case(&name))
    {
        return Err(format!("已存在同名预设：{name}"));
    }
    let mut settings = import.settings;
    settings.active_provider_id = settings.active_provider_id.map(|s| s.trim().to_string());
    // 强制生成 custom-<timestamp>，绝不采信输入里的 id（防止覆盖内置档）。
    let preset = ConfigPreset {
        id: next_custom_id(&customs),
        name,
        builtin: false,
        settings,
        updated_at: now_rfc3339(),
    };
    customs.push(preset.clone());
    save_custom_presets(config_dir, &customs).map_err(|e| e.to_string())?;
    Ok(preset)
}

pub fn import_config_preset_core(config_dir: &Path, json: &str) -> Result<ConfigPreset, String> {
    let result = import_config_preset_inner(config_dir, json);
    match &result {
        Ok(p) => tracing::info!(
            module = "presets",
            action = "import_config_preset",
            preset_name = %p.name,
            result = "ok",
            "import config preset"
        ),
        Err(_) => tracing::warn!(
            module = "presets",
            action = "import_config_preset",
            result = "error",
            "import config preset failed"
        ),
    }
    result
}
