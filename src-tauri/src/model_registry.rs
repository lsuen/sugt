//! model 级启用/禁用与探测结果持久化（PRD v1.1.11 R2）
//!
//! `ProviderConfig` 只描述 provider 级配置；同一 provider 下每个 model 的
//! 启停状态与最近一次连通性探测结果放在独立的 `model-registry.json` 里，
//! 避免高频写入污染 config.toml，也不触发主配置热重载。
//!
//! 文件结构与所有前端结构统一使用 camelCase：`{ "version": 1, "entries": [...] }`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 注册表文件名（位于 config_dir 下）。
const REGISTRY_FILE: &str = "model-registry.json";
const REGISTRY_VERSION: u32 = 1;

/// 一次模型探测的结果，前端按 camelCase 读取（latencyMs / tokensPerSec / ...）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProbe {
    pub ok: bool,
    pub latency_ms: u64,
    pub tokens_per_sec: f64,
    pub completion_tokens: u32,
    pub estimated: bool,
    pub probed_at: DateTime<Utc>,
    pub error: Option<String>,
    /// 聊天可用但工具调用探测未通过；旧缓存缺字段按 false。
    #[serde(default)]
    pub chat_only: bool,
}

/// 单个模型的注册表条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    /// 稳定键：`{provider_id}::{model_name}`；前端只读，定位请用 provider_id + model_name。
    pub id: String,
    pub provider_id: String,
    pub model_name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub last_probe: Option<ModelProbe>,
}

impl ModelEntry {
    fn new(provider_id: &str, model_name: &str) -> Self {
        Self {
            id: entry_id(provider_id, model_name),
            provider_id: provider_id.to_string(),
            model_name: model_name.to_string(),
            enabled: true,
            last_probe: None,
        }
    }
}

fn default_enabled() -> bool {
    true
}

/// 注册表文件顶层结构。解析失败时退化为空表，绝不覆盖用户既有内容之外的东西。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRegistryFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ModelEntry>,
}

fn default_version() -> u32 {
    REGISTRY_VERSION
}

impl Default for ModelRegistryFile {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            entries: Vec::new(),
        }
    }
}

fn entry_id(provider_id: &str, model_name: &str) -> String {
    format!("{provider_id}::{model_name}")
}

fn registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join(REGISTRY_FILE)
}

/// 读取注册表。文件不存在或内容损坏都返回空表并告警，调用方无需处理 IO 错误。
pub fn load(config_dir: &Path) -> ModelRegistryFile {
    let path = registry_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ModelRegistryFile::default(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "读取模型注册表失败，按空表处理");
            return ModelRegistryFile::default();
        }
    };
    match serde_json::from_str(&raw) {
        Ok(registry) => registry,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "模型注册表不是合法 JSON，按空表处理");
            ModelRegistryFile::default()
        }
    }
}

/// 持久化注册表（原子写：pid 临时文件 → 原文件备份 → rename → 失败回滚）。
pub fn save(config_dir: &Path, registry: &ModelRegistryFile) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let content = serde_json::to_string_pretty(registry).context("序列化模型注册表失败")?;
    atomic_write(&registry_path(config_dir), &content)
}

/// 把本次上游返回的模型并入既有注册表。
///
/// - 已存在的条目保留 `enabled` 与 `last_probe`；
/// - 新模型默认启用、未探测；
/// - 上游本次没返回的旧条目保留在文件里（上游抖动不该抹掉用户的禁用状态），
///   只是不参与本次列表返回（见 [`entries_for_provider`]）。
pub fn merge_entries(
    existing: &ModelRegistryFile,
    provider_id: &str,
    upstream_models: &[String],
) -> ModelRegistryFile {
    let mut registry = existing.clone();
    registry.version = REGISTRY_VERSION;
    for model_name in upstream_models {
        let known = registry
            .entries
            .iter()
            .any(|entry| entry.provider_id == provider_id && entry.model_name == *model_name);
        if !known {
            registry.entries.push(ModelEntry::new(provider_id, model_name));
        }
    }
    registry
}

/// 按上游返回顺序取出本次要展示的条目；顺序跟随 `upstream_models`，重复名只保留一次。
pub fn entries_for_provider(
    registry: &ModelRegistryFile,
    provider_id: &str,
    upstream_models: &[String],
) -> Vec<ModelEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for model_name in upstream_models {
        if !seen.insert(model_name.as_str()) {
            continue;
        }
        if let Some(entry) = registry
            .entries
            .iter()
            .find(|entry| entry.provider_id == provider_id && entry.model_name == *model_name)
        {
            out.push(entry.clone());
        }
    }
    out
}

/// 设置单个模型的启用状态；条目不存在时补一条（保留传入的 enabled，未探测）。
pub fn set_enabled(
    registry: &mut ModelRegistryFile,
    provider_id: &str,
    model_name: &str,
    enabled: bool,
) {
    if let Some(entry) = registry
        .entries
        .iter_mut()
        .find(|entry| entry.provider_id == provider_id && entry.model_name == model_name)
    {
        entry.enabled = enabled;
        return;
    }
    registry.entries.push(ModelEntry {
        enabled,
        ..ModelEntry::new(provider_id, model_name)
    });
}

