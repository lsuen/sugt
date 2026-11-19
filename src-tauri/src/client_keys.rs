//! 入站客户端密钥（gateway client keys）
//!
//! 客户端调用 SUGT 网关时携带的 key。等价于 `AppConfig.gateway_client_api_key`
//! 的多 key 版本：可建多把、可启用停用、可统计使用次数。与 `api_key_pool` 是
//! 两套概念——后者管的是「发往上游 provider 的 key 轮换」，互不影响。
//!
//! 持久化文件 `config_dir/client-keys.json`，结构 `{ "version": 1, "keys": [...] }`，
//! 与前端统一 camelCase。文件缺失或解析失败回退空表并告警，绝不 panic。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// 密钥文件名（位于 config_dir 下）。
const KEYS_FILE: &str = "client-keys.json";
const KEYS_VERSION: u32 = 1;
/// 生成密钥的固定前缀。
const KEY_PREFIX: &str = "sk-sugt-";

/// 单个客户端（label + ip）的用量，用于展示「谁在用、用到什么程度」。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientUsage {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub last_seen_at: String,
}

/// 单把密钥的运行时用量（只累加，不参与拦截）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyUsage {
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    /// 最近使用过的客户端（最多 [`MAX_CLIENTS`] 条，按 `last_seen_at` 倒序）。
    #[serde(default)]
    pub clients: Vec<ClientUsage>,
}

/// 配额（只展示与编辑，不参与拦截）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientQuota {
    #[serde(default)]
    pub request_limit: Option<u64>,
    #[serde(default)]
    pub token_limit: Option<u64>,
}

/// 一把入站密钥。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKey {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub key: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 主密钥：不可删除、不可停用；其 key 同步到 `gateway_client_api_key`。
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub quota: ClientQuota,
    #[serde(default)]
    pub usage: ClientKeyUsage,
}

fn default_enabled() -> bool {
    true
}

/// 每个密钥最多保留多少个客户端条目。
pub const MAX_CLIENTS: usize = 32;

/// 密钥文件顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeysFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub keys: Vec<ClientKey>,
}

fn default_version() -> u32 {
    KEYS_VERSION
}

impl Default for ClientKeysFile {
    fn default() -> Self {
        Self {
            version: KEYS_VERSION,
            keys: Vec::new(),
        }
    }
}

fn keys_path(config_dir: &Path) -> PathBuf {
    config_dir.join(KEYS_FILE)
}

/// 读取密钥文件。文件不存在、IO 失败或内容损坏都返回空表并告警。
pub fn load(config_dir: &Path) -> ClientKeysFile {
    let path = keys_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ClientKeysFile::default(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "读取入站密钥失败，按空表处理");
            return ClientKeysFile::default();
        }
    };
    match serde_json::from_str::<ClientKeysFile>(&raw) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "入站密钥不是合法 JSON，按空表处理");
            ClientKeysFile::default()
        }
    }
}

/// 持久化密钥文件（原子写：pid 临时文件 → 原文件备份 → rename → 失败回滚）。
/// 版本号强制写 [`KEYS_VERSION`]，避免把内存里的临时值落到磁盘。
pub fn save(config_dir: &Path, file: &ClientKeysFile) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let payload = ClientKeysFile {
        version: KEYS_VERSION,
        keys: file.keys.clone(),
    };
    let content = serde_json::to_string_pretty(&payload).context("序列化入站密钥失败")?;
    atomic_write(&keys_path(config_dir), &content)
}

/// 生成一把新密钥：`sk-sugt-` + 32 位 uuid simple（小写十六进制），共 40 字符。
pub fn generate_key() -> String {
    format!("{KEY_PREFIX}{}", uuid::Uuid::new_v4().simple())
}

/// 校验密钥名称：不能为空。
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("密钥名称不能为空".to_string());
    }
    Ok(())
}

/// 插入或替换：id 为空生成 uuid；id 已存在时保留旧 `usage`（保存配置不该清零计数）。
pub fn upsert(keys: &mut Vec<ClientKey>, spec: ClientKey) -> ClientKey {
    let mut spec = spec;
    spec.id = spec.id.trim().to_string();
    if spec.id.is_empty() {
        spec.id = uuid::Uuid::new_v4().to_string();
    }
    spec.name = spec.name.trim().to_string();
    spec.key = spec.key.trim().to_string();
    if let Some(error) = spec.usage.last_error.as_mut() {
        *error = error.trim().to_string();
    }

    match keys.iter().position(|key| key.id == spec.id) {
        Some(position) => {
            spec.usage = keys[position].usage.clone();
            keys[position] = spec.clone();
        }
        None => keys.push(spec.clone()),
    }
    spec
}

