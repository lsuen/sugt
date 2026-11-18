//! 策略盒子：多厂商成员的聚合编排配置（PRD v1.1.11 R3 / T3.1）
//!
//! 一个盒子 = 若干成员（provider + model）+ 一套选路策略。配置落在
//! `config_dir/strategy-boxes.json`，运行时健康度只留在内存里、不落盘。
//! 文件结构与所有前端结构统一使用 camelCase：`{ "version": 1, "boxes": [...] }`。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{ProviderConfig, CLIENT_MODEL_ID};

/// 策略盒子文件名（位于 config_dir 下）。
const BOX_FILE: &str = "strategy-boxes.json";
const BOX_VERSION: u32 = 1;
/// 一条 attempt 链最多 8 个成员，避免一次请求打穿所有上游。
pub const MAX_CHAIN: usize = 8;

/// 选路模式。落盘值与前端约定为 snake_case：priority / weighted / round_robin / speed_threshold。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StrategyMode {
    /// 按优先级，数字小优先（1 最高），整条链保留作为降级后备。
    #[default]
    Priority,
    /// 按权重无放回加权抽样得到完整顺序。
    Weighted,
    /// 顺序轮换。
    RoundRobin,
    /// 按最近一次延迟切换，慢成员排到链尾。
    SpeedThreshold,
}

fn default_weight() -> u32 {
    1
}

fn default_priority() -> u32 {
    1
}

fn default_enabled() -> bool {
    true
}

fn default_version() -> u32 {
    BOX_VERSION
}

/// 盒子成员：所属 provider、模型名、权重与优先级。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StrategyMember {
    pub provider_id: String,
    pub model_name: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default = "default_priority")]
    pub priority: u32,
}

/// 一个策略盒子：多个成员 + 一套路由策略，对外暴露一个统一 model id。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StrategyBox {
    /// 新建时为空，由后端生成 uuid。
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub public_model_id: String,
    #[serde(default)]
    pub members: Vec<StrategyMember>,
    #[serde(default)]
    pub mode: StrategyMode,
    #[serde(default)]
    pub speed_threshold_ms: Option<u64>,
    #[serde(default)]
    pub cooldown_ms: Option<u64>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// 文件顶层结构。解析失败时退化为空表，绝不 panic。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StrategyBoxFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    boxes: Vec<StrategyBox>,
}

impl Default for StrategyBoxFile {
    fn default() -> Self {
        Self {
            version: BOX_VERSION,
            boxes: Vec::new(),
        }
    }
}

fn boxes_path(config_dir: &Path) -> PathBuf {
    config_dir.join(BOX_FILE)
}

/// 读取策略盒子。文件不存在或内容损坏都返回空表并告警，调用方无需处理 IO 错误。
pub fn load_boxes(config_dir: &Path) -> Vec<StrategyBox> {
    let path = boxes_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "读取策略盒子失败，按空表处理");
            return Vec::new();
        }
    };
    match serde_json::from_str::<StrategyBoxFile>(&raw) {
        Ok(file) => file.boxes,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "策略盒子不是合法 JSON，按空表处理");
            Vec::new()
        }
    }
}

/// 持久化策略盒子（原子写：pid 临时文件 → 原文件备份 → rename → 失败回滚）。
pub fn save_boxes(config_dir: &Path, boxes: &[StrategyBox]) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let file = StrategyBoxFile {
        version: BOX_VERSION,
        boxes: boxes.to_vec(),
    };
    let content = serde_json::to_string_pretty(&file).context("序列化策略盒子失败")?;
    atomic_write(&boxes_path(config_dir), &content)
}

