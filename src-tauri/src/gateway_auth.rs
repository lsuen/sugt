use axum::http::HeaderMap;

use crate::model::AppConfig;

pub fn validate_client_request(headers: &HeaderMap, config: &AppConfig) -> bool {
    let expected = effective_client_key(config);
    match extract_client_api_key(headers) {
        None => true, // Codex 等可能仅走 config.toml 鉴权，本地网关允许无头
        Some(key) => key == expected || key == DUMMY_LOCAL_KEY,
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

    #[test]
    fn accepts_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer sugt-local-key"),
        );
        let config = AppConfig {
            gateway_client_api_key: "sugt-local-key".to_string(),
            ..AppConfig::default()
        };
        assert!(validate_client_request(&headers, &config));
    }

    #[test]
    fn accepts_missing_auth_for_local() {
        let headers = HeaderMap::new();
        let config = AppConfig::default();
        assert!(validate_client_request(&headers, &config));
    }

    #[test]
    fn rejects_wrong_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("wrong"));
        let config = AppConfig {
            gateway_client_api_key: "sugt-local-key".to_string(),
            ..AppConfig::default()
        };
        assert!(!validate_client_request(&headers, &config));
    }
}
