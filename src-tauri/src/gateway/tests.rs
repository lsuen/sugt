//! `gateway` 模块单元测试（由 gateway.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use super::*;
use crate::anthropic_adapter::{anthropic_to_openai, openai_json_to_anthropic_json};

#[test]
fn rewrites_json_model() {
    let body = Bytes::from_static(br#"{"model":"old","messages":[]}"#);
    let rewritten = rewrite_model(body, "new-model");
    let value: Value = serde_json::from_slice(&rewritten).unwrap();
    assert_eq!(value["model"], "new-model");
}

#[test]
fn maps_volcengine_coding_v3_responses() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/coding/v3",
        "v1/responses",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/coding/v3/responses"
    );
}

#[test]
fn maps_volcengine_coding_v3_chat_completions() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/coding/v3",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
    );
}

#[test]
fn maps_volcengine_v3_chat_completions() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/v3",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
    );
}

#[test]
fn maps_local_v1_path_to_provider_base() {
    let target = build_target_url("https://example.com/v1", "v1/chat/completions", "?x=1");
    assert_eq!(target, "https://example.com/v1/chat/completions?x=1");

    let messages = build_target_url("https://example.com/v1", "v1/messages", "");
    assert_eq!(messages, "https://example.com/v1/messages");
}

#[test]
fn maps_modelscope_anthropic_base_without_v1_suffix() {
    let messages = build_target_url("https://api-inference.modelscope.cn", "v1/messages", "");
    assert_eq!(messages, "https://api-inference.modelscope.cn/v1/messages");

    let chat = build_target_url(
        "https://api-inference.modelscope.cn/v1",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        chat,
        "https://api-inference.modelscope.cn/v1/chat/completions"
    );
}

#[test]
fn converts_anthropic_messages_to_openai_chat() {
    let body = Bytes::from_static(br#"{"system":"be concise","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"max_tokens":8,"temperature":0.2,"stream":true}"#);
    let payload = anthropic_to_openai(body, "target-model").unwrap();
    let value: Value = serde_json::from_slice(&payload).unwrap();

    assert_eq!(value["model"], "target-model");
    assert_eq!(value["messages"][0]["role"], "system");
    assert_eq!(value["messages"][0]["content"], "be concise");
    assert_eq!(value["messages"][1]["role"], "user");
    assert_eq!(value["messages"][1]["content"], "hello");
    assert_eq!(value["max_tokens"], 8);
    assert_eq!(value["stream"], true);
}

#[test]
fn converts_openai_chat_to_anthropic_message() {
    let response = br#"{"id":"chatcmpl-1","model":"target-model","choices":[{"message":{"role":"assistant","content":"pong"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
    let value = openai_json_to_anthropic_json(response).unwrap();

    assert_eq!(value["type"], "message");
    assert_eq!(value["role"], "assistant");
    assert_eq!(value["content"][0]["text"], "pong");
    assert_eq!(value["usage"]["input_tokens"], 3);
    assert_eq!(value["usage"]["output_tokens"], 4);
}

#[test]
fn free_channel_attempts_follow_failover_switch() {
    let free = crate::experimental_free::make_experimental_free_provider();
    let on = upstream_attempts(&free, true);
    assert!(on.len() > 1, "failover on → multi free routes");
    let off = upstream_attempts(&free, false);
    assert_eq!(off.len(), 1, "failover off → only selected model");
    assert_eq!(off[0].model_name, free.model_name);
}

#[test]
fn orders_failover_providers() {
    let mut config = AppConfig::default();
    let first = ProviderConfig::new("a", "p", "https://a/v1", "k", "m1");
    let second = ProviderConfig::new("b", "p", "https://b/v1", "k", "m2");
    config.active_provider_id = Some(second.id.clone());
    config.providers.push(first);
    config.providers.push(second);
    let providers = ordered_providers(&config);
    assert_eq!(providers[0].model_name, "m2");
    assert_eq!(providers[1].model_name, "m1");
    config.failover_enabled = false;
    let only = ordered_providers(&config);
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].model_name, "m2");
}
