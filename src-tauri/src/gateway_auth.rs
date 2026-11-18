use axum::http::HeaderMap;

use crate::model::AppConfig;

/// 校验客户端请求是否被允许访问本机网关。
///
/// - 绑定回环地址时：允许无 Authorization（兼容本机 Codex 等只走配置文件的客户端）；
///   若带了 Key，则必须匹配配置或兼容占位符 `sugt-local-key`。
/// - 绑定非回环（局域网 / `0.0.0.0`）时：**必须**携带有效 `gateway_client_api_key`，
///   缺头或错误 Key 一律拒绝，避免上游凭证被局域网蹭用。
pub fn validate_client_request(headers: &HeaderMap, config: &AppConfig) -> bool {
    validate_client_request_with_keys(headers, config, &[])
}

/// 在 [`validate_client_request`] 基础上额外接受入站密钥池里的 key。
///
/// `extra_keys` 应为 `client_keys::enabled_key_strings` 的结果（已过滤停用与空值）。
/// 空字符串永不匹配，避免停用/损坏的条目意外放行。
pub fn validate_client_request_with_keys(
    headers: &HeaderMap,
    config: &AppConfig,
    extra_keys: &[String],
) -> bool {
    let expected = effective_client_key(config);
    let presented = extract_client_api_key(headers);
    let matches_extra = |key: &str| {
        extra_keys.iter().any(|extra| {
            let extra = extra.trim();
            !extra.is_empty() && extra == key
        })
    };
    if config.binds_non_loopback() {
        match presented {
            Some(key) => key == expected || matches_extra(&key),
            None => false,
        }
    } else {
        match presented {
            None => true, // Codex 等可能仅走 config.toml 鉴权，本地网关允许无头
            Some(key) => key == expected || key == DUMMY_LOCAL_KEY || matches_extra(&key),
        }
    }
}

pub const DUMMY_LOCAL_KEY: &str = "sugt-local-key";

fn effective_client_key(config: &AppConfig) -> String {
    let key = config.gateway_client_api_key.trim();
    if key.is_empty() {
        DUMMY_LOCAL_KEY.to_string()
    } else {
        key.to_string()
    }
}

pub fn extract_client_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-api-key") {
        return value.to_str().ok().map(|s| s.trim().to_string());
    }
    if let Some(value) = headers.get("authorization") {
        let raw = value.to_str().ok()?.trim();
        if let Some(token) = raw.strip_prefix("Bearer ") {
            return Some(token.trim().to_string());
        }
        if let Some(token) = raw.strip_prefix("bearer ") {
            return Some(token.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn config_local() -> AppConfig {
        AppConfig {
            gateway_client_api_key: "sugt-local-key".to_string(),
            ..AppConfig::default()
        }
    }

    fn config_lan(key: &str) -> AppConfig {
        AppConfig {
            allow_lan_access: true,
            gateway_client_api_key: key.to_string(),
            ..AppConfig::default()
        }
    }

    #[test]
    fn accepts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer sugt-local-key"),
        );
        assert!(validate_client_request(&headers, &config_local()));
    }

    #[test]
    fn accepts_missing_auth_for_local() {
        let headers = HeaderMap::new();
        assert!(validate_client_request(&headers, &config_local()));
    }

    #[test]
    fn rejects_wrong_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("wrong"));
        assert!(!validate_client_request(&headers, &config_local()));
    }

    #[test]
    fn lan_rejects_missing_auth() {
        let headers = HeaderMap::new();
        assert!(!validate_client_request(&headers, &config_lan("team-secret")));
    }

    #[test]
    fn lan_accepts_matching_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer team-secret"),
        );
        assert!(validate_client_request(&headers, &config_lan("team-secret")));
    }

    #[test]
    fn lan_rejects_wrong_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sugt-local-key"));
        assert!(!validate_client_request(&headers, &config_lan("team-secret")));
    }

    #[test]
    fn host_zero_zero_also_requires_auth() {
        let config = AppConfig {
            host: "0.0.0.0".into(),
            allow_lan_access: false,
            gateway_client_api_key: "team-secret".into(),
            ..AppConfig::default()
        };
        assert!(config.binds_non_loopback());
        assert!(!validate_client_request(&HeaderMap::new(), &config));
    }

    #[test]
    fn lan_accepts_pooled_key() {
        let extra = vec!["sk-sugt-pooled".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer sk-sugt-pooled"),
        );
        assert!(validate_client_request_with_keys(
            &headers,
            &config_lan("team-secret"),
            &extra
        ));
        // 原入口不接受池 key，行为保持不变。
        assert!(!validate_client_request(&headers, &config_lan("team-secret")));
    }

    #[test]
    fn lan_rejects_key_missing_from_extra() {
        // 停用 key 不会进入 enabled_key_strings，等价于不在 extra 里。
        let extra = vec!["sk-sugt-active".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_static("sk-sugt-disabled"),
        );
        assert!(!validate_client_request_with_keys(
            &headers,
            &config_lan("team-secret"),
            &extra
        ));
    }

    #[test]
    fn loopback_still_rejects_wrong_key_with_extra() {
        let extra = vec!["sk-sugt-pooled".to_string()];
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("sk-sugt-wrong"));
        assert!(!validate_client_request_with_keys(
            &headers,
            &config_local(),
            &extra
        ));
        // 空 key 条目不能变成万能通行证。
        let empty_extra = vec![String::new()];
        let mut blank = HeaderMap::new();
        blank.insert("x-api-key", HeaderValue::from_static(""));
        assert!(!validate_client_request_with_keys(
            &blank,
            &config_local(),
            &empty_extra
        ));
    }
}
