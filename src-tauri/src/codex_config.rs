use crate::model::AppConfig;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// SUGT 在 Codex config.toml 里托管的 provider id（可安全增删，不碰用户其它 provider）。
pub const SUGT_PROVIDER_ID: &str = "sugt";

/// 接管前的原文件快照；释放时按它还原，而不是靠猜字段。
const STATE_FILE: &str = "sugt-takeover-state.json";
const BACKUP_DIR: &str = "sugt-backup";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TakeoverState {
    /// 接管前 config.toml 全文；None 表示原本不存在
    config_before: Option<String>,
    /// SUGT 写入后的全文，用于判断用户是否自行改过
    config_after: String,
    auth_before: Option<String>,
    auth_after: String,
    /// 顶层 web_search 是否由 SUGT 注入（用户原本没写这个键）
    #[serde(default)]
    web_search_injected: bool,
}

fn state_path(codex_dir: &Path) -> PathBuf {
    codex_dir.join(STATE_FILE)
}

fn read_state(codex_dir: &Path) -> Option<TakeoverState> {
    let raw = std::fs::read_to_string(state_path(codex_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// TOML 解析失败必须报错，绝不退化成空表后覆盖用户文件。
fn parse_table(raw: &str, path: &Path) -> Result<toml::Table> {
    toml::from_str(raw).with_context(|| {
        format!(
            "{} 不是合法 TOML，已中止写入以免覆盖原文件",
            path.display()
        )
    })
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
///
/// Windows 的 `rename` 不能覆盖已存在的目标，旧实现先删原文件再搬，
/// 一旦 rename 失败（文件被占用 / 权限）原文件就永久丢失。这里所有平台
/// 统一先把原文件搬到带 pid 的备份位，替换失败就从备份位还原。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let tmp = path.with_file_name(format!("{name}.sugt-tmp"));
    std::fs::write(&tmp, content).with_context(|| format!("无法写入临时文件 {}", tmp.display()))?;
    match replace_with_backup(path, &tmp) {
        Ok(()) => Ok(()),
        Err(err) => {
            // 失败绝不留下半成品临时文件。
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

/// 把 tmp 替换成目标文件；失败时把原文件从备份位还原。
fn replace_with_backup(path: &Path, tmp: &Path) -> Result<()> {
    if path.is_dir() {
        anyhow::bail!("目标路径是目录，拒绝替换 {}", path.display());
    }
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let backup = path.with_file_name(format!("{name}.sugt-{}.backup", std::process::id()));
    if path.exists() {
        std::fs::rename(path, &backup).with_context(|| format!("无法备份 {}", path.display()))?;
    }
    if let Err(err) = std::fs::rename(tmp, path) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, path);
        }
        return Err(err).with_context(|| format!("无法替换 {}", path.display()));
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

/// 首次接管前把用户原始文件留一份带时间戳的备份，快照丢失时仍可人工恢复。
fn backup_once(codex_dir: &Path, config_path: &Path, auth_path: &Path) -> Result<()> {
    let dir = codex_dir.join(BACKUP_DIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("无法创建备份目录 {}", dir.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if config_path.is_file() {
        std::fs::copy(config_path, dir.join(format!("config-{stamp}.toml")))?;
    }
    if auth_path.is_file() {
        std::fs::copy(auth_path, dir.join(format!("auth-{stamp}.json")))?;
    }
    Ok(())
}

/// 给客户端看到的模型名加 SUGT 前缀，避免同名官方 slug 继承 code_mode_only /
/// use_responses_lite 工具协议（CC Switch #8676、Codex #31870 均指向这个根因）。
/// 网关侧本来就用 upstream_model 覆盖请求里的 model，所以别名不影响上游路由。
/// 各 Codex 版本对未知 slug 的处理不完全一致，因此保留关闭开关作为回退口。
fn codex_model_alias_enabled() -> bool {
    !std::env::var("SUGT_CODEX_MODEL_ALIAS")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "off" | "0" | "false" | "disabled"
            )
        })
        .unwrap_or(false)
}

fn codex_model_alias(public_model: &str, enabled: bool) -> String {
    if !enabled || public_model.starts_with("sugt-") {
        return public_model.to_string();
    }
    format!("sugt-{}", public_model)
}

/// 写入/合并 Codex 官方配置（新版 CLI 以 config.toml + model_providers 为准）。
///
/// 写前备份并快照原文件；连续调用不会覆盖最初的快照；解析失败直接中止。
pub fn apply_codex_config(config: &AppConfig) -> Result<PathBuf> {
    let codex_dir = codex_home_dir()?;
    std::fs::create_dir_all(&codex_dir)
        .with_context(|| format!("无法创建 Codex 目录 {}", codex_dir.display()))?;
    let config_path = codex_dir.join("config.toml");
    let auth_path = codex_dir.join("auth.json");
    let openai_base = format!("{}/v1", config.listen_url());
    let public_model = config
        .providers
        .iter()
        .find(|p| config.active_provider_id.as_ref() == Some(&p.id))
        .map(|p| p.public_model_id())
        .unwrap_or_else(|| "gpt-4".to_string());

    let before_raw = if config_path.is_file() {
        Some(std::fs::read_to_string(&config_path)?)
    } else {
        None
    };
    let mut table = match &before_raw {
        Some(raw) => parse_table(raw, &config_path)?,
        None => toml::Table::new(),
    };

    // 新版 Codex：model_provider + wire_api=responses（与 cc-switch 同类做法）
    table.insert(
        "model_provider".to_string(),
        toml::Value::String(SUGT_PROVIDER_ID.to_string()),
    );
    // 别名只影响客户端看到的 slug；网关始终用 upstream_model 覆盖请求里的 model。
    table.insert(
        "model".to_string(),
        toml::Value::String(codex_model_alias(
            &public_model,
            codex_model_alias_enabled(),
        )),
    );
    table.insert(
        "openai_base_url".to_string(),
        toml::Value::String(openai_base.clone()),
    );
    table.insert(
        "preferred_auth_method".to_string(),
        toml::Value::String("apikey".to_string()),
    );
    table.insert(
        "cli_auth_credentials_store".to_string(),
        toml::Value::String("file".to_string()),
    );
    // hosted web_search 的 schema 会打到不支持它的本地网关；用户没配置时显式关掉。
    let web_search_injected = !table.contains_key("web_search");
    if web_search_injected {
        table.insert(
            "web_search".to_string(),
            toml::Value::String("disabled".to_string()),
        );
    }

    let mut provider = toml::Table::new();
    provider.insert("name".to_string(), toml::Value::String("SUGT".to_string()));
    provider.insert("base_url".to_string(), toml::Value::String(openai_base));
    provider.insert(
        "wire_api".to_string(),
        toml::Value::String("responses".to_string()),
    );
    // 第三方 provider 不要求 OpenAI 官方认证；不声明时客户端会按官方账号路径
    // 申请 hosted 工具 / MCP 能力，在自定义网关上一律变成 unsupported call。
    provider.insert(
        "requires_openai_auth".to_string(),
        toml::Value::Boolean(false),
    );
    // Codex CLI 0.162+ 默认把远程压缩交给服务端（compaction_trigger item），
    // 本地网关解析不了会 400 / 中断任务；显式声明改用客户端本地压缩。
    let mut capabilities = toml::Table::new();
    capabilities.insert(
        "remote_compaction".to_string(),
        toml::Value::String("unsupported".to_string()),
    );
    capabilities.insert(
        "external_web_access".to_string(),
        toml::Value::Boolean(false),
    );
    provider.insert("capabilities".to_string(), toml::Value::Table(capabilities));
    let mut providers = match table.get("model_providers").and_then(|v| v.as_table()) {
        Some(existing) => existing.clone(),
        None => toml::Table::new(),
    };
    providers.insert(SUGT_PROVIDER_ID.to_string(), toml::Value::Table(provider));
    table.insert(
        "model_providers".to_string(),
        toml::Value::Table(providers),
    );

    let serialized = toml::to_string_pretty(&table).context("Codex config 序列化失败")?;
    let auth_before = if auth_path.is_file() {
        Some(std::fs::read_to_string(&auth_path)?)
    } else {
        None
    };
    // 只覆盖认证方式与本地网关 Key，保留原有 ChatGPT 登录态字段
    // （tokens / account_id / plan 等）；整体替换会让客户端丢失账号侧能力。
    let auth_after = codex_auth_content(
        &crate::clients::client_api_key_for_config(config),
        auth_before.as_deref(),
    )?;

    let previous = read_state(&codex_dir);
    if previous.is_none() {
        backup_once(&codex_dir, &config_path, &auth_path)?;
    }
    let state = TakeoverState {
        config_before: previous
            .as_ref()
            .map(|s| s.config_before.clone())
            .unwrap_or_else(|| before_raw.clone()),
        config_after: serialized.clone(),
        auth_before: previous
            .as_ref()
            .map(|s| s.auth_before.clone())
            .unwrap_or_else(|| auth_before.clone()),
        auth_after: auth_after.clone(),
        web_search_injected: previous
            .as_ref()
            .map(|s| s.web_search_injected)
            .unwrap_or(web_search_injected),
    };

    atomic_write(&config_path, &serialized)?;
    atomic_write(&auth_path, &auth_after)?;
    atomic_write(
        &state_path(&codex_dir),
        &serde_json::to_string_pretty(&state)?,
    )?;
    Ok(config_path)
}

/// 取消 Codex 接管：优先按快照逐字节还原；用户改过文件时只定点清理 SUGT 字段。
pub fn clear_sugt_codex_overrides(config: &AppConfig) -> Result<()> {
    let codex_dir = codex_home_dir()?;
    let config_path = codex_dir.join("config.toml");
    let auth_path = codex_dir.join("auth.json");

    if let Some(state) = read_state(&codex_dir) {
        let current_config = if config_path.is_file() {
            Some(std::fs::read_to_string(&config_path)?)
        } else {
            None
        };
        if current_config.as_deref() == Some(state.config_after.as_str()) {
            // 用户没有自行改动，整文件还原到接管前状态最可靠。
            match &state.config_before {
                Some(raw) => atomic_write(&config_path, raw)?,
                None => remove_file_if_exists(&config_path)?,
            }
        } else {
            // 快照里记录了接管前的原值，交给 prune 精确还原，不靠字符串猜。
            prune_sugt_fields(&config_path, config, Some(&state), None)?;
        }

        let current_auth = if auth_path.is_file() {
            Some(std::fs::read_to_string(&auth_path)?)
        } else {
            None
        };
        if current_auth.as_deref() == Some(state.auth_after.as_str()) {
            match &state.auth_before {
                Some(raw) => atomic_write(&auth_path, raw)?,
                None => remove_file_if_exists(&auth_path)?,
            }
        } else {
            // 用户自行改过 auth.json：只清确定由 SUGT 注入的两个键，
            // tokens / account_id 等登录态字段一律保留。
            prune_sugt_auth_fields(&auth_path, config)?;
        }
        remove_file_if_exists(&state_path(&codex_dir))?;
        return Ok(());
    }

    // 兼容旧版本（没有快照）：1.1.7 根本不写快照，且无条件覆盖三个标量
    // 与整个 auth.json。识别到该形态时按已知写入值清理，其它情况仍保守不动。
    let legacy_base = legacy_1_1_7_gateway_base(&config_path);
    if legacy_base.is_some() {
        tracing::warn!("检测到 1.1.7 遗留 Codex 接管态，按已知写入值清理残留字段");
    }
    prune_sugt_fields(&config_path, config, None, legacy_base.as_deref())?;
    prune_sugt_auth_fields(&auth_path, config)?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("无法删除 {}", path.display()))?;
    }
    Ok(())
}

/// 识别 1.1.7 已发布版留下的接管态，返回当时写入的网关地址。
///
/// 1.1.7 的特征是：无快照文件、model 没有 `sugt-` 别名前缀、provider 不带
/// `requires_openai_auth` / `capabilities`、顶层没有注入的 `web_search`。
/// 这些条件同时成立时，残留的三个标量与 auth.json 都确定是 SUGT 写入的，
/// 而不是用户原值；只在此时才按已知值清理，其它形态一律保守保留。
fn legacy_1_1_7_gateway_base(config_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(config_path).ok()?;
    let table: toml::Table = toml::from_str(&raw).ok()?;
    if table.get("model_provider").and_then(|value| value.as_str()) != Some(SUGT_PROVIDER_ID) {
        return None;
    }
    let model = table.get("model")?.as_str()?;
    if model.is_empty() || model.starts_with("sugt-") {
        return None;
    }
    if table.contains_key("web_search") {
        return None;
    }
    let provider = table
        .get("model_providers")?
        .as_table()?
        .get(SUGT_PROVIDER_ID)?
        .as_table()?;
    if provider.get("wire_api")?.as_str()? != "responses" {
        return None;
    }
    if provider.contains_key("requires_openai_auth") || provider.contains_key("capabilities") {
        return None;
    }
    Some(provider.get("base_url")?.as_str()?.to_string())
}

/// 清理 auth.json 里确定由 SUGT 注入的两个键，保留用户其它登录态字段。
///
/// 判定条件是「auth_mode=apikey 且 key 等于本机网关 key 或带 sugt- 前缀」，
/// 用户自己换过的 key 绝不碰；清完若为空对象就删掉文件，让客户端重新登录。
fn prune_sugt_auth_fields(auth_path: &Path, config: &AppConfig) -> Result<()> {
    if !auth_path.is_file() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(auth_path)?;
    let mut value: serde_json::Value = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) if value.is_object() => value,
        _ => return Ok(()), // 非法内容绝不猜测、绝不覆盖
    };
    let object = value.as_object_mut().expect("已确保是 JSON object");
    let auth_mode_is_ours = object.get("auth_mode").and_then(|v| v.as_str()) == Some("apikey");
    let gateway_key = crate::clients::client_api_key_for_config(config);
    let key_is_ours = object
        .get("OPENAI_API_KEY")
        .and_then(|v| v.as_str())
        .is_some_and(|key| key == gateway_key || key.starts_with("sugt-"));
    if !(auth_mode_is_ours && key_is_ours) {
        return Ok(());
    }
    object.remove("OPENAI_API_KEY");
    object.remove("auth_mode");
    if object.is_empty() {
        remove_file_if_exists(auth_path)?;
    } else {
        atomic_write(auth_path, &serde_json::to_string_pretty(&value)?)?;
    }
    Ok(())
}