/// 保存前校验：名称/对外 id 非空、对外 id 不与既有 provider 冲突、成员至少一个、
/// 成员 provider 必须存在、模式相关阈值合法、同一盒子里成员不可重复。
pub fn validate(spec: &StrategyBox, providers: &[ProviderConfig]) -> Result<(), String> {
    if spec.name.trim().is_empty() {
        return Err("策略盒子名称不能为空".to_string());
    }
    let public_id = spec.public_model_id.trim();
    if public_id.is_empty() {
        return Err("对外模型 ID 不能为空".to_string());
    }
    if public_id == CLIENT_MODEL_ID {
        return Err(format!("对外模型 ID 不能使用保留名称 {CLIENT_MODEL_ID}"));
    }
    if let Some(provider) = providers
        .iter()
        .find(|provider| provider.public_model_id().trim() == public_id)
    {
        return Err(format!(
            "对外模型 ID 与服务商「{}」冲突（{}）",
            provider.name, public_id
        ));
    }
    if spec.members.is_empty() {
        return Err("策略盒子至少需要一个成员".to_string());
    }
    for member in &spec.members {
        if !providers.iter().any(|provider| provider.id == member.provider_id) {
            return Err(format!("成员引用了不存在的服务商: {}", member.provider_id));
        }
    }
    if spec.mode == StrategyMode::SpeedThreshold
        && spec.speed_threshold_ms.unwrap_or(0) == 0
    {
        return Err("speed_threshold 模式必须设置大于 0 的延迟阈值".to_string());
    }
    if spec.mode == StrategyMode::Weighted
        && spec.members.iter().map(|member| member.weight as u64).sum::<u64>() == 0
    {
        return Err("weighted 模式的总权重必须大于 0".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for member in &spec.members {
        if !seen.insert((member.provider_id.as_str(), member.model_name.as_str())) {
            return Err(format!(
                "同一策略盒子内成员重复: {} / {}",
                member.provider_id, member.model_name
            ));
        }
    }
    Ok(())
}

/// 对外模型 ID 与其他盒子冲突时拒绝保存（与自身比较不算冲突）。
pub fn check_public_id_conflict(boxes: &[StrategyBox], spec: &StrategyBox) -> Result<(), String> {
    let public_id = spec.public_model_id.trim();
    if let Some(other) = boxes
        .iter()
        .find(|other| other.id != spec.id && other.public_model_id.trim() == public_id)
    {
        return Err(format!(
            "对外模型 ID 已被策略盒子「{}」占用（{}）",
            other.name, public_id
        ));
    }
    Ok(())
}

/// 插入或替换：id 为空则生成 uuid，id 已存在则整体替换，同时保证对外 id 全局唯一。
pub fn upsert(boxes: &mut Vec<StrategyBox>, mut spec: StrategyBox) -> StrategyBox {
    spec.id = spec.id.trim().to_string();
    if spec.id.is_empty() {
        spec.id = uuid::Uuid::new_v4().to_string();
    }
    spec.name = spec.name.trim().to_string();
    spec.public_model_id = spec.public_model_id.trim().to_string();
    // 兜底：即使调用方漏了 check_public_id_conflict，也不让两个盒子共享对外 id。
    // 只改本次保存的盒子（追加 -2 / -3 后缀），绝不动其他盒子。
    if boxes
        .iter()
        .any(|other| other.id != spec.id && other.public_model_id == spec.public_model_id)
    {
        let base = spec.public_model_id.clone();
        let mut suffix = 2;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !boxes
                .iter()
                .any(|other| other.id != spec.id && other.public_model_id == candidate)
            {
                spec.public_model_id = candidate;
                break;
            }
            suffix += 1;
        }
    }
    if let Some(existing) = boxes.iter_mut().find(|existing| existing.id == spec.id) {
        *existing = spec.clone();
    } else {
        boxes.push(spec.clone());
    }
    spec
}

/// 成员健康度：只留在内存，不落盘。
#[derive(Debug, Clone, Default)]
pub struct MemberHealth {
    pub last_latency_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub cooldown_until: Option<Instant>,
    pub last_error: Option<String>,
}

/// 进程内运行态：round_robin 游标 + 成员健康度，键为 [`member_key`]。
#[derive(Debug, Clone, Default)]
pub struct StrategyRuntime {
    pub rotation: HashMap<String, u64>,
    pub health: HashMap<String, MemberHealth>,
}

impl StrategyRuntime {
    /// 推进并返回某个盒子的轮换游标。
    pub fn next_rotation(&mut self, box_id: &str) -> u64 {
        let counter = self.rotation.entry(box_id.to_string()).or_insert(0);
        *counter = counter.wrapping_add(1);
        *counter
    }

    /// 记录一次成功：延迟覆盖、失败计数归零、冷却与错误清空。
    pub fn record_success(&mut self, key: &str, latency_ms: u64) {
        let entry = self.health.entry(key.to_string()).or_default();
        entry.last_latency_ms = Some(latency_ms);
        entry.consecutive_failures = 0;
        entry.cooldown_until = None;
        entry.last_error = None;
    }

    /// 记录一次失败：失败计数 +1，配置了冷却则进入冷却期。
    pub fn record_failure(&mut self, key: &str, error: &str, cooldown_ms: Option<u64>) {
        let entry = self.health.entry(key.to_string()).or_default();
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        entry.last_error = Some(error.to_string());
        if let Some(ms) = cooldown_ms {
            entry.cooldown_until = Some(Instant::now() + Duration::from_millis(ms.max(1)));
        }
    }

    /// 读取成员健康度；没有记录时返回默认值。
    pub fn status(&self, member_key: &str) -> MemberHealth {
        self.health.get(member_key).cloned().unwrap_or_default()
    }
}

/// 网关内存中的策略盒子状态：配置快照 + 进程内运行态。
#[derive(Debug, Clone, Default)]
pub struct StrategyState {
    pub boxes: Vec<StrategyBox>,
    pub runtime: StrategyRuntime,
}

/// 健康度键：`{box_id}::{provider_id}::{model_name}`。
pub fn member_key(box_id: &str, member: &StrategyMember) -> String {
    format!("{box_id}::{}::{}", member.provider_id, member.model_name)
}

/// 把成员排成一条 attempt 链（纯函数，不碰网络与全局状态）。
///
/// `health` 是整张健康度表，键为 [`member_key`]；按 `box_id` 精确取本盒子的记录。
pub fn order_members(
    box_id: &str,
    members: &[StrategyMember],
    mode: StrategyMode,
    speed_threshold_ms: Option<u64>,
    health: &HashMap<String, MemberHealth>,
    rotation: u64,
) -> Vec<StrategyMember> {
    let ordered = match mode {
        StrategyMode::Priority => by_priority(members),
        StrategyMode::RoundRobin => rotate(members, rotation),
        StrategyMode::Weighted => weighted_sample(members, rotation),
        StrategyMode::SpeedThreshold => by_speed(box_id, members, speed_threshold_ms, health),
    };
    ordered.into_iter().take(MAX_CHAIN).collect()
}

fn by_priority(members: &[StrategyMember]) -> Vec<StrategyMember> {
    let mut ordered = members.to_vec();
    // 稳定排序：优先级相同时保持配置顺序。
    ordered.sort_by_key(|member| member.priority);
    ordered
}

fn rotate(members: &[StrategyMember], rotation: u64) -> Vec<StrategyMember> {
    if members.is_empty() {
        return Vec::new();
    }
    let len = members.len();
    let start = (rotation % len as u64) as usize;
    members[start..]
        .iter()
        .chain(members[..start].iter())
        .cloned()
        .collect()
}

fn weighted_sample(members: &[StrategyMember], rotation: u64) -> Vec<StrategyMember> {
    let mut remaining: Vec<StrategyMember> = members
        .iter()
        .filter(|member| member.weight > 0)
        .cloned()
        .collect();
    let mut state = rotation;
    let mut out = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let total: u64 = remaining
            .iter()
            .map(|member| u64::from(member.weight))
            .sum();
        // 线性同余：同一个 rotation 得到同一个顺序，便于单测。
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut pick = state % total;
        let mut index = 0;
        for (i, member) in remaining.iter().enumerate() {
            let weight = u64::from(member.weight);
            if pick < weight {
                index = i;
                break;
            }
            pick -= weight;
        }
        out.push(remaining.remove(index));
    }
    out
}

