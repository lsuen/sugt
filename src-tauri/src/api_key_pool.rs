//! API Key 池数据模型与持久化（PRD v1.1.11 R5）
//!
//! 一个池绑定一个 provider，池内可放多个 key，按 `round_robin` / `least_used`
//! 轮换使用。key 明文存储（与既有 `ProviderConfig.api_key` 一致），只在展示层脱敏。
//! 用量是运行时统计值：保存池配置时按 key id 保留旧 `usage`，避免前端保存
//! 配置就把计数清零。
//!
//! 持久化文件 `config_dir/api-key-pools.json`，结构
//! `{ "version": 1, "pools": [...] }`，与前端统一 camelCase。
//! 文件缺失或解析失败回退空表并告警，绝不 panic。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 池文件名（位于 config_dir 下）。
const POOLS_FILE: &str = "api-key-pools.json";
const POOLS_VERSION: u32 = 1;

/// key 轮换策略。序列化为 `"roundRobin"` / `"leastUsed"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySelection {
    RoundRobin,
    LeastUsed,
}

impl Default for KeySelection {
    fn default() -> Self {
        Self::RoundRobin
    }
}

/// 单个 key 的运行时用量（一期只监控，不参与拦截）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyUsage {
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// key 的配额上限（一期只读展示，达到也继续放行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyQuota {
    #[serde(default)]
    pub request_limit: Option<u64>,
    #[serde(default)]
    pub token_limit: Option<u64>,
}

/// 池内的单个 key。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyEntry {
    #[serde(default)]
    pub id: String,
    pub key: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub usage: KeyUsage,
    #[serde(default)]
    pub quota: Option<KeyQuota>,
}

fn default_enabled() -> bool {
    true
}

/// 一个 API Key 池：绑定 provider + 多个 key + 一种轮换策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyPool {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub provider_id: String,
    #[serde(default)]
    pub selection: KeySelection,
    #[serde(default)]
    pub keys: Vec<ApiKeyEntry>,
}

/// 池文件顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyPoolsFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub pools: Vec<ApiKeyPool>,
}

fn default_version() -> u32 {
    POOLS_VERSION
}

impl Default for ApiKeyPoolsFile {
    fn default() -> Self {
        Self {
            version: POOLS_VERSION,
            pools: Vec::new(),
        }
    }
}

fn pools_path(config_dir: &Path) -> PathBuf {
    config_dir.join(POOLS_FILE)
}

/// 展示用脱敏：复用 `model::mask_secret`，不在本模块复制第二份实现。
pub fn mask_key(secret: &str) -> String {
    crate::model::mask_secret(secret)
}

/// 读取池文件。文件不存在、IO 失败或内容损坏都返回空表并告警，调用方无需处理 IO 错误。
pub fn load(config_dir: &Path) -> ApiKeyPoolsFile {
    let path = pools_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ApiKeyPoolsFile::default(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "读取 API Key 池失败，按空表处理");
            return ApiKeyPoolsFile::default();
        }
    };
    match serde_json::from_str::<ApiKeyPoolsFile>(&raw) {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "API Key 池不是合法 JSON，按空表处理");
            ApiKeyPoolsFile::default()
        }
    }
}

/// 持久化池文件（原子写：pid 临时文件 → 原文件备份 → rename → 失败回滚）。
/// 版本号强制写 [`POOLS_VERSION`]，避免把内存里的临时值落到磁盘。
pub fn save(config_dir: &Path, file: &ApiKeyPoolsFile) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let payload = ApiKeyPoolsFile {
        version: POOLS_VERSION,
        pools: file.pools.clone(),
    };
    let content = serde_json::to_string_pretty(&payload).context("序列化 API Key 池失败")?;
    atomic_write(&pools_path(config_dir), &content)
}