/// 只移除 SUGT 写入的字段，保留用户其它配置；用 toml_edit 保留注释与原格式。
///
/// 有快照时按 `config_after`（SUGT 写入的值）判断字段是否被用户改过，再按
/// `config_before` 精确还原；无快照时只有识别到 1.1.7 遗留态才按已知值清理，
/// 其它情况宁可残留，也绝不误删用户配置。
fn prune_sugt_fields(
    config_path: &Path,
    config: &AppConfig,
    state: Option<&TakeoverState>,
    legacy_base: Option<&str>,
) -> Result<()> {
    if !config_path.is_file() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(config_path)?;
    let mut doc: toml_edit::Document = raw.parse().with_context(|| {
        format!(
            "{} 不是合法 TOML，已中止写入以免覆盖原文件",
            config_path.display()
        )
    })?;
    let expected = format!("{}/v1", config.listen_url());
    let public_model = config
        .providers
        .iter()
        .find(|provider| config.active_provider_id.as_ref() == Some(&provider.id))
        .map(|provider| provider.public_model_id())
        .unwrap_or_else(|| "gpt-4".to_string());
    let web_search_injected = state.is_some_and(|s| s.web_search_injected);
    let before_doc = state
        .and_then(|s| s.config_before.as_deref())
        .and_then(|text| text.parse::<toml_edit::Document>().ok());
    let after_doc = state.and_then(|s| s.config_after.parse::<toml_edit::Document>().ok());
    let mut changed = false;

    if doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        == Some(SUGT_PROVIDER_ID)
    {
        doc.remove("model_provider");
        changed = true;
    }
    {
        if let Some(providers) = doc
            .get_mut("model_providers")
            .and_then(|v| v.as_table_mut())
        {
            if providers.remove(SUGT_PROVIDER_ID).is_some() {
                changed = true;
            }
        }
    }
    if doc
        .get("model_providers")
        .and_then(|v| v.as_table())
        .is_some_and(|t| t.is_empty())
    {
        doc.remove("model_providers");
    }
    // 只删 SUGT 自己写进去的那个值；不再按 contains("localhost") 误删用户的本地服务。
    // 1.1.7 遗留态额外按 provider 里记录的 base_url 判定，避免网关端口变化后残留。
    let current_base = doc.get("openai_base_url").and_then(|v| v.as_str());
    if current_base == Some(expected.as_str())
        || (legacy_base.is_some() && current_base == legacy_base)
    {
        doc.remove("openai_base_url");
        changed = true;
    }
    // 只删确认由 SUGT 注入的 web_search，且值仍是被注入的值；用户自己改过就保留。
    if web_search_injected
        && doc.get("web_search").and_then(|v| v.as_str()) == Some("disabled")
    {
        doc.remove("web_search");
        changed = true;
    }
    // 三个被无条件覆盖的标量字段：按快照精确还原，而不是按值猜测后删除。
    // 用户接管前就常有这三项（例如 preferred_auth_method = "apikey"），
    // 直接删除会造成用户配置丢失。
    for key in [
        "model",
        "preferred_auth_method",
        "cli_auth_credentials_store",
    ] {
        let current = doc
            .get(key)
            .and_then(|item| item.as_str())
            .map(str::to_string);
        let ours = after_doc
            .as_ref()
            .and_then(|snapshot| snapshot.get(key))
            .and_then(|item| item.as_str())
            .map(str::to_string);
        let Some(ours) = ours else {
            // 没有快照（旧版本）或快照不可解析：只清理确定由 SUGT 写入的值。
            if legacy_base.is_some() {
                // 1.1.7 无条件覆盖了这三个标量，残留的不是用户原值。
                // 仍按值判定：用户在接管期间改过的字段一律保留。
                let cleanup = match key {
                    "model" => current.as_deref().is_some_and(|value| {
                        value.starts_with("sugt-") || value == public_model.as_str()
                    }),
                    "preferred_auth_method" => current.as_deref() == Some("apikey"),
                    "cli_auth_credentials_store" => current.as_deref() == Some("file"),
                    _ => false,
                };
                if cleanup {
                    doc.remove(key);
                    changed = true;
                }
            } else if key == "model"
                && current.is_some_and(|value| value.starts_with("sugt-"))
            {
                // 新版丢失快照：只清确定由 SUGT 生成的模型别名，鉴权两项保守保留。
                doc.remove(key);
                changed = true;
            }
            continue;
        };
        if current.as_deref() != Some(ours.as_str()) {
            // 用户在接管期间改过这个字段，尊重用户改动，既不删除也不回写快照。
            continue;
        }
        match before_doc.as_ref().and_then(|snapshot| snapshot.get(key)) {
            Some(original) => {
                doc.insert(key, original.clone());
            }
            None => {
                // 接管前原本没有这个字段，才真正删除。
                doc.remove(key);
            }
        }
        changed = true;
    }
    if changed {
        atomic_write(config_path, &doc.to_string())?;
    }
    Ok(())
}

