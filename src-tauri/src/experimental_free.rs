//! 实验·免费通道：公开零 Key 多源，内置只读，可删可开关。
//! 与用户自定义 provider 隔离——探测/failover 仅作用于本通道。

use crate::model::{AppConfig, ProviderConfig, ProviderProtocol, ProviderStatus};
use futures_util::future::join_all;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const EXPERIMENTAL_FREE_ID: &str = "sugt-free";
pub const EXPERIMENTAL_FREE_PROVIDER: &str = "sugt-free";
/// 旧 Zen 实验源 id，加载时迁移
const LEGACY_ZEN_ID: &str = "sugt-zen-free";

#[derive(Clone, Copy)]
struct FreeRoute {
    base_url: &'static str,
    model: &'static str,
}

/// 候选路由（公开 keyless）。顺序即探测/默认/failover 优先级：
/// Tier 1 主力 → Tier 2 备用 → Tier 3 降级。
const ROUTES: &[FreeRoute] = &[
    // === Tier 1: 主力 ===
    // Kilo.ai — 国产免费模型最多
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "kilo-auto/free",
    },
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "baidu/ernie-5.5-flash-preview:free",
    },
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "nvidia/nemotron-3-ultra-550b-a55b:free",
    },
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free",
    },
    // BlockRun — NVIDIA 免费模型（GPT-6 全家桶需付费 402，排除）
    FreeRoute {
        base_url: "https://blockrun.ai/api",
        model: "nvidia/nemotron-3.5-bolt",
    },
    FreeRoute {
        base_url: "https://blockrun.ai/api",
        model: "nvidia/nemotron-3-ultra-550b",
    },
    FreeRoute {
        base_url: "https://blockrun.ai/api",
        model: "nvidia/muse-glimmer-30b",
    },
    // === Tier 2: 备用 ===
    // LLM7.io — Claude 全系（需 Bearer unused）
    FreeRoute {
        base_url: "https://api.llm7.io",
        model: "default",
    },
    // JarvisClaw — 自动路由兜底
    FreeRoute {
        base_url: "https://api.jarvisclaw.ai",
        model: "auto/free",
    },
    // === Tier 3: 降级（rate limit 极严或已收 key）===
    FreeRoute {
        base_url: "https://text.pollinations.ai/openai",
        model: "openai-fast",
    },
    FreeRoute {
        base_url: "https://text.pollinations.ai/openai",
        model: "openai",
    },
];

pub fn is_experimental_free_id(id: &str) -> bool {
    id.eq_ignore_ascii_case(EXPERIMENTAL_FREE_ID) || id.eq_ignore_ascii_case(LEGACY_ZEN_ID)
}

pub fn is_experimental_free(provider: &ProviderConfig) -> bool {
    is_experimental_free_id(&provider.id)
        || provider
            .provider
            .eq_ignore_ascii_case(EXPERIMENTAL_FREE_PROVIDER)
}

#[derive(Debug, Clone, Serialize)]
pub struct FreeRouteChoice {
    pub model: String,
    pub host: String,
}

pub fn route_choices() -> Vec<FreeRouteChoice> {
    ROUTES
        .iter()
        .map(|route| FreeRouteChoice {
            model: route.model.to_string(),
            host: host_of(route.base_url),
        })
        .collect()
}

fn host_of(base_url: &str) -> String {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

pub fn candidate_models() -> Vec<&'static str> {
    ROUTES.iter().map(|r| r.model).collect()
}

/// 在可用模型中按 ROUTES 优先级挑选默认项。
pub fn pick_preferred_model<'a>(available: &[&'a str]) -> Option<&'a str> {
    for route in ROUTES {
        if let Some(name) = available
            .iter()
            .find(|m| m.eq_ignore_ascii_case(route.model))
        {
            return Some(*name);
        }
    }
    available.first().copied()
}

pub fn route_for_model(model: &str) -> Option<(&'static str, &'static str)> {
    ROUTES
        .iter()
        .find(|r| r.model.eq_ignore_ascii_case(model.trim()))
        .map(|r| (r.base_url, r.model))
}

