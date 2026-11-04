//! Responses 上游能力判定与请求改写（由 openai_responses_adapter.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use anyhow::Result;
use bytes::Bytes;
use serde_json::{json, Value};
use tracing::warn;

/// 判断上游是否为支持原生 Responses 的火山方舟 Coding Plan 端点。
pub fn provider_supports_native_responses(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/').to_lowercase();
    let is_volc = base.contains("volces.com") || base.contains("volcengine");
    let has_api_path =
        base.contains("/api/coding") || base.ends_with("/api/v3") || base.contains("/coding/v3");
    is_volc && has_api_path
}

/// Codex 接入 ark-code 等模型时，即使 Base URL 未指向 Coding Plan 也尝试原生 Responses。
pub fn should_try_native_responses(base_url: &str, model: &str) -> bool {
    provider_supports_native_responses(base_url) || is_volcengine_coding_model(model)
}

pub fn is_volcengine_coding_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("ark-code") || m == "ark-code-latest"
}

/// 解析应当使用的原生 Responses Base URL。
pub fn native_responses_base_url(base_url: &str, model: &str) -> String {
    if provider_supports_native_responses(base_url) {
        base_url.trim().trim_end_matches('/').to_string()
    } else if is_volcengine_coding_model(model) {
        warn!(
            configured_base = base_url,
            model = model,
            "ark-code model detected but base_url is not volcengine coding/v3; using default Coding Plan endpoint"
        );
        "https://ark.cn-beijing.volces.com/api/coding/v3".to_string()
    } else {
        base_url.trim().trim_end_matches('/').to_string()
    }
}

/// 把 Responses 请求体中的 model 改写为实际上游模型，保留 tools / input 等内容。
pub fn rewrite_responses_upstream_model(body: &Bytes, upstream_model: &str) -> Result<Bytes> {
    let mut value: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body)?
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert("model".to_string(), json!(upstream_model));
    }
    Ok(Bytes::from(serde_json::to_vec(&value)?))
}
