//! WebUI 鉴权：客户端 API Key 换取短期 HMAC 会话令牌。
//!
//! 复用 `gateway_auth` 的 key 校验语义：LAN 绑定强制 key，且接受 key 池内所有启用 key。
//! 令牌用 sha2 做 HMAC，密钥派生自 `gateway_client_api_key`——key 一改，全部会话立即失效。

use axum::http::HeaderMap;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// 会话有效期（秒）。局域网管理场景 12 小时足够，过期需重新输 key。
const SESSION_TTL_SECS: u64 = 12 * 60 * 60;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn secret(config: &crate::model::AppConfig) -> Vec<u8> {
    let mut seed = format!("sugt-webui|{}", config.gateway_client_api_key).into_bytes();
    seed.extend_from_slice(config.host.as_bytes());
    seed
}

fn sign(secret: &[u8], payload: &str) -> String {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret)
        .expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// 用已校验通过的客户端 key 换取会话令牌。调用前必须先用
/// `validate_webui_key` 校验过 key，否则等于把令牌发给任何人。
pub fn issue_token(config: &crate::model::AppConfig) -> (String, u64) {
    let exp = now_secs() + SESSION_TTL_SECS;
    let payload = exp.to_string();
    let token = format!("{}.{}", payload, sign(&secret(config), &payload));
    (token, SESSION_TTL_SECS)
}

/// 校验会话令牌：格式、签名（constant-time）、过期时间。
pub fn verify_token(token: &str, config: &crate::model::AppConfig) -> bool {
    let (exp_str, sig) = match token.split_once('.') {
        Some(pair) => pair,
        None => return false,
    };
    let expected = sign(&secret(config), exp_str);
    let sig_matches: bool = subtle::ConstantTimeEq::ct_eq(sig.as_bytes(), expected.as_bytes()).into();
    if !sig_matches {
        return false;
    }
    let Ok(exp) = exp_str.parse::<u64>() else {
        return false;
    };
    exp > now_secs()
}

/// 校验登录页提交的 key：复用网关鉴权，接受配置主 key 与 key 池内所有启用 key。
pub async fn validate_webui_key(
    headers: &HeaderMap,
    config: &crate::model::AppConfig,
    runtime: &crate::commands::AppRuntime,
) -> bool {
    // 交给既有网关鉴权逻辑判定，避免重复实现 key 提取与 LAN 强制规则。
    // extra keys 取 client key 池内所有启用项，语义与网关入站完全一致。
    let snapshot = runtime.gateway.client_keys_snapshot().await;
    let enabled = crate::client_keys::enabled_key_strings(&snapshot);
    crate::gateway_auth::validate_client_request_with_keys(headers, config, &enabled)
}

/// 从请求头提取 Bearer 令牌。
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(key: &str) -> crate::model::AppConfig {
        crate::model::AppConfig {
            gateway_client_api_key: key.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn token_roundtrip() {
        let config = cfg("test-key");
        let (token, ttl) = issue_token(&config);
        assert_eq!(ttl, SESSION_TTL_SECS);
        assert!(verify_token(&token, &config));
    }

    #[test]
    fn key_change_invalidates_token() {
        let config = cfg("old-key");
        let (token, _) = issue_token(&config);
        assert!(!verify_token(&token, &cfg("new-key")));
    }

    #[test]
    fn tampered_token_rejected() {
        let config = cfg("test-key");
        let (token, _) = issue_token(&config);
        let tampered = format!("{}x", token);
        assert!(!verify_token(&tampered, &config));
        assert!(!verify_token("garbage", &config));
        assert!(!verify_token("", &config));
    }
}
