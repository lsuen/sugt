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

/// 单把密钥的运行时用量（只累加，不参与拦截）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyUsage {
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
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
    #[serde(default)]
    pub usage: ClientKeyUsage,
}

fn default_enabled() -> bool {
    true
}

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
            entry.usage.last_used_at = Some(Utc::now().to_rfc3339());
            true
        }
        None => false,
    }
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

/// 单个密钥的展示视图：`key` 保留明文（与 provider 表单一致），另给脱敏副本。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyView {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_masked: String,
    pub enabled: bool,
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
    }
}
