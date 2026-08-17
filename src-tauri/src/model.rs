use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gateway_stats::TrafficStatsView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    #[default]
    #[serde(alias = "open_ai", alias = "OpenAi", alias = "OpenAI")]
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderStatus {
    Unknown,
    Available,
    Unavailable,
}

/// Anthropic 接入点的协议模式（用户可在控制台 Agent 接入点切换）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicAccessMode {
    /// 自动匹配：识别到 Claude 系客户端优先走原生端点，否则走转换（默认）
    #[default]
    Auto,
    /// 强制走 OpenAI 协议（始终转换）
    OpenAi,
    /// 强制走 Anthropic 原生协议（服务商不支持时返回明确错误）
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub status: ProviderStatus,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub model_alias: Option<String>,
    /// 保存时自动规范化 Base URL（补 /v1、修复火山路径等）；关闭则原样保存
    #[serde(default = "default_auto_adapt_base_url")]
    pub auto_adapt_base_url: bool,
}

impl ProviderConfig {
    pub fn new(
        name: impl Into<String>,
        provider: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            provider: provider.into(),
            base_url: trim_base_url(base_url.into()),
            api_key: api_key.into(),
            model_name: model_name.into(),
            protocol: ProviderProtocol::OpenAi,
            enabled: true,
            status: ProviderStatus::Unknown,
            last_checked_at: None,
            last_error: None,
            model_alias: None,
            auto_adapt_base_url: default_auto_adapt_base_url(),
        }
    }

    pub fn masked_key(&self) -> String {
        mask_secret(&self.api_key)
    }

    pub fn public_model_id(&self) -> String {
        self.model_alias
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| self.model_name.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuitBehavior {
    /// 仅退出程序；若已接管则尝试留下独立网关（不推荐，默认会清理接管）
    ExitOnly,
    /// 退出并停止网关；仍会清理接管写入
    StopGateway,
    /// 退出、停止网关并清理接管（推荐）
    #[default]
    StopAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub failover_enabled: bool,
    pub active_provider_id: Option<String>,
    pub providers: Vec<ProviderConfig>,
    pub autostart: bool,
    #[serde(default)]
    pub autostart_gateway: bool,
    #[serde(default)]
    pub quit_behavior: QuitBehavior,
    /// 端口被占用时自动尝试备选端口
    #[serde(default = "default_port_fallback_enabled")]
    pub port_fallback_enabled: bool,
    #[serde(default = "default_port_fallback_ports")]
    pub port_fallback_ports: Vec<u16>,
    /// 后台循环检测网关并在不可达时自动拉起
    #[serde(default = "default_gateway_watchdog_enabled")]
    pub gateway_watchdog_enabled: bool,
    /// 允许局域网其它设备连接（绑定 0.0.0.0）
    #[serde(default)]
    pub allow_lan_access: bool,
    /// 客户端连接本地网关时使用的 API Key（可自定义防蹭网）
    #[serde(default = "default_gateway_client_api_key")]
    pub gateway_client_api_key: String,
    /// 是否显式启用 Codex 接管（默认关闭，避免 OPENAI_* 误伤 OpenCode 等）
    #[serde(default)]
    pub codex_takeover_enabled: bool,
    /// 是否显式启用 Claude 接管
    #[serde(default)]
    pub claude_takeover_enabled: bool,
    /// 启动时自动接管（已弃用，保留字段兼容旧配置）
    #[serde(default)]
    pub auto_takeover_enabled: bool,
    /// 应保持接管的模板 ID 列表（退出清环境，下次按此恢复）
    #[serde(default)]
    pub active_takeover_ids: Vec<String>,
    /// 用户已删除实验·OpenCode Zen，不再自动植入
    #[serde(default)]
    pub experimental_zen_dismissed: bool,
    /// Anthropic 接入点的协议模式（auto / openai / anthropic）
    #[serde(default)]
    pub anthropic_access_mode: AnthropicAccessMode,
}

fn default_gateway_client_api_key() -> String {
    "sugt-local-key".to_string()
}

fn default_port_fallback_enabled() -> bool {
    true
}

fn default_port_fallback_ports() -> Vec<u16> {
    vec![9878, 18887, 28887]
}

fn default_gateway_watchdog_enabled() -> bool {
    true
}

fn default_auto_adapt_base_url() -> bool {
    true
}

impl AppConfig {
    /// 实际 bind 地址
    pub fn bind_host(&self) -> String {
        if self.allow_lan_access {
            "0.0.0.0".to_string()
        } else if self.host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            self.host.clone()
        }
    }

    /// 写入客户端环境变量 / 健康检查用的主机名
    pub fn client_host(&self) -> String {
        if self.host == "0.0.0.0" || self.allow_lan_access {
            "127.0.0.1".to_string()
        } else {
            self.host.clone()
        }
    }

    pub fn listen_url(&self) -> String {
        format!("http://{}:{}", self.client_host(), self.port)
    }

    pub fn port_candidates(&self) -> Vec<u16> {
        let mut ports = vec![self.port];
        if self.port_fallback_enabled {
            for port in &self.port_fallback_ports {
                if !ports.contains(port) {
                    ports.push(*port);
                }
            }
        }
        ports
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8787,
            failover_enabled: true,
            active_provider_id: None,
            providers: Vec::new(),
            autostart: false,
            autostart_gateway: false,
            quit_behavior: QuitBehavior::default(),
            port_fallback_enabled: default_port_fallback_enabled(),
            port_fallback_ports: default_port_fallback_ports(),
            gateway_watchdog_enabled: default_gateway_watchdog_enabled(),
            allow_lan_access: false,
            gateway_client_api_key: default_gateway_client_api_key(),
            codex_takeover_enabled: false,
            claude_takeover_enabled: false,
            auto_takeover_enabled: false,
            active_takeover_ids: Vec::new(),
            experimental_zen_dismissed: false,
            anthropic_access_mode: AnthropicAccessMode::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyHit {
    pub provider_id: String,
    pub provider_name: String,
    pub path: String,
    pub failover: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub running: bool,
    pub listen_url: String,
    pub openai_base_url: String,
    pub anthropic_base_url: String,
    pub gateway_client_api_key: String,
    pub gateway_client_api_key_masked: String,
    pub public_model_id: Option<String>,
    pub allow_lan_access: bool,
    pub active_model: Option<String>,
    pub active_provider: Option<String>,
    pub active_provider_id: Option<String>,
    pub last_proxy_provider: Option<String>,
    pub last_proxy_path: Option<String>,
    pub last_proxy_failover: bool,
    pub last_proxy_at: Option<DateTime<Utc>>,
    pub config_dir: String,
    pub log_file: String,
    pub trial: TrialStatusView,
    pub traffic: TrafficStatsView,
    /// Anthropic 接入点协议模式（auto / openai / anthropic）
    pub anthropic_access_mode: AnthropicAccessMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrialStatusView {
    pub edition: String,
    pub product_line: String,
    pub product_label: String,
    pub trial_enabled: bool,
    pub valid: bool,
    pub status: String,
    pub message: String,
    pub build_id: String,
    pub expires_at: Option<String>,
    pub expires_date: Option<String>,
    pub days_remaining: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    #[serde(default)]
    pub model_alias: Option<String>,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    #[serde(default = "default_auto_adapt_base_url")]
    pub auto_adapt_base_url: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    /// 本地配置中的完整 Key，编辑/拉模型列表直接可用
    pub api_key: String,
    pub api_key_masked: String,
    pub model_name: String,
    pub model_alias: Option<String>,
    pub public_model_id: String,
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub status: ProviderStatus,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub auto_adapt_base_url: bool,
    /// 实验性免费通道（可删；失效后可改 Key/地址继续用）
    pub experimental: bool,
}

impl From<&ProviderConfig> for ProviderView {
    fn from(value: &ProviderConfig) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            provider: value.provider.clone(),
            base_url: value.base_url.clone(),
            api_key: value.api_key.clone(),
            api_key_masked: value.masked_key(),
            model_name: value.model_name.clone(),
            model_alias: value.model_alias.clone(),
            public_model_id: value.public_model_id(),
            protocol: value.protocol.clone(),
            enabled: value.enabled,
            status: value.status.clone(),
            last_checked_at: value.last_checked_at,
            last_error: value.last_error.clone(),
            auto_adapt_base_url: value.auto_adapt_base_url,
            experimental: crate::experimental_zen::is_experimental_zen_id(&value.id)
                || value.provider.eq_ignore_ascii_case(crate::experimental_zen::EXPERIMENTAL_ZEN_PROVIDER),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub line: String,
}

pub fn trim_base_url(mut base_url: String) -> String {
    while base_url.ends_with('/') {
        base_url.pop();
    }
    base_url
}

/// OpenAI 兼容上游常见已带版本路径的后缀（火山 /api/v3、/api/coding/v3、智谱 /paas/v4 等），不应再追加 /v1。
pub fn openai_base_has_version_suffix(base: &str) -> bool {
    let base = trim_base_url(base.to_string());
    if base.ends_with("/v1")
        || base.ends_with("/api/v3")
        || base.ends_with("/api/coding/v3")
        || base.ends_with("/coding/v3")
        || base.ends_with("/paas/v4")
        || base.ends_with("/v4")
    {
        return true;
    }
    // 火山方舟 Coding Plan 等路径以 /v3 结尾
    if base.ends_with("/v3") && (base.contains("volces.com") || base.contains("/api/")) {
        return true;
    }
    false
}

/// 修复误追加 /v1 导致的双重后缀。
pub fn repair_openai_base_url(base_url: String) -> String {
    let base = trim_base_url(base_url);
    if base.ends_with("/api/v3/v1") || base.ends_with("/api/coding/v3/v1") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/paas/v4/v1") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/v4/v1") && base.contains("/paas/") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/v3/v1") && base.contains("volces.com") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    base
}

/// 按协议规范化 Base URL（魔搭 OpenAI 需 /v1，Anthropic 原生通常不带 /v1）
pub fn normalize_base_url_for_protocol(base_url: String, protocol: &ProviderProtocol) -> String {
    let base = repair_openai_base_url(trim_base_url(base_url));
    match protocol {
        ProviderProtocol::OpenAi => {
            if openai_base_has_version_suffix(&base) {
                base
            } else {
                format!("{}/v1", base)
            }
        }
        ProviderProtocol::Anthropic => base,
    }
}

/// 保存/加载时解析 Base URL：`auto_adapt` 为 false 时仅去尾部斜杠。
pub fn resolve_provider_base_url(
    base_url: String,
    protocol: &ProviderProtocol,
    auto_adapt: bool,
) -> String {
    let trimmed = trim_base_url(base_url);
    if !auto_adapt {
        tracing::info!(
            raw = %trimmed,
            protocol = ?protocol,
            "provider base_url kept as configured (auto_adapt disabled)"
        );
        return trimmed;
    }
    let normalized = normalize_base_url_for_protocol(trimmed.clone(), protocol);
    if normalized != trimmed {
        tracing::info!(
            raw = %trimmed,
            normalized = %normalized,
            protocol = ?protocol,
            "provider base_url auto-adapted"
        );
    } else {
        tracing::debug!(
            raw = %trimmed,
            protocol = ?protocol,
            "provider base_url unchanged after auto-adapt check"
        );
    }
    normalized
}

pub fn mask_secret(secret: &str) -> String {
    if secret.len() <= 8 {
        return "********".to_string();
    }
    format!("{}****{}", &secret[..4], &secret[secret.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_protocol_aliases() {
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""openai""#).unwrap(),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""open_ai""#).unwrap(),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""anthropic""#).unwrap(),
            ProviderProtocol::Anthropic
        );
    }

    #[test]
    fn serializes_protocol_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProviderProtocol::OpenAi).unwrap(),
            r#""openai""#
        );
    }

    #[test]
    fn normalizes_openai_base_url() {
        let url = normalize_base_url_for_protocol(
            "https://api-inference.modelscope.cn".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://api-inference.modelscope.cn/v1");
    }

    #[test]
    fn keeps_volcengine_api_v3_without_extra_v1() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/v3".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3");
    }

    #[test]
    fn repairs_volcengine_api_v3_v1_suffix() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/v3/v1".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3");
    }

    #[test]
    fn keeps_volcengine_coding_v3_without_extra_v1() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn repairs_volcengine_coding_v3_v1_suffix() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/coding/v3/v1".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn resolve_skips_adapt_when_disabled() {
        let url = resolve_provider_base_url(
            "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            &ProviderProtocol::OpenAi,
            false,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn keeps_anthropic_base_without_v1() {
        let url = normalize_base_url_for_protocol(
            "https://api-inference.modelscope.cn".into(),
            &ProviderProtocol::Anthropic,
        );
        assert_eq!(url, "https://api-inference.modelscope.cn");
    }
}