pub fn codex_home_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = directories::UserDirs::new()
        .context("无法解析用户目录")?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".codex"))
}

pub fn read_openai_base_url() -> Result<Option<String>> {
    let config_path = codex_home_dir()?.join("config.toml");
    if !config_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&config_path)?;
    let table: toml::Table = parse_table(&raw, &config_path)?;
    // 优先读新版 model_providers.sugt.base_url
    if let Some(base) = table
        .get("model_providers")
        .and_then(|v| v.as_table())
        .and_then(|p| p.get(SUGT_PROVIDER_ID))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
    {
        return Ok(Some(base.to_string()));
    }
    Ok(table
        .get("openai_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexConfigStatus {
    pub config_path: String,
    pub openai_base_url: Option<String>,
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub wire_api: Option<String>,
    pub auth_configured: bool,
    pub matches_gateway: bool,
}

pub fn codex_config_status(config: &AppConfig) -> CodexConfigStatus {
    let codex_dir = codex_home_dir().ok();
    let config_path = codex_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| {
            codex_dir
                .as_ref()
                .map(|d| d.join("config.toml").display().to_string())
                .unwrap_or_else(|| "~/.codex/config.toml".to_string())
        });

    let expected_base = format!("{}/v1", config.listen_url());
    let table = codex_dir.as_ref().and_then(|d| {
        let path = d.join("config.toml");
        if !path.exists() {
            return None;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        toml::from_str::<toml::Table>(&raw).ok()
    });

    let openai_base = read_openai_base_url().ok().flatten();
    let model = table
        .as_ref()
        .and_then(|t| t.get("model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let model_provider = table
        .as_ref()
        .and_then(|t| t.get("model_provider"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let wire_api = table
        .as_ref()
        .and_then(|t| t.get("model_providers"))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get(SUGT_PROVIDER_ID))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get("wire_api"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let auth_configured = codex_dir
        .as_ref()
        .map(|d| d.join("auth.json").exists())
        .unwrap_or(false);

    let base_ok = openai_base.as_deref() == Some(expected_base.as_str());
    let provider_ok = model_provider.as_deref() == Some(SUGT_PROVIDER_ID);
    let wire_ok = wire_api.as_deref() == Some("responses") || wire_api.is_none();
    // 新配置：provider=sugt 且 base 指向网关；旧配置：仅 openai_base_url 匹配也算
    let matches_gateway = base_ok && (provider_ok || model_provider.is_none()) && wire_ok;

    CodexConfigStatus {
        config_path,
        openai_base_url: openai_base,
        model,
        model_provider,
        wire_api,
        auth_configured,
        matches_gateway,
    }
}

/// 生成 auth.json 内容：只覆盖认证方式与本地网关 Key，保留原有登录态字段。
fn codex_auth_content(api_key: &str, existing: Option<&str>) -> Result<String> {
    let mut value = match existing {
        Some(raw) => serde_json::from_str::<serde_json::Value>(raw).with_context(|| {
            "auth.json 不是合法 JSON，已中止写入以免清空 ChatGPT 登录态"
        })?,
        None => serde_json::json!({}),
    };
    if !value.is_object() {
        anyhow::bail!("auth.json 顶层必须是 JSON object，已中止写入以免清空登录态");
    }
    let object = value
        .as_object_mut()
        .expect("已确保是 JSON object");
    object.insert("auth_mode".to_string(), serde_json::json!("apikey"));
    object.insert(
        "OPENAI_API_KEY".to_string(),
        serde_json::json!(api_key),
    );
    Ok(serde_json::to_string_pretty(&value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppConfig, ProviderConfig, ProviderProtocol};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    /// CODEX_HOME 是进程级全局状态，读写它的测试必须互斥，避免并行时出现竞态。
    static CODEX_HOME_LOCK: Mutex<()> = Mutex::new(());

    fn temp_codex_home() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "sugt-codex-test-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config(port: u16) -> AppConfig {
        let mut provider = ProviderConfig::new(
            "test",
            "openai",
            "https://example.com/v1",
            "k",
            "demo-model",
        );
        provider.id = "p1".into();
        provider.protocol = ProviderProtocol::OpenAi;
        AppConfig {
            host: "127.0.0.1".into(),
            port,
            providers: vec![provider],
            active_provider_id: Some("p1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn codex_home_default_under_dot_codex() {
        let _guard = CODEX_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("CODEX_HOME");
        std::env::remove_var("CODEX_HOME");
        let dir = codex_home_dir().unwrap();
        assert!(
            dir.to_string_lossy().ends_with(".codex"),
            "未设置 CODEX_HOME 时应回落到用户目录的 .codex，实际 {dir:?}"
        );
        if let Some(value) = previous {
            std::env::set_var("CODEX_HOME", value);
        }
    }

    /// 所有会写 CODEX_HOME 的测试共用互斥锁，并在结束时恢复环境变量。
    fn with_codex_home<F: FnOnce(&Path)>(body: F) {
        let _guard = CODEX_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("CODEX_HOME");
        let home = temp_codex_home();
        std::env::set_var("CODEX_HOME", &home);
        body(&home);
        match previous {
            Some(value) => std::env::set_var("CODEX_HOME", value),
            None => std::env::remove_var("CODEX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn apply_then_release_restores_original_files_byte_for_byte() {
        with_codex_home(|home| {
            let config = sample_config(18787);
            let config_path = home.join("config.toml");
            let auth_path = home.join("auth.json");
            let original_config = concat!(
                "# 我自己的 Codex 配置\n",
                "model = \"o3\"\n",
                "\n[model_providers.mine]\n",
                "name = \"Mine\"\n",
                "base_url = \"https://mine.example/v1\"\n",
                "wire_api = \"chat\"\n",
            );
            let original_auth = "{\"auth_mode\":\"chatgpt\",\"tokens\":{\"keep\":1}}\n";
            std::fs::write(&config_path, original_config).unwrap();
            std::fs::write(&auth_path, original_auth).unwrap();

            let path = apply_codex_config(&config).unwrap();
            assert_eq!(path, config_path);
            let taken = std::fs::read_to_string(&config_path).unwrap();
            assert!(taken.contains("wire_api = \"responses\""));
            assert!(taken.contains("model_providers.mine"), "用户自己的 provider 不能被删");
            assert!(codex_config_status(&config).matches_gateway);

            clear_sugt_codex_overrides(&config).unwrap();
            assert_eq!(
                std::fs::read_to_string(&config_path).unwrap(),
                original_config,
                "释放后 config.toml 必须逐字节还原"
            );
            assert_eq!(
                std::fs::read_to_string(&auth_path).unwrap(),
                original_auth,
                "释放后 auth.json 必须逐字节还原"
            );
            assert!(!home.join(STATE_FILE).exists(), "释放后应清理快照");
            assert!(home.join(BACKUP_DIR).is_dir(), "首次接管应留下备份");
        });
    }

    #[test]
    fn invalid_toml_is_never_overwritten() {
        with_codex_home(|home| {
            let config = sample_config(18788);
            let path = home.join("config.toml");
            let broken = "this is = not = valid toml\n[[\n";
            std::fs::write(&path, broken).unwrap();

            let err = apply_codex_config(&config).unwrap_err();
            assert!(
                err.to_string().contains("不是合法 TOML"),
                "实际错误：{err}"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
            assert!(!home.join("auth.json").exists(), "失败时不能留下半个写入");
        });
    }

    #[test]
    fn second_apply_keeps_the_original_snapshot() {
        with_codex_home(|home| {
            let config = sample_config(18789);
            let path = home.join("config.toml");
            let original = "model = \"o3\"\n";
            std::fs::write(&path, original).unwrap();
            apply_codex_config(&config).unwrap();
            apply_codex_config(&config).unwrap();
            clear_sugt_codex_overrides(&config).unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                original,
                "连续接管不能把快照覆盖成 SUGT 自己的版本"
            );
        });
    }

    #[test]
    fn release_preserves_edits_made_by_the_user() {
        with_codex_home(|home| {
            let config = sample_config(18790);
            let path = home.join("config.toml");
            std::fs::write(&path, "model = \"o3\"\n").unwrap();
            apply_codex_config(&config).unwrap();

            let mut edited = std::fs::read_to_string(&path).unwrap();
            edited.push_str("\n# 用户手动新增\n");
            std::fs::write(&path, &edited).unwrap();

            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(after.contains("# 用户手动新增"), "用户改动必须保留");
            assert!(!after.contains("model_provider = "), "SUGT 字段仍应清理");
            assert!(!after.contains("[model_providers.sugt]"));
            assert!(
                !after.contains("sugt-"),
                "改过文件的释放路径也要清掉 sugt- 模型别名：{after}"
            );
            assert!(
                !after.contains("preferred_auth_method"),
                "改过文件的释放路径也要清掉 SUGT 写入的鉴权偏好：{after}"
            );
        });
    }

    #[test]
    fn release_restores_original_fields_when_user_edited_config() {
        with_codex_home(|home| {
            let config = sample_config(18795);
            let path = home.join("config.toml");
            // 取自真机 ~/.codex/sugt-takeover-state.json 的 config_before 形态：
            // 接管前用户就已经有 model 与两项鉴权配置。
            let original = concat!(
                "# 用户自己的注释\n",
                "model = \"openrouter/free\"\n",
                "preferred_auth_method = \"apikey\"\n",
                "cli_auth_credentials_store = \"file\"\n",
                "\n[mcp_servers.node_repl]\n",
                "command = \"node\"\n",
            );
            std::fs::write(&path, original).unwrap();
            apply_codex_config(&config).unwrap();

            let mut edited = std::fs::read_to_string(&path).unwrap();
            edited.push_str("\n# 接管期间用户手动新增\n");
            std::fs::write(&path, &edited).unwrap();

            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(
                after.contains("# 接管期间用户手动新增"),
                "用户改动必须保留：{after}"
            );
            assert!(
                !after.contains("model_provider = "),
                "SUGT 字段应清理：{after}"
            );
            assert!(
                !after.contains("[model_providers.sugt]"),
                "SUGT provider 应清理：{after}"
            );
            assert!(
                !after.contains("openai_base_url"),
                "SUGT base_url 应清理：{after}"
            );
            assert!(
                !after.contains("web_search"),
                "SUGT 注入的 web_search 应清理：{after}"
            );
            assert!(
                after.contains("model = \"openrouter/free\""),
                "用户接管前原有的 model 必须按快照还原，不能删掉：{after}"
            );
            assert!(
                after.contains("preferred_auth_method = \"apikey\""),
                "用户接管前原有的鉴权偏好必须按快照还原：{after}"
            );
            assert!(
                after.contains("cli_auth_credentials_store = \"file\""),
                "用户接管前原有的凭据存储必须按快照还原：{after}"
            );
            assert!(
                after.contains("[mcp_servers.node_repl]"),
                "MCP 配置必须保留：{after}"
            );
        });
    }

    #[test]
    fn release_keeps_model_edited_by_user_after_takeover() {
        with_codex_home(|home| {
            let config = sample_config(18796);
            let path = home.join("config.toml");
            std::fs::write(&path, "model = \"o3\"\n").unwrap();
            apply_codex_config(&config).unwrap();

            // 用户在接管期间自己改了 model：释放时不能回写快照的 o3，要尊重用户改动。
            let mut edited = std::fs::read_to_string(&path).unwrap();
            edited.push_str("\n# 用户改动\n");
            let edited = edited.replacen("sugt-", "my-", 1);
            std::fs::write(&path, &edited).unwrap();

            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(
                after.contains("my-"),
                "用户在接管期间改的 model 必须保留：{after}"
            );
            assert!(
                !after.contains("model = \"o3\""),
                "不能把用户改动回退成快照值：{after}"
            );
        });
    }

    #[test]
    fn legacy_release_without_snapshot_keeps_user_auth_preferences() {
        with_codex_home(|home| {
            let config = sample_config(18797);
            let path = home.join("config.toml");
            std::fs::write(
                &path,
                "model = \"o3\"\npreferred_auth_method = \"apikey\"\ncli_auth_credentials_store = \"file\"\n",
            )
            .unwrap();
            apply_codex_config(&config).unwrap();
            // 模拟旧版本：没有快照文件时无法确定字段归属，只能保守不动鉴权项。
            std::fs::remove_file(home.join(STATE_FILE)).unwrap();

            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(
                after.contains("preferred_auth_method = \"apikey\""),
                "无快照时宁可残留也不能误删用户配置：{after}"
            );
            assert!(
                after.contains("cli_auth_credentials_store = \"file\""),
                "无快照时宁可残留也不能误删用户配置：{after}"
            );
        });
    }

    #[test]
    fn invalid_auth_json_aborts_without_wiping_login() {
        with_codex_home(|home| {
            let config = sample_config(18794);
            let auth_path = home.join("auth.json");
            let broken = "{\"auth_mode\":\"chatgpt\",";
            std::fs::write(&auth_path, broken).unwrap();
            let err = apply_codex_config(&config).unwrap_err();
            assert!(
                err.to_string().contains("不是合法 JSON"),
                "实际错误：{err}"
            );
            assert_eq!(
                std::fs::read_to_string(&auth_path).unwrap(),
                broken,
                "非法 auth.json 绝不能被清空重写"
            );
        });
    }

    #[test]
    fn model_alias_is_idempotent_and_switchable() {
        assert_eq!(codex_model_alias("gpt-5.6-sol", true), "sugt-gpt-5.6-sol");
        assert_eq!(
            codex_model_alias("gpt-5.6-sol", false),
            "gpt-5.6-sol",
            "开关关闭时必须保持上游原名"
        );
        assert_eq!(
            codex_model_alias("sugt-gpt-5.6-sol", true),
            "sugt-gpt-5.6-sol",
            "重复接管不能叠前缀"
        );
    }

    #[test]
    fn takeover_declares_third_party_capabilities_and_keeps_login_fields() {
        with_codex_home(|home| {
            let config = sample_config(18792);
            let config_path = home.join("config.toml");
            let auth_path = home.join("auth.json");
            std::fs::write(
                &config_path,
                "[mcp_servers.file]\ncommand = \"npx\"\nargs = [\"-y\", \"mcp-server-filesystem\"]\n",
            )
            .unwrap();
            std::fs::write(
                &auth_path,
                "{\"auth_mode\":\"chatgpt\",\"tokens\":{\"id_token\":\"keep\"},\"account_id\":\"acct_1\"}",
            )
            .unwrap();

            apply_codex_config(&config).unwrap();
            let taken = std::fs::read_to_string(&config_path).unwrap();
            let table: toml::Table = toml::from_str(&taken).unwrap();
            let provider = table["model_providers"]["sugt"].as_table().unwrap();
            assert_eq!(provider["wire_api"].as_str(), Some("responses"));
            assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
            let capabilities = provider["capabilities"].as_table().unwrap();
            assert_eq!(
                capabilities["remote_compaction"].as_str(),
                Some("unsupported")
            );
            assert_eq!(capabilities["external_web_access"].as_bool(), Some(false));
            assert_eq!(table["web_search"].as_str(), Some("disabled"));
            assert!(
                table["model"]
                    .as_str()
                    .is_some_and(|model| model.starts_with("sugt-")),
                "客户端看到的模型名必须避开官方 slug 以规避 code_mode_only"
            );
            assert!(
                table.get("mcp_servers").is_some(),
                "接管绝不能碰用户自己的 MCP server 配置"
            );

            let auth: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
            assert_eq!(auth["auth_mode"], "apikey");
            assert_eq!(auth["tokens"]["id_token"], "keep", "原登录态字段必须保留");
            assert_eq!(auth["account_id"], "acct_1");
            assert!(auth["OPENAI_API_KEY"].as_str().is_some());
        });
    }

    #[test]
    fn takeover_respects_user_web_search_and_prunes_only_injected_value() {
        with_codex_home(|home| {
            let config = sample_config(18793);
            let config_path = home.join("config.toml");
            std::fs::write(&config_path, "web_search = \"live\"\n").unwrap();
            apply_codex_config(&config).unwrap();
            let taken = std::fs::read_to_string(&config_path).unwrap();
            assert!(
                taken.contains("web_search = \"live\""),
                "用户显式配置不能被覆盖：{taken}"
            );
            clear_sugt_codex_overrides(&config).unwrap();
            assert_eq!(
                std::fs::read_to_string(&config_path).unwrap(),
                "web_search = \"live\"\n"
            );

            // 用户原本没有该键时，SUGT 注入并在释放后清掉（走兜底路径）。
            std::fs::write(&config_path, "model = \"o3\"\n").unwrap();
            apply_codex_config(&config).unwrap();
            let mut edited = std::fs::read_to_string(&config_path).unwrap();
            edited.push_str("\n# 用户手动新增\n");
            std::fs::write(&config_path, &edited).unwrap();
            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&config_path).unwrap();
            assert!(after.contains("# 用户手动新增"));
            assert!(!after.contains("web_search"), "注入的 web_search 应被清理");
        });
    }

    #[test]
    fn legacy_prune_does_not_delete_unrelated_local_service() {
        with_codex_home(|home| {
            let config = sample_config(18791);
            let path = home.join("config.toml");
            let raw = concat!(
                "openai_base_url = \"http://127.0.0.1:9999/my-own-service/v1\"\n",
                "model_provider = \"sugt\"\n",
                "\n[model_providers.sugt]\n",
                "name = \"SUGT\"\n",
            );
            std::fs::write(&path, raw).unwrap();
            clear_sugt_codex_overrides(&config).unwrap();
            let after = std::fs::read_to_string(&path).unwrap();
            assert!(
                after.contains("my-own-service"),
                "不能再按 contains(localhost) 误删用户的本地服务地址"
            );
            assert!(!after.contains("model_provider = "));
        });
    }

    /// 复刻 1.1.7 已发布版的写入结果：没有快照文件、model 无 sugt- 别名前缀、
    /// provider 不带第三方能力声明、auth.json 只剩 SUGT 注入的两个键。
    fn write_legacy_1_1_7_takeover(home: &Path, config: &AppConfig) {
        let openai_base = format!("{}/v1", config.listen_url());
        std::fs::write(
            home.join("config.toml"),
            format!(
                concat!(
                    "model_provider = \"sugt\"\n",
                    "model = \"demo-model\"\n",
                    "openai_base_url = \"{base}\"\n",
                    "preferred_auth_method = \"apikey\"\n",
                    "cli_auth_credentials_store = \"file\"\n",
                    "\n[model_providers.sugt]\n",
                    "name = \"SUGT\"\n",
                    "base_url = \"{base}\"\n",
                    "wire_api = \"responses\"\n"
                ),
                base = openai_base
            ),
        )
        .unwrap();
        std::fs::write(
            home.join("auth.json"),
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": crate::clients::client_api_key_for_config(config),
            })
            .to_string(),
        )
        .unwrap();
        let _ = std::fs::remove_file(home.join(STATE_FILE));
    }

    /// DEF-004：1.1.7 升级用户取消接管时，不能把 SUGT 自己写入的固定值
    /// 当成「用户原值」保守保留，否则仍是半接管状态。
    #[test]
    fn release_cleans_legacy_1_1_7_takeover_residue() {
        with_codex_home(|home| {
            let config = sample_config(18801);
            write_legacy_1_1_7_takeover(home, &config);

            clear_sugt_codex_overrides(&config).unwrap();

            let after = std::fs::read_to_string(home.join("config.toml")).unwrap();
            let table: toml::Table = toml::from_str(&after).unwrap();
            for key in [
                "model_provider",
                "model_providers",
                "openai_base_url",
                "model",
                "preferred_auth_method",
                "cli_auth_credentials_store",
            ] {
                assert!(
                    table.get(key).is_none(),
                    "1.1.7 残留字段 {key} 未被清理：{after}"
                );
            }
            let auth_path = home.join("auth.json");
            if auth_path.is_file() {
                let auth: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
                assert!(
                    auth.get("OPENAI_API_KEY").is_none(),
                    "1.1.7 注入的本地网关 key 必须清理：{auth}"
                );
                assert!(
                    auth.get("auth_mode").and_then(|v| v.as_str()) != Some("apikey"),
                    "apikey 认证方式必须清理，否则 ChatGPT 登录态不可用：{auth}"
                );
            }
        });
    }

    /// DEF-004：用户在 1.1.7 接管期间手工补过登录态字段时，
    /// 只清 SUGT 注入的两个键，其它字段一律保留。
    #[test]
    fn release_keeps_user_fields_when_legacy_auth_was_edited() {
        with_codex_home(|home| {
            let config = sample_config(18802);
            write_legacy_1_1_7_takeover(home, &config);
            let auth_path = home.join("auth.json");
            std::fs::write(
                &auth_path,
                serde_json::json!({
                    "auth_mode": "apikey",
                    "OPENAI_API_KEY": crate::clients::client_api_key_for_config(&config),
                    "tokens": { "id_token": "keep" },
                    "account_id": "acct_1",
                })
                .to_string(),
            )
            .unwrap();

            clear_sugt_codex_overrides(&config).unwrap();

            let auth: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
            assert_eq!(auth["tokens"]["id_token"], "keep", "用户登录态字段必须保留");
            assert_eq!(auth["account_id"], "acct_1");
            assert!(
                auth.get("OPENAI_API_KEY").is_none(),
                "假网关 key 必须清理：{auth}"
            );
            assert!(
                auth.get("auth_mode").is_none(),
                "SUGT 注入的 apikey 必须清理：{auth}"
            );
        });
    }

    /// DEF-007：替换失败时不能留下临时文件，也不能破坏原内容。
    /// 目标是目录时 rename 必然失败，这条用例在所有平台都能复现该失败路径。
    #[test]
    fn atomic_write_failure_cleans_temp_and_keeps_original() {
        with_codex_home(|home| {
            let target = home.join("config.toml");
            std::fs::create_dir(&target).unwrap();
            std::fs::write(target.join("keep.txt"), "keep").unwrap();

            let result = atomic_write(&target, "new content");
            assert!(result.is_err(), "目标是目录时替换应当失败");
            assert!(
                target.join("keep.txt").is_file(),
                "替换失败后原内容必须保持不变"
            );
            let tmp = home.join("config.toml.sugt-tmp");
            assert!(!tmp.exists(), "替换失败后必须清理临时文件：{tmp:?}");
        });
    }

    /// DEF-007：替换失败时把原文件从备份位还原，绝不丢用户配置。
    #[test]
    fn replace_with_backup_restores_original_when_rename_fails() {
        with_codex_home(|home| {
            let target = home.join("config.toml");
            std::fs::write(&target, "original").unwrap();
            let missing_tmp = home.join("missing.tmp");
            assert!(
                replace_with_backup(&target, &missing_tmp).is_err(),
                "临时文件不存在时替换必须失败"
            );
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "original",
                "替换失败后原文件必须从备份位还原"
            );
        });
    }

    /// 正常写入后临时文件与备份文件都不能残留。
    #[test]
    fn atomic_write_success_cleans_temp_and_backup() {
        with_codex_home(|home| {
            let target = home.join("config.toml");
            std::fs::write(&target, "old").unwrap();
            atomic_write(&target, "new").unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
            assert!(!home.join("config.toml.sugt-tmp").exists());
            let leftovers: Vec<String> = std::fs::read_dir(home)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name.contains(".backup"))
                .collect();
            assert!(leftovers.is_empty(), "成功后不能留下备份文件：{leftovers:?}");
        });
    }
}
