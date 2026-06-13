use chrono::{Local, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_CLIENT_ENTRIES: usize = 32;
const BUCKET_MINUTES: i64 = 60;

#[derive(Debug, Clone)]
pub struct RequestRecord {
    pub success: bool,
    pub latency_ms: u64,
    pub user_agent: String,
    pub provider_name: String,
    pub model_name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HourlyBucketView {
    pub success: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientActivityView {
    pub label: String,
    pub provider_name: String,
    pub model_name: String,
    pub request_count: u32,
    pub last_path: String,
    pub last_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrafficStatsView {
    pub total_requests: u64,
    pub today_requests: u64,
    pub success_count: u64,
    pub failed_count: u64,
    pub avg_latency_ms: u64,
    pub fail_rate_percent: f32,
    pub active_clients: u32,
    pub hourly_buckets: Vec<HourlyBucketView>,
    pub recent_clients: Vec<ClientActivityView>,
}

#[derive(Debug, Default)]
struct StatsInner {
    total: u64,
    success: u64,
    failed: u64,
    today_date: String,
    today_count: u64,
    latency_sum_ms: u64,
    latency_count: u64,
    minute_buckets: HashMap<i64, (u32, u32)>,
    clients: HashMap<String, ClientEntry>,
}

#[derive(Debug, Clone)]
struct ClientEntry {
    label: String,
    provider_name: String,
    model_name: String,
    request_count: u32,
    last_path: String,
    last_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct GatewayStatsCollector {
    inner: Arc<RwLock<StatsInner>>,
}

impl GatewayStatsCollector {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StatsInner {
                today_date: today_key(),
                ..StatsInner::default()
            })),
        }
    }

    pub async fn record(&self, record: RequestRecord) {
        let mut inner = self.inner.write().await;
        let today = today_key();
        if inner.today_date != today {
            inner.today_date = today;
            inner.today_count = 0;
        }
        inner.total += 1;
        inner.today_count += 1;
        if record.success {
            inner.success += 1;
        } else {
            inner.failed += 1;
        }
        inner.latency_sum_ms += record.latency_ms;
        inner.latency_count += 1;

        let minute = minute_epoch();
        let bucket = inner.minute_buckets.entry(minute).or_insert((0, 0));
        if record.success {
            bucket.0 += 1;
        } else {
            bucket.1 += 1;
        }
        prune_old_minutes(&mut inner.minute_buckets, minute);

        let label = client_label(&record.user_agent);
        let key = label.clone();
        let entry = inner.clients.entry(key).or_insert_with(|| ClientEntry {
            label,
            provider_name: record.provider_name.clone(),
            model_name: record.model_name.clone(),
            request_count: 0,
            last_path: record.path.clone(),
            last_at: Utc::now(),
        });
        entry.request_count += 1;
        entry.provider_name = record.provider_name;
        entry.model_name = record.model_name;
        entry.last_path = record.path;
        entry.last_at = Utc::now();

        if inner.clients.len() > MAX_CLIENT_ENTRIES {
            let oldest = inner
                .clients
                .iter()
                .min_by_key(|(_, v)| v.last_at)
                .map(|(k, _)| k.clone());
            if let Some(key) = oldest {
                inner.clients.remove(&key);
            }
        }
    }

    pub async fn snapshot(&self) -> TrafficStatsView {
        let inner = self.inner.read().await;
        let total_req = inner.total;
        let fail_rate = if total_req == 0 {
            0.0
        } else {
            (inner.failed as f32 / total_req as f32) * 100.0
        };
        let avg_latency = if inner.latency_count == 0 {
            0
        } else {
            inner.latency_sum_ms / inner.latency_count
        };

        let now_min = minute_epoch();
        let hourly_buckets = (0..BUCKET_MINUTES)
            .map(|offset| {
                let min = now_min - (BUCKET_MINUTES - 1 - offset);
                let (success, failed) = inner.minute_buckets.get(&min).copied().unwrap_or((0, 0));
                HourlyBucketView { success, failed }
            })
            .collect();

        let mut recent: Vec<ClientActivityView> = inner
            .clients
            .values()
            .map(|entry| ClientActivityView {
                label: entry.label.clone(),
                provider_name: entry.provider_name.clone(),
                model_name: entry.model_name.clone(),
                request_count: entry.request_count,
                last_path: entry.last_path.clone(),
                last_at: entry.last_at,
            })
            .collect();
        recent.sort_by(|a, b| b.last_at.cmp(&a.last_at));
        recent.truncate(8);

        TrafficStatsView {
            total_requests: inner.total,
            today_requests: inner.today_count,
            success_count: inner.success,
            failed_count: inner.failed,
            avg_latency_ms: avg_latency,
            fail_rate_percent: fail_rate,
            active_clients: inner.clients.len() as u32,
            hourly_buckets,
            recent_clients: recent,
        }
    }
}

fn today_key() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn minute_epoch() -> i64 {
    Utc::now().timestamp() / 60
}

fn prune_old_minutes(map: &mut HashMap<i64, (u32, u32)>, current: i64) {
    let min_keep = current - BUCKET_MINUTES - 2;
    map.retain(|minute, _| *minute >= min_keep);
}

fn client_label(user_agent: &str) -> String {
    let trimmed = user_agent.trim();
    if trimmed.is_empty() {
        return "本地客户端".to_string();
    }
    let label = trimmed.split_whitespace().next().unwrap_or(trimmed);
    if label.len() > 48 {
        format!("{}…", label.chars().take(48).collect::<String>())
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_success_and_failure() {
        let stats = GatewayStatsCollector::new();
        stats.record(RequestRecord {
            success: true,
            latency_ms: 100,
            user_agent: "claude-cli".into(),
            provider_name: "p".into(),
            model_name: "m".into(),
            path: "v1/chat/completions".into(),
        })
        .await;
        stats.record(RequestRecord {
            success: false,
            latency_ms: 50,
            user_agent: "codex".into(),
            provider_name: "p2".into(),
            model_name: "m2".into(),
            path: "v1/messages".into(),
        })
        .await;
        let snap = stats.snapshot().await;
        assert_eq!(snap.total_requests, 2);
        assert_eq!(snap.success_count, 1);
        assert_eq!(snap.failed_count, 1);
        assert_eq!(snap.active_clients, 2);
    }
}