/// 按 id 查密钥。
pub fn find_key<'a>(keys: &'a [ClientKey], id: &str) -> Option<&'a ClientKey> {
    keys.iter().find(|key| key.id == id)
}

/// 删除指定密钥，返回是否真的删掉。
pub fn remove_key(keys: &mut Vec<ClientKey>, id: &str) -> bool {
    let before = keys.len();
    keys.retain(|key| key.id != id);
    keys.len() != before
}

/// 可放行的 key 字符串：只取 `enabled` 且非空，用于入站校验。
pub fn enabled_key_strings(file: &ClientKeysFile) -> Vec<String> {
    file.keys
        .iter()
        .filter(|key| key.enabled && !key.key.trim().is_empty())
        .map(|key| key.key.trim().to_string())
        .collect()
}

/// 累计一次请求：`request_count + 1`，`last_used_at` 写 RFC3339 UTC。
pub fn record_use(file: &mut ClientKeysFile, key: &str) -> bool {
    let target = key.trim();
    match file.keys.iter_mut().find(|entry| entry.key.trim() == target) {
        Some(entry) => {
            entry.usage.request_count = entry.usage.request_count.saturating_add(1);
            entry.usage.last_used_at = Some(now_rfc3339());
            true
        }
        None => false,
    }
}

/// 记一次入站请求，并更新对应客户端（`label + ip`）的计数。
///
/// 与 [`record_use`] 的区别：这里同时维护 `usage.clients`，是「谁在用」的数据来源。
pub fn record_request(file: &mut ClientKeysFile, key: &str, label: &str, ip: &str) -> bool {
    let target = key.trim();
    let Some(entry) = file.keys.iter_mut().find(|entry| entry.key.trim() == target) else {
        return false;
    };
    entry.usage.request_count = entry.usage.request_count.saturating_add(1);
    entry.usage.last_used_at = Some(now_rfc3339());
    {
        let client = touch_client(&mut entry.usage.clients, label, ip);
        client.request_count = client.request_count.saturating_add(1);
    }
    trim_clients(&mut entry.usage.clients);
    true
}

/// 把一次响应的 token 用量累加到密钥与对应客户端上（客户端不存在则创建）。
pub fn record_tokens(
    file: &mut ClientKeysFile,
    key: &str,
    label: &str,
    ip: &str,
    tokens: u64,
) -> bool {
    let target = key.trim();
    let Some(entry) = file.keys.iter_mut().find(|entry| entry.key.trim() == target) else {
        return false;
    };
    entry.usage.total_tokens = entry.usage.total_tokens.saturating_add(tokens);
    entry.usage.last_used_at = Some(now_rfc3339());
    {
        let client = touch_client(&mut entry.usage.clients, label, ip);
        client.total_tokens = client.total_tokens.saturating_add(tokens);
    }
    trim_clients(&mut entry.usage.clients);
    true
}

/// 找到或插入一个客户端条目，并刷新 `last_seen_at`。
fn touch_client<'a>(
    clients: &'a mut Vec<ClientUsage>,
    label: &str,
    ip: &str,
) -> &'a mut ClientUsage {
    let position = clients
        .iter()
        .position(|client| client.label == label && client.ip == ip);
    let index = match position {
        Some(index) => index,
        None => {
            clients.push(ClientUsage {
                label: label.to_string(),
                ip: ip.to_string(),
                ..Default::default()
            });
            clients.len() - 1
        }
    };
    clients[index].last_seen_at = now_rfc3339();
    &mut clients[index]
}