/// 按当前 model_name 绑定上游；未知模型回落到首条路由。
pub fn apply_route(provider: &mut ProviderConfig) {
    if !is_experimental_free(provider) {
        return;
    }
    let (base_url, model) = route_for_model(&provider.model_name)
        .unwrap_or((ROUTES[0].base_url, ROUTES[0].model));
    provider.base_url = base_url.to_string();
    provider.model_name = model.to_string();
    // LLM7.io 需要公开匿名 key "unused"，其余通道免 key
    provider.api_key = if base_url.contains("llm7.io") {
        "unused".to_string()
    } else {
        String::new()
    };
    provider.protocol = ProviderProtocol::OpenAi;
    provider.auto_adapt_base_url = false;
    provider.model_alias = Some(crate::model::CLIENT_MODEL_ID.to_string());
}

/// 在可用探测结果中选最优：优先更高 t/s，其次更低延迟；都无指标时保持 ROUTES 顺序。
pub fn pick_best_probe<'a>(probes: &'a [FreeModelProbe]) -> Option<&'a FreeModelProbe> {
    let mut best: Option<&FreeModelProbe> = None;
    for probe in probes.iter().filter(|p| p.available) {
        best = Some(match best {
            None => probe,
            Some(cur) => {
                let cur_speed = cur.tokens_per_sec.unwrap_or(0.0);
                let new_speed = probe.tokens_per_sec.unwrap_or(0.0);
                if new_speed > cur_speed + f64::EPSILON {
                    probe
                } else if (new_speed - cur_speed).abs() <= f64::EPSILON {
                    match (cur.latency_ms, probe.latency_ms) {
                        (Some(a), Some(b)) if b < a => probe,
                        (None, Some(_)) => probe,
                        _ => cur,
                    }
                } else {
                    cur
                }
            }
        });
    }
    best
}

/// 若探测出更优路由则改写 provider 并返回 true。
pub fn apply_best_from_probes(
    provider: &mut ProviderConfig,
    probes: &[FreeModelProbe],
) -> bool {
    if !is_experimental_free(provider) {
        return false;
    }
    let Some(best) = pick_best_probe(probes) else {
        return false;
    };
    if best.name.eq_ignore_ascii_case(provider.model_name.trim()) {
        return false;
    }
    provider.model_name = best.name.clone();
    apply_route(provider);
    true
}

/// 请求失败时尝试的下一批 (base_url, model)，不含当前模型。
pub fn failover_routes(current_model: &str) -> Vec<(&'static str, &'static str)> {
    ROUTES
        .iter()
        .filter(|r| !r.model.eq_ignore_ascii_case(current_model.trim()))
        .map(|r| (r.base_url, r.model))
        .collect()
}

pub fn make_experimental_free_provider() -> ProviderConfig {
    let route = &ROUTES[0];
    ProviderConfig {
        id: EXPERIMENTAL_FREE_ID.to_string(),
        name: "实验·免费通道".to_string(),
        provider: EXPERIMENTAL_FREE_PROVIDER.to_string(),
        base_url: route.base_url.to_string(),
        api_key: String::new(),
        model_name: route.model.to_string(),
        protocol: ProviderProtocol::OpenAi,
        enabled: true,
        status: ProviderStatus::Unknown,
        last_checked_at: None,
        last_error: None,
        model_alias: Some(crate::model::CLIENT_MODEL_ID.to_string()),
        auto_adapt_base_url: false,
        tool_text_retry_enabled: false,
        tool_text_retry_extra: 0,
    }
}

/// 迁移旧 Zen + 若未 dismiss 则植入。返回是否改动配置。
pub fn ensure_experimental_free(config: &mut AppConfig) -> bool {
    let mut changed = migrate_legacy_zen(config);
    // 升级祖父：已启用的免费通道视为已接受免责（避免旧配置被突然卡住）。
    if !config.experimental_free_disclaimer_accepted
        && config
            .providers
            .iter()
            .any(|p| is_experimental_free_id(&p.id) && p.enabled)
    {
        config.experimental_free_disclaimer_accepted = true;
        changed = true;
    }

    if config.experimental_zen_dismissed {
        return changed;
    }
    if config
        .providers
        .iter()
        .any(|p| is_experimental_free_id(&p.id))
    {
        return changed;
    }

    let mut provider = make_experimental_free_provider();
    // 新用户首次安装：默认启用实验免费通道并设为当前上游，免责视为安装时接受；
    // 设置弹窗仍可关闭通道或重新确认。用户主动「恢复」走 restore_experimental_free，会重新要求免责。
    provider.enabled = true;
    config.experimental_free_disclaimer_accepted = true;
    if config.active_provider_id.is_none() {
        config.active_provider_id = Some(provider.id.clone());
    }
    config.providers.insert(0, provider);
    true
}

