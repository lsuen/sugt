//! 实验免费通道：探测判定缓存（与真实调用观察分开）。
//!
//! - 成功 / 仅对话：较长 TTL，避免每次打开都打满公开源
//! - 瞬时失败（超时、限流、5xx、连不上）：短 TTL，几分钟后再试
//! - 硬拒绝（鉴权、模型不存在、明确 400）：中等 TTL
//!
//! 能力指纹：`base_url|model`。路由表改绑后旧结论自动作废。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::experimental_free::{FreeModelProbe, ProbeCategory, ProbeSource};

pub const CACHE_FILE: &str = "experimental-free-probe-cache.json";
const VERSION: u32 = 1;

pub const OK_TTL: Duration = Duration::from_secs(6 * 60 * 60);
pub const TRANSIENT_TTL: Duration = Duration::from_secs(10 * 60);
pub const REJECT_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: u32,
    entries: HashMap<String, CachedVerdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedVerdict {
    pub available: bool,
    pub chat_only: bool,
    pub category: ProbeCategory,
    pub latency_ms: Option<u64>,
    pub tokens_per_sec: Option<f64>,
    pub fingerprint: String,
    /// Unix 毫秒
    pub at_ms: u64,
}

pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CACHE_FILE)
}

pub fn route_fingerprint(base_url: &str, model: &str) -> String {
    format!("{}|{}", base_url.trim_end_matches('/'), model.trim())
}

pub fn route_id(model: &str) -> String {
    model.trim().to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn ttl_for(category: ProbeCategory) -> Duration {
    match category {
        ProbeCategory::Ok | ProbeCategory::ChatOnly => OK_TTL,
        ProbeCategory::Transient => TRANSIENT_TTL,
        ProbeCategory::Reject => REJECT_TTL,
    }
}

pub fn load(path: &Path) -> HashMap<String, CachedVerdict> {
    let Ok(raw) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(doc) = serde_json::from_str::<CacheFile>(&raw) else {
        return HashMap::new();
    };
    if doc.version != VERSION {
        return HashMap::new();
    }
    doc.entries
}

pub fn save(path: &Path, entries: &HashMap<String, CachedVerdict>) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let doc = CacheFile {
        version: VERSION,
        entries: entries.clone(),
    };
    let Ok(raw) = serde_json::to_string_pretty(&doc) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, raw).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// 新鲜且指纹一致才命中；否则当作没有。
pub fn fresh<'a>(
    entries: &'a HashMap<String, CachedVerdict>,
    model: &str,
    fingerprint: &str,
) -> Option<&'a CachedVerdict> {
    let verdict = entries.get(&route_id(model))?;
    if verdict.fingerprint != fingerprint {
        return None;
    }
    let age = now_ms().saturating_sub(verdict.at_ms);
    if age >= ttl_for(verdict.category).as_millis() as u64 {
        return None;
    }
    Some(verdict)
}

pub fn record(
    entries: &mut HashMap<String, CachedVerdict>,
    model: &str,
    fingerprint: String,
    probe: &FreeModelProbe,
) {
    let category = probe.category.unwrap_or(if probe.available {
        if probe.chat_only {
            ProbeCategory::ChatOnly
        } else {
            ProbeCategory::Ok
        }
    } else {
        ProbeCategory::Reject
    });
    entries.insert(
        route_id(model),
        CachedVerdict {
            available: probe.available,
            chat_only: probe.chat_only,
            category,
            latency_ms: probe.latency_ms,
            tokens_per_sec: probe.tokens_per_sec,
            fingerprint,
            at_ms: now_ms(),
        },
    );
}

pub fn to_probe(model: &str, verdict: &CachedVerdict) -> FreeModelProbe {
    FreeModelProbe {
        name: model.to_string(),
        available: verdict.available,
        latency_ms: verdict.latency_ms,
        tokens_per_sec: verdict.tokens_per_sec,
        chat_only: verdict.chat_only,
        source: ProbeSource::Cache,
        category: Some(verdict.category),
    }
}

/// 按当前 ROUTES 丢掉已不存在的模型条目，避免缓存无限长大。
pub fn prune(entries: &mut HashMap<String, CachedVerdict>, alive_models: &[&str]) -> usize {
    let alive: std::collections::HashSet<&str> =
        alive_models.iter().copied().map(str::trim).collect();
    let before = entries.len();
    entries.retain(|id, _| alive.contains(id.as_str()));
    before.saturating_sub(entries.len())
}

/// 从错误文案归类瞬时 / 硬拒绝（对齐 error_hint 输出与 HTTP 语义）。
pub fn classify_failure(error: Option<&str>) -> ProbeCategory {
    let text = error.unwrap_or("").to_lowercase();
    if text.contains("超时")
        || text.contains("timeout")
        || text.contains("429")
        || text.contains("限流")
        || text.contains("rate limit")
        || text.contains("无法连接")
        || text.contains("connect")
        || text.contains("5xx")
        || text.contains("http 5")
        || text.contains("上游服务异常")
        || text.contains("网络请求失败")
    {
        return ProbeCategory::Transient;
    }
    ProbeCategory::Reject
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experimental_free::{FreeModelProbe, ProbeCategory, ProbeSource};

    #[test]
    fn fresh_respects_ttl_and_fingerprint() {
        let mut entries = HashMap::new();
        let probe = FreeModelProbe {
            name: "m".into(),
            available: true,
            latency_ms: Some(10),
            tokens_per_sec: Some(1.0),
            chat_only: false,
            source: ProbeSource::Live,
            category: Some(ProbeCategory::Ok),
        };
        record(&mut entries, "m", "fp-a".into(), &probe);
        assert!(fresh(&entries, "m", "fp-a").is_some());
        assert!(fresh(&entries, "m", "fp-b").is_none());
    }

    #[test]
    fn classify_timeout_as_transient() {
        assert_eq!(
            classify_failure(Some("连接超时：请检查网络")),
            ProbeCategory::Transient
        );
        assert_eq!(
            classify_failure(Some("请求过于频繁 (429)：已被上游限流")),
            ProbeCategory::Transient
        );
        assert_eq!(
            classify_failure(Some("模型不存在 (404)")),
            ProbeCategory::Reject
        );
    }

    #[test]
    fn roundtrip_file() {
        let dir = std::env::temp_dir().join(format!(
            "sugt-probe-cache-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = cache_path(&dir);
        let mut entries = HashMap::new();
        let probe = FreeModelProbe {
            name: "kilo-auto/free".into(),
            available: true,
            latency_ms: Some(100),
            tokens_per_sec: Some(20.0),
            chat_only: true,
            source: ProbeSource::Live,
            category: Some(ProbeCategory::ChatOnly),
        };
        record(
            &mut entries,
            "kilo-auto/free",
            route_fingerprint("https://api.kilo.ai/api/gateway", "kilo-auto/free"),
            &probe,
        );
        save(&path, &entries);
        let loaded = load(&path);
        let hit = fresh(
            &loaded,
            "kilo-auto/free",
            &route_fingerprint("https://api.kilo.ai/api/gateway", "kilo-auto/free"),
        )
        .expect("fresh");
        assert!(hit.available);
        assert!(hit.chat_only);
        let _ = fs::remove_dir_all(&dir);
    }
}
