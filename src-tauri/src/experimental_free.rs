//! 实验·免费通道：公开零 Key 多源，内置只读，可删可开关。
//! 与用户自定义 provider 隔离——探测/failover 仅作用于本通道。

use crate::model::{AppConfig, ProviderConfig, ProviderProtocol, ProviderStatus};
use futures_util::future::join_all;
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
/// openrouter/free → kilo-auto/free → 其余。
const ROUTES: &[FreeRoute] = &[
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "openrouter/free",
    },
    FreeRoute {
        base_url: "https://api.kilo.ai/api/gateway",
        model: "kilo-auto/free",
    },
    FreeRoute {
        base_url: "https://text.pollinations.ai/openai",
        model: "openai-fast",
    },
    FreeRoute {
        base_url: "https://text.pollinations.ai/openai",
        model: "openai",
    },
    FreeRoute {
        base_url: "https://api.llm7.io/v1",
        model: "fast",
    },
    FreeRoute {
        base_url: "https://api.llm7.io/v1",
        model: "default",
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
    provider.api_key.clear();
    provider.protocol = ProviderProtocol::OpenAi;
    provider.auto_adapt_base_url = false;
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
        model_alias: None,
        auto_adapt_base_url: false,
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
pub struct FreeModelProbe {
    pub name: String,
    pub available: bool,
}

/// 并发探测候选模型；4xx / 网络失败标为不可用。有可用即保留，不做无限重试。
pub async fn probe_free_models(client: &reqwest::Client) -> Vec<FreeModelProbe> {
    let futs = ROUTES.iter().map(|route| {
        let client = client.clone();
        let name = route.model.to_string();
        let base = route.base_url.to_string();
        let model = route.model.to_string();
        async move {
            let available = probe_one(&client, &base, &model).await;
            FreeModelProbe { name, available }
        }
    });
    join_all(futs).await
}

async fn probe_one(client: &reqwest::Client, base_url: &str, model: &str) -> bool {
    let url = chat_url(base_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 8,
        "stream": false
    });
    let req = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(18));
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
        assert!(!candidate_models().is_empty());
        assert!(route_for_model("openai-fast").is_some());
        assert_eq!(candidate_models()[0], "openrouter/free");
        assert_eq!(candidate_models()[1], "kilo-auto/free");
    }

    #[test]
    fn pick_preferred_respects_priority() {
        assert_eq!(
            pick_preferred_model(&["openai-fast", "kilo-auto/free", "openrouter/free"]),
            Some("openrouter/free")
        );
        assert_eq!(
            pick_preferred_model(&["openai", "kilo-auto/free"]),
            Some("kilo-auto/free")
        );
        assert_eq!(
            pick_preferred_model(&["fast", "openai-fast"]),
            Some("openai-fast")
        );
    }

    #[tokio::test]
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