fn by_speed(
    box_id: &str,
    members: &[StrategyMember],
    threshold: Option<u64>,
    health: &HashMap<String, MemberHealth>,
) -> Vec<StrategyMember> {
    let now = Instant::now();
    let limit = threshold.unwrap_or(u64::MAX);
    let mut healthy: Vec<(Option<u64>, StrategyMember)> = Vec::new();
    let mut slow: Vec<StrategyMember> = Vec::new();
    for member in members {
        let record = health.get(&member_key(box_id, member));
        let cooling = record
            .and_then(|record| record.cooldown_until)
            .map(|until| until > now)
            .unwrap_or(false);
        let over_threshold = record
            .and_then(|record| record.last_latency_ms)
            .map(|latency| latency > limit)
            .unwrap_or(false);
        if cooling || over_threshold {
            slow.push(member.clone());
        } else {
            healthy.push((record.and_then(|record| record.last_latency_ms), member.clone()));
        }
    }
    if healthy.is_empty() {
        // 没有健康成员：退化为 priority 顺序，不整条拒绝。
        return by_priority(members);
    }
    healthy.sort_by_key(|(latency, _)| latency.unwrap_or(u64::MAX));
    let mut ordered: Vec<StrategyMember> = healthy
        .into_iter()
        .map(|(_, member)| member)
        .collect();
    ordered.extend(by_priority(&slow));
    ordered
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
/// 与 `model_registry.rs::atomic_write` 同一做法，避免 Windows rename 不能覆盖导致丢文件。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("strategy-boxes");
    let tmp = path.with_file_name(format!("{name}.sugt-tmp"));
    std::fs::write(&tmp, content)
        .with_context(|| format!("无法写入临时文件 {}", tmp.display()))?;
    match replace_with_backup(path, &tmp) {
        Ok(()) => Ok(()),
        Err(err) => {
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
        .unwrap_or("strategy-boxes");
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
    use std::collections::HashSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sugt-strategy-box-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn provider(id: &str, alias: Option<&str>) -> ProviderConfig {
        let mut config = ProviderConfig::new(
            format!("厂商 {id}"),
            "openai",
            "https://example.com/v1",
            "sk-test",
            "upstream-model",
        );
        config.id = id.to_string();
        config.model_alias = alias.map(|alias| alias.to_string());
        config
    }

    fn member(provider_id: &str, model_name: &str, weight: u32, priority: u32) -> StrategyMember {
        StrategyMember {
            provider_id: provider_id.to_string(),
            model_name: model_name.to_string(),
            weight,
            priority,
        }
    }

    fn spec(id: &str, public_id: &str, members: Vec<StrategyMember>) -> StrategyBox {
        StrategyBox {
            id: id.to_string(),
            name: "盒子".to_string(),
            public_model_id: public_id.to_string(),
            members,
            mode: StrategyMode::Priority,
            speed_threshold_ms: None,
            cooldown_ms: None,
            enabled: true,
        }
    }

    fn model_names(ordered: &[StrategyMember]) -> Vec<String> {
        ordered.iter().map(|m| m.model_name.clone()).collect()
    }

    #[test]
    fn priority_orders_by_ascending_number() {
        let members = vec![
            member("p1", "c", 1, 3),
            member("p2", "a", 1, 1),
            member("p3", "b", 1, 2),
        ];
        let ordered = order_members("box", &members, StrategyMode::Priority, None, &HashMap::new(), 0);
        assert_eq!(model_names(&ordered), vec!["a", "b", "c"]);
    }

    #[test]
    fn round_robin_starts_at_rotation() {
        let members = vec![
            member("p1", "a", 1, 1),
            member("p2", "b", 1, 2),
            member("p3", "c", 1, 3),
        ];
        let empty = HashMap::new();
        let ordered = order_members("box", &members, StrategyMode::RoundRobin, None, &empty, 1);
        assert_eq!(model_names(&ordered), vec!["b", "c", "a"]);
        let wrapped = order_members("box", &members, StrategyMode::RoundRobin, None, &empty, 3);
        assert_eq!(model_names(&wrapped), vec!["a", "b", "c"]);
        assert!(order_members("box", &[], StrategyMode::RoundRobin, None, &empty, 7).is_empty());
    }

    #[test]
    fn weighted_excludes_zero_and_varies_with_rotation() {
        let members = vec![
            member("p1", "a", 3, 1),
            member("p2", "b", 1, 2),
            member("p3", "zero", 0, 3),
        ];
        let empty = HashMap::new();
        let first = order_members("box", &members, StrategyMode::Weighted, None, &empty, 0);
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|m| m.model_name != "zero"));
        let orders: HashSet<Vec<String>> = (0..12)
            .map(|rotation| model_names(&order_members("box", &members, StrategyMode::Weighted, None, &empty, rotation)))
            .collect();
        assert!(orders.len() > 1, "不同 rotation 应得到不同顺序: {orders:?}");
    }

    #[test]
    fn speed_threshold_pushes_slow_members_to_tail() {
        let members = vec![
            member("p1", "fast", 1, 1),
            member("p2", "slow", 1, 2),
            member("p3", "unknown", 1, 3),
        ];
        let mut health = HashMap::new();
        health.insert(
            "box::p1::fast".to_string(),
            MemberHealth {
                last_latency_ms: Some(100),
                ..MemberHealth::default()
            },
        );
        health.insert(
            "box::p2::slow".to_string(),
            MemberHealth {
                last_latency_ms: Some(9000),
                consecutive_failures: 2,
                cooldown_until: Some(Instant::now() + Duration::from_secs(30)),
                ..MemberHealth::default()
            },
        );
        let ordered = order_members("box", &members, StrategyMode::SpeedThreshold, Some(2000), &health, 0);
        assert_eq!(model_names(&ordered), vec!["fast", "unknown", "slow"]);
    }

    #[test]
    fn speed_threshold_all_slow_falls_back_to_priority() {
        let members = vec![member("p1", "b", 1, 2), member("p2", "a", 1, 1)];
        let mut health = HashMap::new();
        health.insert(
            "box::p1::b".to_string(),
            MemberHealth {
                last_latency_ms: Some(9000),
                ..MemberHealth::default()
            },
        );
        health.insert(
            "box::p2::a".to_string(),
            MemberHealth {
                last_latency_ms: Some(9000),
                ..MemberHealth::default()
            },
        );
        let ordered = order_members("box", &members, StrategyMode::SpeedThreshold, Some(1000), &health, 0);
        assert_eq!(model_names(&ordered), vec!["a", "b"]);
    }

    #[test]
    fn chain_is_truncated_to_eight() {
        let members: Vec<StrategyMember> = (0..10)
            .map(|index| member(&format!("p{index}"), &format!("m{index}"), 1, index))
            .collect();
        let ordered = order_members("box", &members, StrategyMode::Priority, None, &HashMap::new(), 0);
        assert_eq!(ordered.len(), MAX_CHAIN);
        assert_eq!(model_names(&ordered).first().map(String::as_str), Some("m0"));
        assert_eq!(model_names(&ordered).last().map(String::as_str), Some("m7"));
    }

    #[test]
    fn validate_rejects_invalid_specs() {
        let providers = vec![provider("p1", Some("vendor-a")), provider("p2", None)];
        let ok_member = || vec![member("p1", "m1", 1, 1)];

        let mut blank_name = spec("", "box-a", ok_member());
        blank_name.name = "   ".to_string();
        assert!(validate(&blank_name, &providers).is_err());

        let empty_public = spec("", "  ", ok_member());
        assert!(validate(&empty_public, &providers).is_err());

        let reserved = spec("", CLIENT_MODEL_ID, ok_member());
        assert!(validate(&reserved, &providers).is_err());

        let provider_conflict = spec("", "vendor-a", ok_member());
        assert!(validate(&provider_conflict, &providers).is_err());

        let no_members = spec("", "box-a", Vec::new());
        assert!(validate(&no_members, &providers).is_err());

        let unknown_provider = spec("", "box-a", vec![member("missing", "m1", 1, 1)]);
        assert!(validate(&unknown_provider, &providers).is_err());

        let duplicate = spec(
            "",
            "box-a",
            vec![member("p1", "m1", 1, 1), member("p1", "m1", 2, 2)],
        );
        assert!(validate(&duplicate, &providers).is_err());

        let mut speed = spec("", "box-a", ok_member());
        speed.mode = StrategyMode::SpeedThreshold;
        speed.speed_threshold_ms = None;
        assert!(validate(&speed, &providers).is_err());

        let mut weighted = spec("", "box-a", vec![member("p1", "m1", 0, 1)]);
        weighted.mode = StrategyMode::Weighted;
        assert!(validate(&weighted, &providers).is_err());

        assert!(validate(&spec("", "box-a", ok_member()), &providers).is_ok());
    }

    #[test]
    fn upsert_generates_id_and_replaces_existing() {
        let mut boxes = Vec::new();
        let created = upsert(&mut boxes, spec("", "box-a", vec![member("p1", "m1", 1, 1)]));
        assert!(!created.id.is_empty());
        assert_eq!(boxes.len(), 1);

        let mut renamed = created.clone();
        renamed.name = "改名".to_string();
        let replaced = upsert(&mut boxes, renamed);
        assert_eq!(replaced.id, created.id);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].name, "改名");

        let second = upsert(&mut boxes, spec("", "box-b", vec![member("p1", "m2", 1, 1)]));
        assert_ne!(second.id, created.id);
        assert_eq!(boxes.len(), 2);
    }

    #[test]
    fn public_id_conflict_with_other_box_is_rejected() {
        let boxes = vec![spec("b1", "shared", vec![member("p1", "m1", 1, 1)])];
        let conflicting = spec("b2", "shared", vec![member("p1", "m2", 1, 1)]);
        assert!(check_public_id_conflict(&boxes, &conflicting).is_err());
        // 与自身同名不算冲突（编辑保存）。
        let same_box = spec("b1", "shared", vec![member("p1", "m2", 1, 1)]);
        assert!(check_public_id_conflict(&boxes, &same_box).is_ok());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        assert!(load_boxes(&dir).is_empty());

        let mut boxes = Vec::new();
        let mut weighted = spec("", "box-b", vec![member("p1", "m1", 3, 2)]);
        weighted.mode = StrategyMode::Weighted;
        weighted.cooldown_ms = Some(30_000);
        weighted.members[0].weight = 5;
        let mut priority = spec("", "box-a", vec![member("p2", "m2", 1, 1)]);
        priority.id = "box-a-id".to_string();
        upsert(&mut boxes, priority);
        upsert(&mut boxes, weighted);
        save_boxes(&dir, &boxes).expect("save");

        let loaded = load_boxes(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].mode, StrategyMode::Weighted);
        assert_eq!(loaded[1].members[0].weight, 5);
        assert_eq!(loaded[1].members[0].priority, 2);
        assert_eq!(loaded[1].cooldown_ms, Some(30_000));
        assert!(loaded[0].enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_empty_boxes() {
        let dir = temp_dir("corrupt");
        std::fs::write(boxes_path(&dir), "{not valid json").expect("write corrupt file");
        assert!(load_boxes(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_records_rotation_health() {
        let mut runtime = StrategyRuntime::default();
        assert_eq!(runtime.next_rotation("box"), 1);
        assert_eq!(runtime.next_rotation("box"), 2);
        assert_eq!(runtime.next_rotation("other"), 1);

        let key = member_key("box", &member("p1", "m1", 1, 1));
        assert_eq!(key, "box::p1::m1");

        runtime.record_failure(&key, "boom", Some(1_000));
        let failed = runtime.status(&key);
        assert_eq!(failed.consecutive_failures, 1);
        assert_eq!(failed.last_error.as_deref(), Some("boom"));
        assert!(failed.cooldown_until.is_some());

        runtime.record_success(&key, 120);
        let ok = runtime.status(&key);
        assert_eq!(ok.consecutive_failures, 0);
        assert_eq!(ok.last_latency_ms, Some(120));
        assert!(ok.cooldown_until.is_none());
        assert!(runtime.status("box::p9::m9").last_latency_ms.is_none());
    }
}
