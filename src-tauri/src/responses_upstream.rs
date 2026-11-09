//! Responses 上游能力判定与请求改写（由 openai_responses_adapter.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use anyhow::Result;
use bytes::Bytes;
use serde_json::{json, Value};

/// 判断上游是否为支持原生 Responses 的火山方舟 Plan 端点（coding / plan / 通用 v3）。
pub fn provider_supports_native_responses(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/').to_lowercase();
    let is_volc = base.contains("volces.com") || base.contains("volcengine");
    let has_api_path = base.contains("/api/coding")
        || base.contains("/api/plan")
        || base.ends_with("/api/v3")
        || base.contains("/coding/v3")
        || base.contains("/plan/v3");
    is_volc && has_api_path
}

/// 是否尝试原生 Responses 透传（仅无 tools 的纯文本轮次）。
///
/// 火山方舟 `/responses` 常对带 tools 的请求返回 200 + 纯文本「我去做 X」，
/// 却不发出 function/custom tool call，Codex 会当成正常收工——表现为「自己停了」。
/// 兼容路径是 Responses→Chat Completions 桥，保留工具环；原生透传只留给无工具请求。
pub fn should_try_native_responses(base_url: &str, model: &str) -> bool {
    provider_supports_native_responses(base_url) || is_volcengine_coding_model(model)
}

/// Codex `/v1/responses` 请求体是否声明了非空 `tools`。
pub fn responses_body_declares_tools(body: &Bytes) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("tools").and_then(Value::as_array).cloned())
        .map(|t| !t.is_empty())
        .unwrap_or(false)
}

/// 最终是否走原生 Responses：上游看似支持，且本轮没有 tools。
pub fn should_try_native_responses_for_body(base_url: &str, model: &str, body: &Bytes) -> bool {
    should_try_native_responses(base_url, model) && !responses_body_declares_tools(body)
}

pub fn is_volcengine_coding_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("ark-code") || m == "ark-code-latest"
}

/// 原生 Responses 一律使用用户配置的 Base URL，不硬编码改写到某一家默认端点。
pub fn native_responses_base_url(base_url: &str, _model: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
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