fn migrate_legacy_zen(config: &mut AppConfig) -> bool {
    let Some(idx) = config
        .providers
        .iter()
        .position(|p| p.id.eq_ignore_ascii_case(LEGACY_ZEN_ID))
    else {
        return false;
    };
    let fresh = make_experimental_free_provider();
    let enabled = config.providers[idx].enabled;
    let was_active = config
        .active_provider_id
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case(LEGACY_ZEN_ID));
    config.providers[idx] = fresh;
    config.providers[idx].enabled = enabled;
    if was_active {
        config.active_provider_id = Some(EXPERIMENTAL_FREE_ID.to_string());
    }
    true
}

pub fn restore_experimental_free(config: &mut AppConfig) -> bool {
    config.experimental_zen_dismissed = false;
    // 恢复后需重新确认免责声明，再允许启用。
    config.experimental_free_disclaimer_accepted = false;
    if let Some(existing) = config
        .providers
        .iter_mut()
        .find(|p| is_experimental_free_id(&p.id))
    {
        let enabled = existing.enabled;
        *existing = make_experimental_free_provider();
        // 未确认免责前强制禁用，避免恢复即对外服务。
        existing.enabled = enabled && config.experimental_free_disclaimer_accepted;
        return true;
    }
    let changed = ensure_experimental_free(config);
    if let Some(provider) = config
        .providers
        .iter_mut()
        .find(|p| is_experimental_free_id(&p.id))
    {
        provider.enabled = false;
    }
    changed
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ProbeSource {
    #[default]
    Live,
    Cache,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCategory {
    Ok,
    ChatOnly,
    Transient,
    Reject,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeModelProbe {
    pub name: String,
    pub available: bool,
    pub latency_ms: Option<u64>,
    pub tokens_per_sec: Option<f64>,
    /// 聊天可用但工具调用探测未产生 tool_calls / 上游明确不支持 tools
    #[serde(default)]
    pub chat_only: bool,
    #[serde(default)]
    pub source: ProbeSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<ProbeCategory>,
}

impl FreeModelProbe {
    fn live(
        name: String,
        available: bool,
        latency_ms: Option<u64>,
        tokens_per_sec: Option<f64>,
        chat_only: bool,
        category: ProbeCategory,
    ) -> Self {
        Self {
            name,
            available,
            latency_ms,
            tokens_per_sec,
            chat_only,
            source: ProbeSource::Live,
            category: Some(category),
        }
    }
}

/// 并发探测候选模型；4xx / 网络失败标为不可用。有可用即保留，不做无限重试。
/// 这是廉价 ping 级探测（refresh_on_enable 内部使用），不携带延迟/速度指标。
pub async fn probe_free_models(client: &reqwest::Client) -> Vec<FreeModelProbe> {
    let futs = ROUTES.iter().map(|route| {
        let client = client.clone();
        let name = route.model.to_string();
        let base = route.base_url.to_string();
        let model = route.model.to_string();
        async move {
            let available = probe_one(&client, &base, &model).await;
            FreeModelProbe::live(
                name,
                available,
                None,
                None,
                false,
                if available {
                    ProbeCategory::Ok
                } else {
                    ProbeCategory::Transient
                },
            )
        }
    });
    join_all(futs).await
}

#[derive(Debug, Clone, Default)]
pub struct TestFreeOpts {
    /// true：忽略缓存，全量或单条重测
    pub force: bool,
    /// 仅测这一条模型名（大小写不敏感）；None = 全部 ROUTES
    pub only_model: Option<String>,
    /// 单次操作的预算。到点后未完成的路由回退到新鲜缓存（source=cache），
    /// 保证点击「获取模型列表」在有限时间内返回，而不是等满全部路由的 30s+25s。
    /// None 用于闲时后台探测等无界面等待的场景。
    pub total_timeout: Option<std::time::Duration>,
}

/// 预算到点后的结果归并：真实结果优先，未完成路由只在缓存新鲜且指纹一致时回退，
/// 无缓存则保持「未知」。绝不为超时路由编造可用性或 0 指标。
fn resolve_probe_results(
    live: Vec<(String, Option<FreeModelProbe>)>,
    entries: &std::collections::HashMap<String, crate::experimental_free_cache::CachedVerdict>,
) -> (Vec<FreeModelProbe>, usize, usize) {
    use crate::experimental_free_cache as cache;
    let mut out = Vec::with_capacity(live.len());
    let mut from_cache = 0usize;
    let mut unanswered = 0usize;
    for (model, result) in live {
        match result {
            Some(probe) => out.push(probe),
            None => {
                let cached = route_for_model(&model).and_then(|(base, canonical)| {
                    let fp = cache::route_fingerprint(base, canonical);
                    cache::fresh(entries, &model, &fp).map(|hit| cache::to_probe(&model, hit))
                });
                if let Some(probe) = cached {
                    from_cache += 1;
                    out.push(probe);
                } else {
                    unanswered += 1;
                }
            }
        }
    }
    (out, from_cache, unanswered)
}

/// 逐路由真实 LLM 测速：复用 probe_llm_connectivity；成功后再做一轮轻量 tools 探测。
/// 并发上限 4。`force=false` 时新鲜缓存直接返回（source=cache）。
pub async fn test_free_models(
    client: &reqwest::Client,
    config_dir: &std::path::Path,
    opts: TestFreeOpts,
) -> Vec<FreeModelProbe> {
    use crate::experimental_free_cache as cache;

    let path = cache::cache_path(config_dir);
    let mut entries = cache::load(&path);
    let alive: Vec<&str> = ROUTES.iter().map(|r| r.model).collect();
    cache::prune(&mut entries, &alive);

    let only = opts
        .only_model
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let routes: Vec<(usize, String, String)> = ROUTES
        .iter()
        .enumerate()
        .filter(|(_, route)| {
            only.as_ref()
                .map(|m| route.model.eq_ignore_ascii_case(m))
                .unwrap_or(true)
        })
        .map(|(idx, route)| (idx, route.base_url.to_string(), route.model.to_string()))
        .collect();

    let mut out: Vec<FreeModelProbe> = Vec::with_capacity(routes.len());
    let mut to_probe: Vec<(usize, String, String)> = Vec::new();

    for (route_id, base_url, model) in routes {
        let fp = cache::route_fingerprint(&base_url, &model);
        if !opts.force {
            if let Some(hit) = cache::fresh(&entries, &model, &fp) {
                out.push(cache::to_probe(&model, hit));
                continue;
            }
        }
        to_probe.push((route_id, base_url, model));
    }

    let started = std::time::Instant::now();
    let deadline = opts
        .total_timeout
        .map(|budget| tokio::time::Instant::now() + budget);
    info!(
        action = "free_models_test",
        force = opts.force,
        routes = out.len() + to_probe.len(),
        to_probe = to_probe.len(),
        budget_ms = opts
            .total_timeout
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default(),
        "free model test started"
    );

    // 每条路由要么在预算内拿到真实结果，要么返回 None 交给下面的缓存兜底。
    // 不再让 UI 等满 11 条路由 × (30s 聊天 + 25s tools) 的最坏耗时。
    let live: Vec<(String, Option<FreeModelProbe>)> = futures_util::stream::iter(
        to_probe.into_iter().map(|(route_id, base_url, model)| {
            let client = client.clone();
            let name = model.clone();
            let probe_name = name.clone();
            let deadline = deadline;
            async move {
                let probe_fut = async {
                    let provider = probe_provider(&base_url, &model);
                    let report = crate::probe::probe_llm_connectivity(&client, &provider).await;
                    let api_key = crate::model::mask_secret(&provider.api_key);
                    if !report.ok {
                        let category =
                            cache::classify_failure(report.error.as_deref());
                        warn!(
                            action = "free_route_probe",
                            provider = %provider.provider,
                            model = %probe_name,
                            route_id,
                            status = "fail",
                            category = ?category,
                            latency_ms = report.latency_ms,
                            api_key = %api_key,
                            error = report.error.as_deref().unwrap_or_default(),
                            "free route probe failed"
                        );
                        return FreeModelProbe::live(
                            probe_name,
                            false,
                            Some(report.latency_ms),
                            None,
                            false,
                            category,
                        );
                    }

                    let tools = probe_tools_capability(&client, &provider).await;
                    let (chat_only, category) = match tools {
                        ToolsVerdict::Supported => (false, ProbeCategory::Ok),
                        ToolsVerdict::ChatOnly => (true, ProbeCategory::ChatOnly),
                        ToolsVerdict::Inconclusive => (false, ProbeCategory::Ok),
                    };
                    info!(
                        action = "free_route_probe",
                        provider = %provider.provider,
                        model = %probe_name,
                        route_id,
                        status = "ok",
                        chat_only,
                        category = ?category,
                        latency_ms = report.latency_ms,
                        tokens_per_sec = report.tokens_per_sec,
                        api_key = %api_key,
                        "free route probe succeeded"
                    );
                    FreeModelProbe::live(
                        probe_name,
                        true,
                        Some(report.latency_ms),
                        Some(report.tokens_per_sec),
                        chat_only,
                        category,
                    )
                };
                match deadline {
                    Some(at) => match tokio::time::timeout_at(at, probe_fut).await {
                        Ok(probe) => (name, Some(probe)),
                        Err(_) => {
                            warn!(
                                action = "free_route_probe",
                                model = %name,
                                route_id,
                                status = "budget_exceeded",
                                "free route probe skipped by total budget"
                            );
                            (name, None)
                        }
                    },
                    None => (name, Some(probe_fut.await)),
                }
            }
        }),
    )
    .buffer_unordered(4)
    .collect()
    .await;

    let (resolved, from_cache, unanswered) = resolve_probe_results(live, &entries);
    let fresh_probes: Vec<FreeModelProbe> = resolved
        .iter()
        .filter(|probe| probe.source == ProbeSource::Live)
        .cloned()
        .collect();
    out.extend(resolved);

    // 只有本次真实测到的结果才刷新缓存 TTL；缓存兜底项不能把自己「续期」。
    for probe in &fresh_probes {
        if let Some((base, model)) = route_for_model(&probe.name) {
            cache::record(
                &mut entries,
                model,
                cache::route_fingerprint(base, model),
                probe,
            );
        }
    }
    cache::save(&path, &entries);

    info!(
        action = "free_models_test",
        probed = fresh_probes.len(),
        from_cache,
        unanswered,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "free model test finished"
    );

    // 保持 ROUTES 顺序，方便 UI / auto_rotate
    out.sort_by_key(|p| {
        ROUTES
            .iter()
            .position(|r| r.model.eq_ignore_ascii_case(&p.name))
            .unwrap_or(usize::MAX)
    });
    out
}

/// 读缓存新鲜判定（不发网），供控制台打开时展示上次结果。
pub fn list_cached_probes(config_dir: &std::path::Path) -> Vec<FreeModelProbe> {
    use crate::experimental_free_cache as cache;
    let entries = cache::load(&cache::cache_path(config_dir));
    ROUTES
        .iter()
        .filter_map(|route| {
            let fp = cache::route_fingerprint(route.base_url, route.model);
            cache::fresh(&entries, route.model, &fp).map(|v| cache::to_probe(route.model, v))
        })
        .collect()
}

fn probe_provider(base_url: &str, model: &str) -> ProviderConfig {
    ProviderConfig {
        id: "free-route-probe".to_string(),
        name: "免费路由探测".to_string(),
        provider: "free-route-probe".to_string(),
        base_url: base_url.to_string(),
        api_key: if base_url.contains("llm7.io") {
            "unused".to_string()
        } else {
            String::new()
        },
        model_name: model.to_string(),
        protocol: ProviderProtocol::OpenAi,
        enabled: true,
        status: ProviderStatus::Unknown,
        last_checked_at: None,
        last_error: None,
        model_alias: None,
        auto_adapt_base_url: false,
        tool_text_retry_enabled: false,
        tool_text_retry_extra: 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsVerdict {
    Supported,
    ChatOnly,
    Inconclusive,
}

/// 聊天已通后：带一个 function 工具，看模型会不会产出 tool_calls。
/// 明确「只回文本」或上游拒绝 tools → ChatOnly；超时/网络 → Inconclusive（不误伤）。
pub async fn probe_tools_capability(
    client: &reqwest::Client,
    provider: &ProviderConfig,
) -> ToolsVerdict {
    use crate::gateway::build_target_url;
    use serde_json::json;

    let url = build_target_url(&provider.base_url, "v1/chat/completions", "");
    let body = json!({
        "model": provider.model_name,
        "messages": [{
            "role": "user",
            "content": "Call the tool named ping_probe with argument note=\"ok\". Do not answer in plain text."
        }],
        "tools": [{
            "type": "function",
            "function": {
                "name": "ping_probe",
                "description": "Connectivity probe tool; call it once.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "note": { "type": "string" }
                    },
                    "required": ["note"]
                }
            }
        }],
        "tool_choice": "auto",
        "max_tokens": 64,
        "stream": false
    });

    let mut req = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(25));
    if !provider.api_key.trim().is_empty() {
        req = req.bearer_auth(&provider.api_key);
    }
    let response = match req.send().await {
        Ok(r) => r,
        Err(_) => return ToolsVerdict::Inconclusive,
    };
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let lower = text.to_lowercase();
        if lower.contains("tool")
            || lower.contains("function")
            || lower.contains("not support")
            || lower.contains("unsupported")
            || status.as_u16() == 400
        {
            return ToolsVerdict::ChatOnly;
        }
        return ToolsVerdict::Inconclusive;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ToolsVerdict::Inconclusive;
    };
    let calls = value
        .pointer("/choices/0/message/tool_calls")
        .and_then(|v| v.as_array());
    if calls.map(|a| !a.is_empty()).unwrap_or(false) {
        return ToolsVerdict::Supported;
    }
    // 成功 HTTP 但只有文本 → 诚实标仅对话
    ToolsVerdict::ChatOnly
}

async fn probe_one(client: &reqwest::Client, base_url: &str, model: &str) -> bool {
    let url = chat_url(base_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 8,
        "stream": false
    });
    let mut req = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    if base_url.contains("llm7.io") {
        req = req.header(reqwest::header::AUTHORIZATION, "Bearer unused");
    }
    let req = req.json(&body).timeout(std::time::Duration::from_secs(18));
    match req.send().await {
        Ok(resp) => {
            let ok = resp.status().is_success();
            if !ok {
                warn!(
                    action = "free_route_ping",
                    model,
                    status = %resp.status(),
                    "free route ping probe returned non-success"
                );
            }
            ok
        }
        Err(err) => {
            warn!(
                action = "free_route_ping",
                model,
                error = %err,
                "free route ping probe failed"
            );
            false
        }
    }
}