/// 按 `last_seen_at` 倒序并截断到 [`MAX_CLIENTS`]。
fn trim_clients(clients: &mut Vec<ClientUsage>) {
    clients.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
    clients.truncate(MAX_CLIENTS);
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// 记录最近一次错误（截断到 300 字符，避免把整段上游响应写进配置）。
pub fn record_error(file: &mut ClientKeysFile, key: &str, message: &str) -> bool {
    let target = key.trim();
    match file.keys.iter_mut().find(|entry| entry.key.trim() == target) {
        Some(entry) => {
            let summary: String = message.trim().chars().take(300).collect();
            entry.usage.last_error = Some(summary);
            true
        }
        None => false,
    }
}

/// 保证池内存在一把主密钥。
///
/// - 已有 primary：幂等返回 `false`；
/// - 池非空但没有 primary：提升第一条（并强制启用）；
/// - 池为空：用配置里的 `gateway_client_api_key`（trim 后为空则 `sugt-local-key`）补种。
pub fn ensure_primary(file: &mut ClientKeysFile, configured_key: &str) -> bool {
    // 旧版补种名「本机默认密钥」→「默认」：只改展示名，不碰 key / id / 用量。
    let mut renamed = false;
    for key in file.keys.iter_mut() {
        if key.primary && key.name == "本机默认密钥" {
            key.name = "默认".to_string();
            renamed = true;
        }
    }
    if file.keys.iter().any(|key| key.primary) {
        return renamed;
    }
    if let Some(first) = file.keys.first_mut() {
        first.primary = true;
        first.enabled = true;
        return true;
    }
    let configured = configured_key.trim();
    let key = if configured.is_empty() {
        crate::gateway_auth::DUMMY_LOCAL_KEY.to_string()
    } else {
        configured.to_string()
    };
    file.keys.push(ClientKey {
        id: "local".to_string(),
        name: "默认".to_string(),
        key,
        enabled: true,
        primary: true,
        quota: ClientQuota::default(),
        usage: ClientKeyUsage::default(),
    });
    true
}

/// 主密钥不可停用。命令层调用它给出中文错误。
pub fn validate_update(existing: &ClientKey, enabled: Option<bool>) -> Result<(), String> {
    if existing.primary && enabled == Some(false) {
        return Err("主密钥不可停用".to_string());
    }
    Ok(())
}

/// 主密钥不可删除。
pub fn validate_delete(existing: &ClientKey) -> Result<(), String> {
    if existing.primary {
        return Err("主密钥不可删除".to_string());
    }
    Ok(())
}

/// 清空用量（保留 key、名称、启用状态与配额）。
pub fn reset_usage(key: &mut ClientKey) {
    key.usage = ClientKeyUsage::default();
}

/// 单个密钥的展示视图：`key` 保留明文（与 provider 表单一致），另给脱敏副本。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyView {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_masked: String,
    pub enabled: bool,
    pub primary: bool,
    pub quota: ClientQuota,
    pub usage: ClientKeyUsage,
}

