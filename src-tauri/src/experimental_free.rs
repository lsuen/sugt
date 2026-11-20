//! 实验·免费通道：公开零 Key 多源，内置只读，可删可开关。
//! 与用户自定义 provider 隔离——探测/failover 仅作用于本通道。

use crate::model::{AppConfig, ProviderConfig, ProviderProtocol, ProviderStatus};
use futures_util::future::join_all;
use futures_util::StreamExt;
use serde::Serialize;

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
    let changed = migrate_legacy_zen(config);

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

    let provider = make_experimental_free_provider();
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
    if let Some(existing) = config
        .providers
        .iter_mut()
        .find(|p| is_experimental_free_id(&p.id))
    {
        let enabled = existing.enabled;
        *existing = make_experimental_free_provider();
        existing.enabled = enabled;
        return true;
    }
    ensure_experimental_free(config)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeModelProbe {
    pub name: String,
    pub available: bool,
    pub latency_ms: Option<u64>,
    pub tokens_per_sec: Option<f64>,
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
            FreeModelProbe {
                name,
                available,
                latency_ms: None,
                tokens_per_sec: None,
            }
        }
    });
    join_all(futs).await
}

/// 逐路由真实 LLM 测速：复用 probe_llm_connectivity 发一条真实消息，
/// 返回可用性 + 延迟 + 生成速度。并发上限 4，避免瞬时打满公开免费上游。
pub async fn test_free_models(client: &reqwest::Client) -> Vec<FreeModelProbe> {
    // 先落成 owned 列表，避免闭包参数引用生命周期在 boxed future 里触发 HRTB 限制。
    let routes: Vec<(String, String)> = ROUTES
        .iter()
        .map(|route| (route.base_url.to_string(), route.model.to_string()))
        .collect();
    futures_util::stream::iter(routes.into_iter().map(|(base_url, model)| {
        let client = client.clone();
        let name = model.clone();
        // 临时 provider 只用于探测：不取 sugt-free id，避免 prepare_upstream_provider 再改绑路由。
        let provider = ProviderConfig {
            id: "free-route-probe".to_string(),
            name: "免费路由探测".to_string(),
            provider: "free-route-probe".to_string(),
            base_url: base_url.clone(),
            // LLM7.io 需要公开匿名 key "unused"，其余通道免 key（与 apply_route 一致）
            api_key: if base_url.contains("llm7.io") {
                "unused".to_string()
            } else {
                String::new()
            },
            model_name: model,
            protocol: ProviderProtocol::OpenAi,
            enabled: true,
            status: ProviderStatus::Unknown,
            last_checked_at: None,
            last_error: None,
            model_alias: None,
            auto_adapt_base_url: false,
            tool_text_retry_enabled: false,
            tool_text_retry_extra: 0,
        };
        async move {
            let report = crate::probe::probe_llm_connectivity(&client, &provider).await;
            FreeModelProbe {
                name,
                available: report.ok,
                latency_ms: Some(report.latency_ms),
                tokens_per_sec: report.ok.then_some(report.tokens_per_sec),
            }
        }
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await
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
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
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
        let value = serde_json::to_value(FreeModelProbe {
            name: "kilo-auto/free".into(),
            available: true,
            latency_ms: Some(1234),
            tokens_per_sec: Some(12.5),
        })
        .expect("to_value");
        assert_eq!(value["name"], "kilo-auto/free");
        assert_eq!(value["available"], true);
        assert_eq!(value["latencyMs"], 1234);
        assert_eq!(value["tokensPerSec"], 12.5);
        assert!(value.get("latency_ms").is_none());
        assert!(value.get("tokens_per_sec").is_none());

        // 廉价探测路径：指标缺省序列化为 null，字段名仍是 camelCase
        let value = serde_json::to_value(FreeModelProbe {
            name: "openai-fast".into(),
            available: false,
            latency_ms: None,
            tokens_per_sec: None,
        })
        .expect("to_value");
        assert!(value["latencyMs"].is_null());
        assert!(value["tokensPerSec"].is_null());
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
