use axum::http::HeaderMap;

use crate::model::AppConfig;

/// 校验客户端请求是否被允许访问本机网关。
///
/// - 绑定回环地址时：允许无 Authorization（兼容本机 Codex 等只走配置文件的客户端）；
///   若带了 Key，则必须匹配配置或兼容占位符 `sugt-local-key`。
/// - 绑定非回环（局域网 / `0.0.0.0`）时：**必须**携带有效 `gateway_client_api_key`，
///   缺头或错误 Key 一律拒绝，避免上游凭证被局域网蹭用。
pub fn validate_client_request(headers: &HeaderMap, config: &AppConfig) -> bool {
    let expected = effective_client_key(config);
    let presented = extract_client_api_key(headers);
    if config.binds_non_loopback() {
        match presented {
            Some(key) => key == expected,
            None => false,
        }
    } else {
        match presented {
            None => true, // Codex 等可能仅走 config.toml 鉴权，本地网关允许无头
            Some(key) => key == expected || key == DUMMY_LOCAL_KEY,
        }
    }
}

const DUMMY_LOCAL_KEY: &str = "sugt-local-key";

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
}