/// 校验单个池；`existing` 用于池名重名检查（同一 id 自身不算重名）。
pub fn validate(pool: &ApiKeyPool, existing: &[ApiKeyPool]) -> Result<(), String> {
    let name = pool.name.trim();
    if name.is_empty() {
        return Err("池名称不能为空".to_string());
    }
    if pool.provider_id.trim().is_empty() {
        return Err("API Key 池必须绑定一个 provider".to_string());
    }
    if pool.keys.is_empty() {
        return Err("API Key 池至少需要一个 key".to_string());
    }
    for (index, entry) in pool.keys.iter().enumerate() {
        if entry.key.trim().is_empty() {
            return Err(format!("第 {} 个 key 不能为空", index + 1));
        }
    }
    if existing
        .iter()
        .any(|other| other.id != pool.id && other.name.trim() == name)
    {
        return Err(format!("池名称已存在: {name}"));
    }
    Ok(())
}

/// 插入或替换：池 id / key id 为空则生成 uuid，id 已存在则整体替换。
///
/// 同 id 的 key 保留旧 `usage`（前端保存配置不该清零计数），旧的 `quota`
/// 仅在传入为 `None` 时保留。这是用量累加能存活下来的前提。
pub fn upsert(pools: &mut Vec<ApiKeyPool>, spec: ApiKeyPool) -> ApiKeyPool {
    let mut spec = spec;
    spec.id = spec.id.trim().to_string();
    if spec.id.is_empty() {
        spec.id = uuid::Uuid::new_v4().to_string();
    }
    spec.name = spec.name.trim().to_string();
    spec.provider_id = spec.provider_id.trim().to_string();
    for entry in &mut spec.keys {
        entry.id = entry.id.trim().to_string();
        if entry.id.is_empty() {
            entry.id = uuid::Uuid::new_v4().to_string();
        }
        entry.key = entry.key.trim().to_string();
        if let Some(error) = entry.usage.last_error.as_mut() {
            *error = error.trim().to_string();
        }
    }

    match pools.iter().position(|pool| pool.id == spec.id) {
        Some(position) => {
            let old = pools[position].clone();
            for entry in &mut spec.keys {
                if let Some(old_entry) = old.keys.iter().find(|old| old.id == entry.id) {
                    entry.usage = old_entry.usage.clone();
                    if entry.quota.is_none() {
                        entry.quota = old_entry.quota.clone();
                    }
                }
            }
            pools[position] = spec.clone();
        }
        None => pools.push(spec.clone()),
    }
    spec
}

/// 按 id 查池。
pub fn find_pool<'a>(pools: &'a [ApiKeyPool], pool_id: &str) -> Option<&'a ApiKeyPool> {
    pools.iter().find(|pool| pool.id == pool_id)
}

/// 删除指定池，返回是否真的删掉。
pub fn remove_pool(pools: &mut Vec<ApiKeyPool>, pool_id: &str) -> bool {
    let before = pools.len();
    pools.retain(|pool| pool.id != pool_id);
    pools.len() != before
}