/// 写入一次探测结果；条目不存在时补一条默认启用的条目。
pub fn apply_probe(
    registry: &mut ModelRegistryFile,
    provider_id: &str,
    model_name: &str,
    probe: ModelProbe,
) {
    if let Some(entry) = registry
        .entries
        .iter_mut()
        .find(|entry| entry.provider_id == provider_id && entry.model_name == model_name)
    {
        entry.last_probe = Some(probe);
        return;
    }
    registry.entries.push(ModelEntry {
        last_probe: Some(probe),
        ..ModelEntry::new(provider_id, model_name)
    });
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
/// 与 `codex_config.rs::atomic_write` 同一做法，避免 Windows rename 不能覆盖导致丢文件。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("model-registry");
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
        .unwrap_or("model-registry");
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
            "sugt-model-registry-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn probe(ok: bool, latency_ms: u64) -> ModelProbe {
        ModelProbe {
            ok,
            latency_ms,
            tokens_per_sec: if ok { 4.0 } else { 0.0 },
            completion_tokens: if ok { 8 } else { 0 },
            estimated: false,
            probed_at: Utc::now(),
            error: if ok { None } else { Some("boom".to_string()) },
            chat_only: false,
        }
    }

    #[test]
    fn new_models_default_to_enabled_and_unprobed() {
        let merged = merge_entries(
            &ModelRegistryFile::default(),
            "prov",
            &["a".to_string(), "b".to_string()],
        );
        let entries = entries_for_provider(&merged, "prov", &["a".to_string(), "b".to_string()]);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.enabled));
        assert!(entries.iter().all(|entry| entry.last_probe.is_none()));
        assert_eq!(entries[0].id, "prov::a");
        assert_eq!(entries[1].id, "prov::b");
    }

    #[test]
    fn disabled_state_and_probe_survive_remerge() {
        let upstream = vec!["a".to_string(), "b".to_string()];
        let mut registry = merge_entries(&ModelRegistryFile::default(), "prov", &upstream);
        let entry = registry
            .entries
            .iter_mut()
            .find(|entry| entry.model_name == "a")
            .expect("entry a");
        entry.enabled = false;
        apply_probe(&mut registry, "prov", "a", probe(true, 120));

        let remerged = merge_entries(&registry, "prov", &upstream);
        let entries = entries_for_provider(&remerged, "prov", &upstream);
        let a = entries.iter().find(|entry| entry.model_name == "a").unwrap();
        assert!(!a.enabled);
        assert_eq!(a.last_probe.as_ref().map(|p| p.latency_ms), Some(120));
        assert!(entries
            .iter()
            .find(|entry| entry.model_name == "b")
            .unwrap()
            .enabled);

        // 上游暂时没返回 b，旧条目留在文件里但不参与本次列表。
        let shrunk = vec!["a".to_string()];
        let remerged = merge_entries(&remerged, "prov", &shrunk);
        assert!(remerged.entries.iter().any(|entry| entry.model_name == "b"));
        assert_eq!(entries_for_provider(&remerged, "prov", &shrunk).len(), 1);
    }

    #[test]
    fn apply_probe_persists_to_disk() {
        let dir = temp_dir("probe-persist");
        let mut registry = load(&dir);
        apply_probe(&mut registry, "prov", "a", probe(true, 88));
        save(&dir, &registry).expect("save");

        let loaded = load(&dir);
        let entries = entries_for_provider(&loaded, "prov", &["a".to_string()]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].enabled);
        let last = entries[0].last_probe.as_ref().expect("probe");
        assert!(last.ok);
        assert_eq!(last.latency_ms, 88);
        assert_eq!(last.completion_tokens, 8);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_empty_registry() {
        let dir = temp_dir("corrupt");
        std::fs::write(registry_path(&dir), "{not valid json").expect("write corrupt file");
        let registry = load(&dir);
        assert!(registry.entries.is_empty());
        assert_eq!(registry.version, REGISTRY_VERSION);

        let merged = merge_entries(&registry, "prov", &["a".to_string()]);
        assert_eq!(entries_for_provider(&merged, "prov", &["a".to_string()]).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        assert!(load(&dir).entries.is_empty());

        let mut registry = load(&dir);
        set_enabled(&mut registry, "prov", "a", false);
        apply_probe(&mut registry, "prov", "a", probe(false, 40));
        apply_probe(&mut registry, "other", "b", probe(true, 200));
        save(&dir, &registry).expect("save");

        let loaded = load(&dir);
        assert_eq!(loaded.version, REGISTRY_VERSION);
        assert_eq!(loaded.entries.len(), 2);
        let a = loaded
            .entries
            .iter()
            .find(|entry| entry.id == "prov::a")
            .expect("prov::a");
        assert!(!a.enabled);
        assert_eq!(
            a.last_probe.as_ref().and_then(|probe| probe.error.clone()),
            Some("boom".to_string())
        );
        let b = loaded
            .entries
            .iter()
            .find(|entry| entry.id == "other::b")
            .expect("other::b");
        assert!(b.last_probe.as_ref().unwrap().ok);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