impl From<&ClientKey> for ClientKeyView {
    fn from(key: &ClientKey) -> Self {
        Self {
            id: key.id.clone(),
            name: key.name.clone(),
            key: key.key.clone(),
            key_masked: crate::model::mask_secret(&key.key),
            enabled: key.enabled,
            primary: key.primary,
            quota: key.quota.clone(),
            usage: key.usage.clone(),
        }
    }
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
/// 与 `api_key_pool.rs::atomic_write` 同一做法，避免 Windows rename 不能覆盖导致丢文件。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("client-keys");
    let tmp = path.with_file_name(format!("{name}.sugt-{}.tmp", std::process::id()));
    std::fs::write(&tmp, content)
        .with_context(|| format!("无法写入临时文件 {}", tmp.display()))?;
    match replace_with_backup(path, &tmp) {
        Ok(()) => Ok(()),
        Err(err) => {
            // 失败绝不留下半成品临时文件。
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn replace_with_backup(path: &Path, tmp: &Path) -> Result<()> {
    if path.is_dir() {
        anyhow::bail!("目标路径是目录，拒绝替换 {}", path.display());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("client-keys");
    let backup = path.with_file_name(format!("{name}.sugt-{}.backup", std::process::id()));
    if path.exists() {
        std::fs::rename(path, &backup)
            .with_context(|| format!("无法备份 {}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// OS 中立的独立临时目录，避免 Windows 下 /tmp 字面量与测试互相干扰。
    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sugt-client-keys-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn entry(id: &str, name: &str, key: &str) -> ClientKey {
        ClientKey {
            id: id.to_string(),
            name: name.to_string(),
            key: key.to_string(),
            enabled: true,
            primary: false,
            quota: ClientQuota::default(),
            usage: ClientKeyUsage::default(),
        }
    }

    #[test]
    fn missing_file_returns_empty_keys() {
        let dir = temp_dir("missing");
        let file = load(&dir);
        assert!(file.keys.is_empty());
        assert_eq!(file.version, KEYS_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_uses_camel_case_fields() {
        let dir = temp_dir("roundtrip");
        let mut first = entry("k1", "团队", "sk-sugt-first");
        first.usage = ClientKeyUsage {
            request_count: 5,
            last_used_at: Some("2026-11-18T01:02:03+00:00".to_string()),
            last_error: Some("429".to_string()),
            ..Default::default()
        };
        let mut second = entry("k2", "备用", "sk-sugt-second");
        second.enabled = false;
        let file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![first, second],
        };
        save(&dir, &file).expect("save");

        let loaded = load(&dir);
        assert_eq!(loaded.version, KEYS_VERSION);
        assert_eq!(loaded.keys.len(), 2);
        assert_eq!(loaded.keys[0].name, "团队");
        assert_eq!(loaded.keys[0].key, "sk-sugt-first");
        assert_eq!(loaded.keys[0].usage.request_count, 5);
        assert_eq!(
            loaded.keys[0].usage.last_used_at.as_deref(),
            Some("2026-11-18T01:02:03+00:00")
        );
        assert_eq!(loaded.keys[0].usage.last_error.as_deref(), Some("429"));
        assert!(!loaded.keys[1].enabled);

        // camelCase 契约：落盘字段名必须是前端读取的那个。
        let raw = std::fs::read_to_string(keys_path(&dir)).expect("read file");
        assert!(raw.contains("\"requestCount\""));
        assert!(raw.contains("\"lastUsedAt\""));
        assert!(raw.contains("\"lastError\""));
        assert!(raw.contains("\"version\""));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_empty_keys() {
        let dir = temp_dir("corrupt");
        std::fs::write(keys_path(&dir), "{not valid json").expect("write corrupt file");
        let file = load(&dir);
        assert!(file.keys.is_empty());
        assert_eq!(file.version, KEYS_VERSION);

        save(&dir, &file).expect("save over corrupt file");
        assert!(load(&dir).keys.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_generates_uuid_and_preserves_usage() {
        let mut keys = Vec::new();
        let created = upsert(&mut keys, entry("", " 团队 ", " sk-sugt-a "));
        assert!(!created.id.is_empty());
        assert_eq!(created.name, "团队");
        assert_eq!(created.key, "sk-sugt-a");
        assert_eq!(keys.len(), 1);
        let id = created.id.clone();

        keys[0].usage = ClientKeyUsage {
            request_count: 9,
            last_used_at: Some("2026-11-18T01:02:03+00:00".to_string()),
            last_error: Some("timeout".to_string()),
            ..Default::default()
        };

        let updated = upsert(&mut keys, entry(&id, "团队改名", "sk-sugt-a"));
        assert_eq!(keys.len(), 1);
        assert_eq!(updated.id, id);
        assert_eq!(keys[0].name, "团队改名");
        // 同 id 保留旧用量，保存配置不会清零计数。
        assert_eq!(keys[0].usage.request_count, 9);
        assert_eq!(
            keys[0].usage.last_used_at.as_deref(),
            Some("2026-11-18T01:02:03+00:00")
        );
        assert_eq!(keys[0].usage.last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn find_and_remove_key() {
        let mut keys = vec![entry("k1", "一", "sk-1"), entry("k2", "二", "sk-2")];
        assert_eq!(find_key(&keys, "k2").map(|key| key.name.as_str()), Some("二"));
        assert!(find_key(&keys, "missing").is_none());

        assert!(remove_key(&mut keys, "k1"));
        assert_eq!(keys.len(), 1);
        assert!(!remove_key(&mut keys, "k1"));
        assert!(find_key(&keys, "k1").is_none());
    }

    #[test]
    fn enabled_key_strings_skips_disabled_and_blank() {
        let mut disabled = entry("k1", "停用", "sk-disabled");
        disabled.enabled = false;
        let blank = entry("k2", "空", "   ");
        let file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![disabled, blank, entry("k3", "可用", " sk-enabled ")],
        };
        assert_eq!(enabled_key_strings(&file), vec!["sk-enabled".to_string()]);
        assert!(enabled_key_strings(&ClientKeysFile::default()).is_empty());
    }

    #[test]
    fn generate_key_has_prefix_and_hex_suffix() {
        let key = generate_key();
        assert!(key.starts_with(KEY_PREFIX));
        let suffix = &key[KEY_PREFIX.len()..];
        // 单次 uuid simple：32 位小写十六进制。
        assert_eq!(suffix.len(), 32);
        assert!(suffix
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
        assert_ne!(key, generate_key());
    }

    #[test]
    fn validate_name_rejects_blank() {
        assert!(validate_name("团队").is_ok());
        assert!(validate_name("   ").is_err());
    }

    #[test]
    fn record_use_and_error_hit_only_matching_key() {
        let mut file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![entry("k1", "一", "sk-1"), entry("k2", "二", "sk-2")],
        };
        assert!(record_use(&mut file, " sk-1 "));
        assert_eq!(file.keys[0].usage.request_count, 1);
        assert!(file.keys[0].usage.last_used_at.is_some());
        assert_eq!(file.keys[1].usage.request_count, 0);
        assert!(!record_use(&mut file, "sk-unknown"));

        let long = "错".repeat(1000);
        assert!(record_error(&mut file, "sk-2", &long));
        let stored = file.keys[1].usage.last_error.as_deref().expect("last_error");
        assert_eq!(stored.chars().count(), 300);
        assert!(!record_error(&mut file, "sk-unknown", "boom"));
    }

    #[test]
    fn view_masks_key_and_keeps_usage() {
        let mut key = entry("k1", "团队", "sk-sugt-1234567890abcd");
        key.usage.request_count = 3;
        let view = ClientKeyView::from(&key);
        assert_eq!(view.id, "k1");
        assert_eq!(view.name, "团队");
        assert_eq!(view.key, "sk-sugt-1234567890abcd");
        assert_eq!(view.key_masked, "sk-s****abcd");
        assert_eq!(view.usage.request_count, 3);
        assert!(!view.primary);
        assert_eq!(view.usage.total_tokens, 0);
        assert!(view.usage.clients.is_empty());
    }

    #[test]
    fn ensure_primary_seeds_once_and_is_idempotent() {
        let mut file = ClientKeysFile::default();
        assert!(ensure_primary(&mut file, "  sk-config  "));
        assert_eq!(file.keys.len(), 1);
        assert_eq!(file.keys[0].id, "local");
        assert_eq!(file.keys[0].name, "默认");
        assert_eq!(file.keys[0].key, "sk-config");
        assert!(file.keys[0].primary);
        assert!(file.keys[0].enabled);

        // 旧展示名一次性迁移；再次调用应幂等。
        file.keys[0].name = "本机默认密钥".to_string();
        assert!(ensure_primary(&mut file, "sk-config"));
        assert_eq!(file.keys[0].name, "默认");
        assert!(!ensure_primary(&mut file, "sk-config"));

        // 幂等：再来一次不新增也不改动。
        assert!(!ensure_primary(&mut file, "sk-other"));
        assert_eq!(file.keys.len(), 1);
        assert_eq!(file.keys[0].key, "sk-config");

        // 配置为空时回退占位 key。
        let mut blank = ClientKeysFile::default();
        assert!(ensure_primary(&mut blank, "   "));
        assert_eq!(blank.keys[0].key, crate::gateway_auth::DUMMY_LOCAL_KEY);
    }

    #[test]
    fn ensure_primary_promotes_first_existing_key() {
        let mut disabled = entry("k1", "旧的", "sk-1");
        disabled.enabled = false;
        let mut file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![disabled, entry("k2", "二", "sk-2")],
        };
        assert!(ensure_primary(&mut file, "sk-config"));
        assert_eq!(file.keys.len(), 2);
        assert!(file.keys[0].primary);
        assert!(file.keys[0].enabled, "提升为主密钥时必须启用");
        assert!(!file.keys[1].primary);
    }

    #[test]
    fn clients_are_capped_and_sorted_by_last_seen() {
        let mut file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![entry("k1", "一", "sk-1")],
        };
        for index in 0..(MAX_CLIENTS + 5) {
            let label = format!("client-{index:02}");
            assert!(record_request(&mut file, "sk-1", &label, "10.0.0.1"));
        }
        let clients = &file.keys[0].usage.clients;
        assert_eq!(clients.len(), MAX_CLIENTS);
        // 最新插入的排在前面。
        assert_eq!(clients[0].label, format!("client-{:02}", MAX_CLIENTS + 4));
        let mut sorted = clients.clone();
        sorted.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
        assert_eq!(
            clients.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
            sorted.iter().map(|c| c.label.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn request_and_tokens_accumulate_on_the_same_client() {
        let mut file = ClientKeysFile {
            version: KEYS_VERSION,
            keys: vec![entry("k1", "一", "sk-1")],
        };
        assert!(record_request(&mut file, "sk-1", "codex", "127.0.0.1"));
        assert!(record_tokens(&mut file, "sk-1", "codex", "127.0.0.1", 10));
        assert!(record_request(&mut file, "sk-1", "codex", "127.0.0.1"));
        assert!(record_tokens(&mut file, "sk-1", "codex", "127.0.0.1", 7));
        // 不同 ip 视为不同客户端。
        assert!(record_request(&mut file, "sk-1", "codex", "10.0.0.2"));

        let usage = &file.keys[0].usage;
        assert_eq!(usage.request_count, 3);
        assert_eq!(usage.total_tokens, 17);
        assert_eq!(usage.clients.len(), 2);
        let same = usage
            .clients
            .iter()
            .find(|client| client.ip == "127.0.0.1")
            .expect("同客户端条目");
        assert_eq!(same.label, "codex");
        assert_eq!(same.request_count, 2);
        assert_eq!(same.total_tokens, 17);
        // token 先于请求到达也能创建客户端条目。
        assert!(record_tokens(&mut file, "sk-1", "claude", "", 5));
        assert!(usage_or(&file, "claude").total_tokens == 5);
        assert!(!record_tokens(&mut file, "sk-unknown", "x", "", 1));
    }

    fn usage_or<'a>(file: &'a ClientKeysFile, label: &str) -> &'a ClientUsage {
        file.keys[0]
            .usage
            .clients
            .iter()
            .find(|client| client.label == label)
            .expect("client exists")
    }

    #[test]
    fn reset_usage_keeps_key_and_quota() {
        let mut key = entry("k1", "团队", "sk-1");
        key.quota = ClientQuota {
            request_limit: Some(100),
            token_limit: Some(1000),
        };
        key.usage.request_count = 4;
        key.usage.total_tokens = 99;
        key.usage.last_error = Some("boom".to_string());
        key.usage.clients.push(ClientUsage {
            label: "codex".to_string(),
            ip: "127.0.0.1".to_string(),
            request_count: 4,
            total_tokens: 99,
            last_seen_at: "2026-11-18T01:02:03+00:00".to_string(),
        });

        reset_usage(&mut key);
        assert_eq!(key.id, "k1");
        assert_eq!(key.key, "sk-1");
        assert_eq!(key.quota.request_limit, Some(100));
        assert_eq!(key.quota.token_limit, Some(1000));
        assert_eq!(key.usage.request_count, 0);
        assert_eq!(key.usage.total_tokens, 0);
        assert!(key.usage.clients.is_empty());
        assert!(key.usage.last_error.is_none());
    }

    #[test]
    fn quota_roundtrips_both_ways() {
        let dir = temp_dir("quota");
        let mut key = entry("k1", "团队", "sk-1");
        key.primary = true;
        key.quota = ClientQuota {
            request_limit: Some(50),
            token_limit: None,
        };
        save(
            &dir,
            &ClientKeysFile {
                version: KEYS_VERSION,
                keys: vec![key],
            },
        )
        .expect("save");

        let loaded = load(&dir);
        assert!(loaded.keys[0].primary);
        assert_eq!(loaded.keys[0].quota.request_limit, Some(50));
        assert_eq!(loaded.keys[0].quota.token_limit, None);

        // 旧文件缺字段时用 serde default 兜底（向后兼容）。
        let raw = r#"{"version":1,"keys":[{"id":"old","name":"旧","key":"sk-old","enabled":true}]}"#;
        std::fs::write(keys_path(&dir), raw).expect("write old");
        let old = load(&dir);
        assert!(!old.keys[0].primary);
        assert!(old.keys[0].quota.request_limit.is_none());
        assert_eq!(old.keys[0].usage.total_tokens, 0);
        assert!(old.keys[0].usage.clients.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn primary_cannot_be_disabled_or_removed() {
        let mut key = entry("k1", "主", "sk-main");
        key.primary = true;
        assert!(validate_update(&key, Some(false)).is_err());
        assert!(validate_update(&key, Some(true)).is_ok());
        assert!(validate_update(&key, None).is_ok());
        assert!(validate_delete(&key).is_err());

        let other = entry("k2", "普通", "sk-other");
        assert!(validate_update(&other, Some(false)).is_ok());
        assert!(validate_delete(&other).is_ok());
    }
}
