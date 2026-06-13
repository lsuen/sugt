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
        }
    }

    pub fn masked_key(&self) -> String {
        mask_secret(&self.api_key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuitBehavior {
    /// 仅退出程序，网关与接管保持不变
    #[default]
    ExitOnly,
    /// 退出并停止网关，保留接管环境变量
    StopGateway,
    /// 退出、停止网关并关闭接管
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
    pub active_model: Option<String>,
    pub active_provider: Option<String>,
    pub last_proxy_provider: Option<String>,
    pub last_proxy_path: Option<String>,
    pub last_proxy_failover: bool,
    pub last_proxy_at: Option<DateTime<Utc>>,
    pub config_dir: String,
    pub log_file: String,
    pub trial: TrialStatusView,
    pub traffic: TrafficStatsView,
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
    pub protocol: ProviderProtocol,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key_masked: String,
    pub model_name: String,
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub status: ProviderStatus,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl From<&ProviderConfig> for ProviderView {
    fn from(value: &ProviderConfig) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            provider: value.provider.clone(),
            base_url: value.base_url.clone(),
            api_key_masked: value.masked_key(),
            model_name: value.model_name.clone(),
            protocol: value.protocol.clone(),
            enabled: value.enabled,
            status: value.status.clone(),
            last_checked_at: value.last_checked_at,
            last_error: value.last_error.clone(),
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

/// 按协议规范化 Base URL（魔搭 OpenAI 需 /v1，Anthropic 原生通常不带 /v1）
pub fn normalize_base_url_for_protocol(base_url: String, protocol: &ProviderProtocol) -> String {
    let base = trim_base_url(base_url);
    match protocol {
        ProviderProtocol::OpenAi => {
            if base.ends_with("/v1") {
                base
            } else {
                format!("{}/v1", base)
            }
        }
        ProviderProtocol::Anthropic => base,
    }
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
    fn keeps_anthropic_base_without_v1() {
        let url = normalize_base_url_for_protocol(
            "https://api-inference.modelscope.cn".into(),
            &ProviderProtocol::Anthropic,
        );
        assert_eq!(url, "https://api-inference.modelscope.cn");
    }
}