fn chat_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else if base.ends_with("/v1") || base.ends_with("/openai") || base.ends_with("/gateway") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

/// 启用时：探测并把 model 调到第一个可用；全挂则写 last_error。
pub async fn refresh_on_enable(
    client: &reqwest::Client,
    provider: &mut ProviderConfig,
) -> Vec<FreeModelProbe> {
    let probes = probe_free_models(client).await;
    let available: Vec<&str> = probes
        .iter()
        .filter(|p| p.available)
        .map(|p| p.name.as_str())
        .collect();
    if available.is_empty() {
        warn!(
            action = "refresh_on_enable",
            provider = %EXPERIMENTAL_FREE_PROVIDER,
            model = %provider.model_name,
            available_count = 0,
            error = "免费实验通道暂无可用模型，请稍后重试或恢复实验源",
            "free channel refresh found no available route"
        );
        provider.last_error = Some("免费实验通道暂无可用模型，请稍后重试或恢复实验源".into());
        provider.status = ProviderStatus::Unavailable;
    } else {
        if !available
            .iter()
            .any(|m| m.eq_ignore_ascii_case(provider.model_name.trim()))
        {
            if let Some(preferred) = pick_preferred_model(&available) {
                provider.model_name = preferred.to_string();
            }
        }
        apply_route(provider);
        provider.last_error = None;
        provider.status = ProviderStatus::Available;
    }
    provider.last_checked_at = Some(chrono::Utc::now());
    info!(
        action = "refresh_on_enable",
        provider = %EXPERIMENTAL_FREE_PROVIDER,
        model = %provider.model_name,
        available_count = available.len(),
        status = ?provider.status,
        "free channel refresh finished"
    );
    probes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_legacy() {
        assert!(is_experimental_free_id("sugt-free"));
        assert!(is_experimental_free_id("sugt-zen-free"));
        assert!(!is_experimental_free_id("my-openai"));
    }

    #[test]
    fn apply_route_binds_url() {
        let mut p = make_experimental_free_provider();
        p.model_name = "kilo-auto/free".into();
        apply_route(&mut p);
        assert!(p.base_url.contains("kilo.ai"));
        assert!(p.api_key.is_empty());
        assert_eq!(p.public_model_id(), crate::model::CLIENT_MODEL_ID);
    }

    #[test]
    fn migrate_replaces_zen() {
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: LEGACY_ZEN_ID.into(),
            name: "旧".into(),
            provider: "opencode-zen".into(),
            base_url: "https://opencode.ai/zen/v1".into(),
            api_key: "public".into(),
            model_name: "deepseek-v4-flash-free".into(),
            protocol: ProviderProtocol::OpenAi,
            enabled: true,
            status: ProviderStatus::Unknown,
            last_checked_at: None,
            last_error: None,
            model_alias: None,
            auto_adapt_base_url: false,
            tool_text_retry_enabled: false,
            tool_text_retry_extra: 0,
        });
        cfg.active_provider_id = Some(LEGACY_ZEN_ID.into());
        assert!(ensure_experimental_free(&mut cfg));
        assert!(cfg
            .providers
            .iter()
            .any(|p| p.id == EXPERIMENTAL_FREE_ID));
        assert!(!cfg
            .providers
            .iter()
            .any(|p| p.id.eq_ignore_ascii_case(LEGACY_ZEN_ID)));
        assert_eq!(
            cfg.active_provider_id.as_deref(),
            Some(EXPERIMENTAL_FREE_ID)
        );
    }

    #[test]
    fn fresh_install_enables_free_channel_by_default() {
        let mut cfg = AppConfig::default();
        assert!(cfg.providers.is_empty());
        assert!(ensure_experimental_free(&mut cfg));
        let free = cfg
            .providers
            .iter()
            .find(|p| p.id == EXPERIMENTAL_FREE_ID)
            .expect("sugt-free");
        assert!(free.enabled, "新用户应默认启用实验免费通道");
        assert!(cfg.experimental_free_disclaimer_accepted);
        assert_eq!(cfg.active_provider_id.as_deref(), Some(EXPERIMENTAL_FREE_ID));
    }

    #[test]
    fn candidates_nonempty() {
        let models = candidate_models();
        assert!(!models.is_empty());
        assert_eq!(models.len(), 11);
        // Tier 1 起始：Kilo.ai
        assert_eq!(models[0], "kilo-auto/free");
        assert_eq!(models[1], "baidu/ernie-5.5-flash-preview:free");
        assert_eq!(route_choices()[0].host, "api.kilo.ai");
        // Tier 1 中段：BlockRun（index 4-6）
        assert_eq!(route_choices()[4].host, "blockrun.ai");
        // Tier 2：LLM7（index 7）、JarvisClaw（index 8）
        assert_eq!(route_choices()[7].host, "api.llm7.io");
        // Tier 3：Pollinations 降级（index 9-10）
        assert!(route_for_model("openai-fast").is_some());
        assert_eq!(route_choices()[9].host, "text.pollinations.ai");
    }

    #[test]
    fn pick_best_probe_prefers_speed_then_latency() {
        let probes = vec![
            FreeModelProbe::live("slow".into(), true, Some(50), Some(10.0), false, ProbeCategory::Ok),
            FreeModelProbe::live("fast".into(), true, Some(80), Some(40.0), false, ProbeCategory::Ok),
            FreeModelProbe::live("down".into(), false, None, None, false, ProbeCategory::Reject),
        ];
        assert_eq!(pick_best_probe(&probes).map(|p| p.name.as_str()), Some("fast"));
        let tie = vec![
            FreeModelProbe::live("a".into(), true, Some(120), Some(20.0), false, ProbeCategory::Ok),
            FreeModelProbe::live("b".into(), true, Some(40), Some(20.0), false, ProbeCategory::Ok),
        ];
        assert_eq!(pick_best_probe(&tie).map(|p| p.name.as_str()), Some("b"));
    }

    #[test]
    fn pick_preferred_respects_priority() {
        // kilo-auto/free 是 ROUTES[0]，优先级最高
        assert_eq!(
            pick_preferred_model(&["openai-fast", "kilo-auto/free"]),
            Some("kilo-auto/free")
        );
        // openai 在 Tier 3，openai-fast 也在 Tier 3，优先靠前
        assert_eq!(
            pick_preferred_model(&["openai", "nvidia/nemotron-3.5-bolt"]),
            Some("nvidia/nemotron-3.5-bolt")
        );
        // openrouter/free 已移除，只留 openai-fast
        assert_eq!(
            pick_preferred_model(&["unknown", "openai-fast"]),
            Some("openai-fast")
        );
    }

    #[test]
    fn free_model_probe_serializes_as_camel_case() {
        let value = serde_json::to_value(FreeModelProbe::live(
            "kilo-auto/free".into(),
            true,
            Some(1234),
            Some(12.5),
            true,
            ProbeCategory::ChatOnly,
        ))
        .expect("to_value");
        assert_eq!(value["name"], "kilo-auto/free");
        assert_eq!(value["available"], true);
        assert_eq!(value["latencyMs"], 1234);
        assert_eq!(value["tokensPerSec"], 12.5);
        assert_eq!(value["chatOnly"], true);
        assert_eq!(value["source"], "live");
        assert_eq!(value["category"], "chat_only");
        assert!(value.get("latency_ms").is_none());
        assert!(value.get("tokens_per_sec").is_none());

        // 廉价探测路径：指标缺省序列化为 null，字段名仍是 camelCase
        let value = serde_json::to_value(FreeModelProbe::live(
            "openai-fast".into(),
            false,
            None,
            None,
            false,
            ProbeCategory::Transient,
        ))
        .expect("to_value");
        assert!(value["latencyMs"].is_null());
        assert!(value["tokensPerSec"].is_null());
        assert_eq!(value["chatOnly"], false);
    }

    #[test]
    fn model_test_result_serializes_as_camel_case() {
        let value = serde_json::to_value(crate::commands::ModelTestResult {
            model_name: "gpt-4o-mini".into(),
            available: false,
            latency_ms: Some(800),
            tokens_per_sec: None,
        })
        .expect("to_value");
        assert_eq!(value["modelName"], "gpt-4o-mini");
        assert_eq!(value["available"], false);
        assert_eq!(value["latencyMs"], 800);
        assert!(value["tokensPerSec"].is_null());
        assert!(value.get("model_name").is_none());
    }

    #[test]
    fn budget_fallback_uses_fresh_cache_and_keeps_unknown() {
        let mut entries = std::collections::HashMap::new();
        let (base, model) = route_for_model("openai-fast").expect("known route");
        let fp = crate::experimental_free_cache::route_fingerprint(base, model);
        let cached = FreeModelProbe::live(
            model.to_string(),
            true,
            Some(123),
            Some(4.5),
            false,
            ProbeCategory::Ok,
        );
        crate::experimental_free_cache::record(&mut entries, model, fp, &cached);

        let live = vec![
            (
                "openai".to_string(),
                Some(FreeModelProbe::live(
                    "openai".to_string(),
                    false,
                    Some(9),
                    None,
                    false,
                    ProbeCategory::Transient,
                )),
            ),
            ("openai-fast".to_string(), None),
            ("unknown/model".to_string(), None),
        ];
        let (resolved, from_cache, unanswered) = resolve_probe_results(live, &entries);

        assert_eq!(resolved.len(), 2, "未知路由不能被编造成一条结果");
        assert_eq!(from_cache, 1);
        assert_eq!(unanswered, 1);
        let cached_row = resolved
            .iter()
            .find(|p| p.name == "openai-fast")
            .expect("cache fallback");
        assert_eq!(cached_row.source, ProbeSource::Cache);
        assert!(cached_row.available);
        assert_eq!(cached_row.latency_ms, Some(123));
        let live_row = resolved
            .iter()
            .find(|p| p.name == "openai")
            .expect("live result");
        assert_eq!(live_row.source, ProbeSource::Live);
        assert!(!live_row.available);
    }

    /// 真实公网探测：免费通道是否在线取决于外部服务，不能作为默认套件的一部分，
    /// 否则上游抖动会把 CI 弄红。需要时手动跑 `cargo test -- --ignored`。
    #[tokio::test]
    #[ignore = "live network probe; run with cargo test -- --ignored"]
    async fn live_probe_finds_at_least_one() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(25))
            .build()
            .expect("client");
        let probes = probe_free_models(&client).await;
        let ok = probes.iter().filter(|p| p.available).count();
        assert!(
            ok > 0,
            "expected at least one keyless free model, got {:?}",
            probes
        );
    }
}
