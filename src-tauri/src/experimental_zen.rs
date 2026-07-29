//! 实验性 OpenCode Zen 免费通道（弱集成，可删可改）。

use crate::model::{ProviderConfig, ProviderProtocol, ProviderStatus, AppConfig};

pub const EXPERIMENTAL_ZEN_ID: &str = "sugt-zen-free";
pub const EXPERIMENTAL_ZEN_PROVIDER: &str = "opencode-zen";

pub fn is_experimental_zen_id(id: &str) -> bool {
    id.eq_ignore_ascii_case(EXPERIMENTAL_ZEN_ID)
}

/// 是否走 OpenCode Zen 上游（实验源或用户改造成同地址）
pub fn is_zen_upstream(provider: &ProviderConfig) -> bool {
    is_experimental_zen_id(&provider.id)
        || provider
            .provider
            .eq_ignore_ascii_case(EXPERIMENTAL_ZEN_PROVIDER)
        || provider
            .base_url
            .to_ascii_lowercase()
            .contains("opencode.ai/zen")
}

pub fn make_experimental_zen_provider() -> ProviderConfig {
    ProviderConfig {
        id: EXPERIMENTAL_ZEN_ID.to_string(),
        name: "实验·OpenCode Zen".to_string(),
        provider: EXPERIMENTAL_ZEN_PROVIDER.to_string(),
        base_url: "https://opencode.ai/zen/v1".to_string(),
        api_key: "public".to_string(),
        model_name: "deepseek-v4-flash-free".to_string(),
        protocol: ProviderProtocol::OpenAi,
        enabled: true,
        status: ProviderStatus::Unknown,
        last_checked_at: None,
        last_error: None,
        model_alias: None,
        auto_adapt_base_url: false,
    }
}

/// 若未 dismiss 且列表中不存在，则植入实验 Provider。返回是否改动了配置。
pub fn ensure_experimental_zen(config: &mut AppConfig) -> bool {
    if config.experimental_zen_dismissed {
        return false;
    }
    if config
        .providers
        .iter()
        .any(|p| is_experimental_zen_id(&p.id))
    {
        return false;
    }

    let provider = make_experimental_zen_provider();
    if config.active_provider_id.is_none() {
        config.active_provider_id = Some(provider.id.clone());
    }
    // 放在列表前面，方便新用户看见
    config.providers.insert(0, provider);
    true
}

/// 恢复实验源（清除 dismiss，若不存在则重新植入）。
pub fn restore_experimental_zen(config: &mut AppConfig) -> bool {
    config.experimental_zen_dismissed = false;
    if let Some(existing) = config
        .providers
        .iter_mut()
        .find(|p| is_experimental_zen_id(&p.id))
    {
        // 已存在：恢复为默认实验参数（保留用户改过的也可以，这里重置为可用默认）
        let fresh = make_experimental_zen_provider();
        existing.name = fresh.name;
        existing.provider = fresh.provider;
        existing.base_url = fresh.base_url;
        existing.api_key = fresh.api_key;
        existing.model_name = fresh.model_name;
        existing.protocol = fresh.protocol;
        existing.enabled = true;
        existing.auto_adapt_base_url = false;
        existing.last_error = None;
        existing.status = ProviderStatus::Unknown;
        return true;
    }
    ensure_experimental_zen(config)
}
