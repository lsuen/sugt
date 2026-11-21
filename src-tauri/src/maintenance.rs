//! 维护类操作（PRD v1.1.11 R7）：清理缓存 + 日志保留期。
//!
//! 只清理可重建的缓存，绝不碰用户配置（provider / theme / pet / 技能仓元数据）。

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

/// 一次缓存清理的结果，前端按 camelCase 读取。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheReport {
    /// 删除的顶层条目数 + 被重置的探测结果条数
    pub items: usize,
    /// 删除文件的字节总和
    pub bytes: u64,
}

/// 清理已知缓存：
/// 1. `config_dir/store/cache` 下所有子项（清空后重建目录本身）；
/// 2. `model-registry.json` 里每条 `ModelEntry.last_probe`（保留 `enabled`）。
pub fn clear_cache(config_dir: &Path) -> Result<CacheReport> {
    let mut items = 0usize;
    let mut bytes = 0u64;

    let cache_dir = config_dir.join("store").join("cache");
    if cache_dir.is_dir() {
        for entry in std::fs::read_dir(&cache_dir)
            .with_context(|| format!("无法读取缓存目录 {}", cache_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            bytes += measure_bytes(&path);
            let is_dir = entry.metadata().map(|meta| meta.is_dir()).unwrap_or(false);
            let result = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => items += 1,
                Err(err) => {
                    tracing::warn!(error = %err, path = %path.display(), "删除缓存条目失败");
                }
            }
        }
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("无法重建缓存目录 {}", cache_dir.display()))?;
    }

    let mut registry = crate::model_registry::load(config_dir);
    let mut reset = 0usize;
    for entry in &mut registry.entries {
        if entry.last_probe.is_some() {
            entry.last_probe = None;
            reset += 1;
        }
    }
    if reset > 0 {
        crate::model_registry::save(config_dir, &registry)?;
    }
    items += reset;

    Ok(CacheReport { items, bytes })
}

fn measure_bytes(path: &Path) -> u64 {
    if path.is_dir() {
        return std::fs::read_dir(path)
            .map(|entries| entries.flatten().map(|entry| measure_bytes(&entry.path())).sum())
            .unwrap_or(0);
    }
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sugt-maintenance-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn clear_cache_clears_store_cache_and_resets_probes_only() {
        let dir = temp_dir("clear");
        let repo = dir.join("store").join("cache").join("repo-a");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("file.bin"), b"0123456789").unwrap();

        let mut registry = crate::model_registry::ModelRegistryFile::default();
        registry.entries.push(crate::model_registry::ModelEntry {
            id: "prov::a".to_string(),
            provider_id: "prov".to_string(),
            model_name: "a".to_string(),
            enabled: false,
            last_probe: Some(crate::model_registry::ModelProbe {
                ok: true,
                latency_ms: 42,
                tokens_per_sec: 3.5,
                completion_tokens: 8,
                estimated: false,
                probed_at: chrono::Utc::now(),
                error: None,
            }),
        });
        crate::model_registry::save(&dir, &registry).unwrap();

        let report = clear_cache(&dir).expect("clear cache");
        assert_eq!(report.items, 2, "1 个缓存目录 + 1 条探测结果");
        assert_eq!(report.bytes, 10);
        assert!(!repo.exists(), "缓存子项应被删除");
        assert!(dir.join("store").join("cache").is_dir(), "缓存目录本身必须重建");

        let loaded = crate::model_registry::load(&dir);
        let entry = loaded.entries.first().expect("entry");
        assert!(entry.last_probe.is_none(), "探测结果应清空");
        assert!(!entry.enabled, "启用状态必须保留");

        // 二次清理应幂等且无事可做。
        let again = clear_cache(&dir).expect("clear cache again");
        assert_eq!((again.items, again.bytes), (0, 0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_cache_tolerates_missing_cache_dir() {
        let dir = temp_dir("missing");
        let report = clear_cache(&dir).expect("clear cache");
        assert_eq!((report.items, report.bytes), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