/// 清空用量：`None` 清整池，`Some(entry_id)` 只清指定 key。
pub fn reset_usage(pool: &mut ApiKeyPool, entry_id: Option<&str>) {
    match entry_id {
        None => {
            for entry in &mut pool.keys {
                entry.usage = KeyUsage::default();
            }
        }
        Some(id) => {
            if let Some(entry) = pool.keys.iter_mut().find(|entry| entry.id == id) {
                entry.usage = KeyUsage::default();
            }
        }
    }
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
/// 与 `model_registry.rs::atomic_write` 同一做法，避免 Windows rename 不能覆盖导致丢文件。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("api-key-pools");
    let tmp = path.with_file_name(format!("{name}.sugt-tmp"));
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
        .unwrap_or("api-key-pools");
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
            "sugt-api-key-pool-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn entry(id: &str, key: &str) -> ApiKeyEntry {
        ApiKeyEntry {
            id: id.to_string(),
            key: key.to_string(),
            enabled: true,
            usage: KeyUsage::default(),
            quota: None,
        }
    }

    fn pool(id: &str, name: &str, keys: Vec<ApiKeyEntry>) -> ApiKeyPool {
        ApiKeyPool {
            id: id.to_string(),
            name: name.to_string(),
            provider_id: "prov".to_string(),
            selection: KeySelection::RoundRobin,
            keys,
        }
    }

    #[test]
    fn missing_file_returns_empty_pools() {
        let dir = temp_dir("missing");
        let file = load(&dir);
        assert!(file.pools.is_empty());
        assert_eq!(file.version, POOLS_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip_keeps_usage_quota_selection() {
        let dir = temp_dir("roundtrip");
        let mut first = entry("k1", "sk-first");
        first.usage = KeyUsage {
            request_count: 7,
            total_tokens: 1024,
            last_error: Some("429".to_string()),
        };
        first.quota = Some(KeyQuota {
            request_limit: Some(100),
            token_limit: Some(200_000),
        });
        let mut spec = pool("p1", "主池", vec![first, entry("k2", "sk-second")]);
        spec.selection = KeySelection::LeastUsed;
        let file = ApiKeyPoolsFile {
            version: POOLS_VERSION,
            pools: vec![spec],
        };
        save(&dir, &file).expect("save");

        let loaded = load(&dir);
        assert_eq!(loaded.version, POOLS_VERSION);
        assert_eq!(loaded.pools.len(), 1);
        let loaded_pool = &loaded.pools[0];
        assert_eq!(loaded_pool.id, "p1");
        assert_eq!(loaded_pool.name, "主池");
        assert_eq!(loaded_pool.provider_id, "prov");
        assert_eq!(loaded_pool.selection, KeySelection::LeastUsed);
        assert_eq!(loaded_pool.keys.len(), 2);
        let k1 = &loaded_pool.keys[0];
        assert_eq!(k1.usage.request_count, 7);
        assert_eq!(k1.usage.total_tokens, 1024);
        assert_eq!(k1.usage.last_error.as_deref(), Some("429"));
        assert_eq!(
            k1.quota.as_ref().and_then(|quota| quota.request_limit),
            Some(100)
        );
        assert_eq!(
            k1.quota.as_ref().and_then(|quota| quota.token_limit),
            Some(200_000)
        );
        // camelCase 契约：落盘字段名必须是前端读取的那个。
        let raw = std::fs::read_to_string(pools_path(&dir)).expect("read file");
        assert!(raw.contains("\"providerId\""));
        assert!(raw.contains("\"leastUsed\""));
        assert!(raw.contains("\"requestCount\""));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_empty_pools() {
        let dir = temp_dir("corrupt");
        std::fs::write(pools_path(&dir), "{not valid json").expect("write corrupt file");
        let file = load(&dir);
        assert!(file.pools.is_empty());
        assert_eq!(file.version, POOLS_VERSION);

        save(&dir, &file).expect("save over corrupt file");
        assert!(load(&dir).pools.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_rejects_invalid_inputs() {
        let existing = vec![pool("keep", "已有池", vec![entry("k1", "sk-keep")])];

        let empty_name = pool("", "  ", vec![entry("k1", "sk-a")]);
        assert!(validate(&empty_name, &existing).is_err());

        let empty_provider = ApiKeyPool {
            provider_id: "  ".to_string(),
            ..pool("", "新池", vec![entry("k1", "sk-a")])
        };
        assert!(validate(&empty_provider, &existing).is_err());

        let empty_keys = pool("", "新池", Vec::new());
        assert!(validate(&empty_keys, &existing).is_err());

        let blank_key = pool("", "新池", vec![entry("k1", "  ")]);
        assert!(validate(&blank_key, &existing).is_err());

        let duplicate = pool("", "已有池", vec![entry("k1", "sk-a")]);
        assert!(validate(&duplicate, &existing).is_err());

        // 同一 id 重新保存自身不算重名。
        let self_save = pool("keep", "已有池", vec![entry("k1", "sk-keep")]);
        assert!(validate(&self_save, &existing).is_ok());
    }

    #[test]
    fn upsert_generates_uuid_and_preserves_usage() {
        let mut pools = Vec::new();
        let created = upsert(
            &mut pools,
            pool("", " 主池 ", vec![entry("", " sk-first ")]),
        );
        assert!(!created.id.is_empty());
        assert_eq!(created.name, "主池");
        assert_eq!(created.keys[0].key, "sk-first");
        let pool_id = created.id.clone();
        let key_id = created.keys[0].id.clone();
        assert!(!key_id.is_empty());
        assert_eq!(pools.len(), 1);

        // 模拟网关累计用量后再保存配置（同一个 key id 传回）。
        pools[0].keys[0].usage = KeyUsage {
            request_count: 12,
            total_tokens: 4096,
            last_error: Some("timeout".to_string()),
        };
        pools[0].keys[0].quota = Some(KeyQuota {
            request_limit: Some(500),
            token_limit: None,
        });

        let mut respec = pool(
            &pool_id,
            "主池改名",
            vec![entry(&key_id, "sk-first"), entry("", "sk-second")],
        );
        respec.selection = KeySelection::LeastUsed;
        let updated = upsert(&mut pools, respec);

        assert_eq!(pools.len(), 1);
        assert_eq!(updated.id, pool_id);
        assert_eq!(pools[0].name, "主池改名");
        assert_eq!(pools[0].selection, KeySelection::LeastUsed);
        assert_eq!(pools[0].keys.len(), 2);
        // 老 key 的用量与配额存活，新 key 从零开始。
        assert_eq!(pools[0].keys[0].usage.request_count, 12);
        assert_eq!(pools[0].keys[0].usage.total_tokens, 4096);
        assert_eq!(
            pools[0].keys[0].usage.last_error.as_deref(),
            Some("timeout")
        );
        assert_eq!(
            pools[0].keys[0]
                .quota
                .as_ref()
                .and_then(|quota| quota.request_limit),
            Some(500)
        );
        assert_ne!(pools[0].keys[1].id, key_id);
        assert_eq!(pools[0].keys[1].usage.request_count, 0);
        assert_eq!(pools[0].keys[1].usage.total_tokens, 0);
        assert!(pools[0].keys[1].usage.last_error.is_none());
        assert!(pools[0].keys[1].quota.is_none());
    }

    #[test]
    fn reset_usage_handles_single_key_and_whole_pool() {
        let filled = || KeyUsage {
            request_count: 3,
            total_tokens: 99,
            last_error: Some("boom".to_string()),
        };
        let mut spec = pool(
            "p1",
            "池",
            vec![entry("k1", "sk-1"), entry("k2", "sk-2")],
        );
        spec.keys[0].usage = filled();
        spec.keys[1].usage = filled();

        reset_usage(&mut spec, Some("k1"));
        assert_eq!(spec.keys[0].usage.request_count, 0);
        assert!(spec.keys[0].usage.last_error.is_none());
        assert_eq!(spec.keys[1].usage.request_count, 3);

        reset_usage(&mut spec, None);
        assert!(spec
            .keys
            .iter()
            .all(|entry| entry.usage.request_count == 0 && entry.usage.total_tokens == 0));
    }

    #[test]
    fn find_and_remove_pool() {
        let mut pools = vec![
            pool("p1", "池一", vec![entry("k1", "sk-1")]),
            pool("p2", "池二", vec![entry("k2", "sk-2")]),
        ];
        assert_eq!(find_pool(&pools, "p2").map(|pool| pool.name.as_str()), Some("池二"));
        assert!(find_pool(&pools, "missing").is_none());

        assert!(remove_pool(&mut pools, "p1"));
        assert_eq!(pools.len(), 1);
        assert!(!remove_pool(&mut pools, "p1"));
        assert!(find_pool(&pools, "p1").is_none());
    }

    #[test]
    fn mask_key_reuses_model_mask_secret() {
        assert_eq!(mask_key("sk-1234567890abcd"), "sk-1****abcd");
        assert_eq!(mask_key("short"), "********");
    }
}
