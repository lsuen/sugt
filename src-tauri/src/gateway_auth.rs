use axum::http::HeaderMap;

use crate::model::AppConfig;

pub fn validate_client_request(headers: &HeaderMap, config: &AppConfig) -> bool {
    let expected = config.gateway_client_api_key.trim();
    if expected.is_empty() {
        return true;
    }
    extract_client_api_key(headers)
        .map(|key| key == expected)
        .unwrap_or(false)
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
