//! /v1/responses 的上游失败出口必须是 Responses 形状。
//! 作者：sun / 孙文龙
//!
//! 预算用尽或不可重试的上游失败，回 HTTP 200 的 completed 助手提示，
//! 会话还在。空 body / 非法 JSON 仍由适配器写成 status=failed，不在这里测。
//! 5xx 会在同一次请求里等待重试；测试把预算设为 0，避免睡满 8 分钟。

use axum::{
    http::{header, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use serde_json::{json, Value};
use sugt_lib::gateway::{build_router, GatewayState};
use sugt_lib::model::{AppConfig, ProviderConfig, ProviderProtocol};

async fn upstream_400() -> impl IntoResponse {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":{"message":"quota exceeded","type":"upstream_error"}}"#,
    )
}

async fn upstream_500() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":{"message":"upstream unavailable","type":"upstream_error"}}"#,
    )
}

/// 起一个真实 mock 上游 + 真实网关，返回 /v1/responses 的完整地址。
/// `retry_budget_ms` 写在这份配置上，不能用进程环境变量：cargo test 会并行。
async fn start_gateway(upstream: Router, retry_budget_ms: Option<u64>) -> String {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock 上游可绑定");
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("mock 上游运行失败");
    });

    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        format!("http://{upstream_addr}/v1"),
        "test-key",
        "mock-model",
    );
    provider.id = "e2e-upstream-failure".into();
    provider.protocol = ProviderProtocol::OpenAi;
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e-upstream-failure".into()),
        upstream_retry_budget_ms: retry_budget_ms,
        ..Default::default()
    };
    let gateway = build_router(GatewayState::new(config, None).expect("网关状态可构造"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("网关可绑定");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, gateway).await.expect("网关运行失败");
    });
    format!("http://{addr}/v1/responses")
}

/// 非流式遇上上游 400：这是终态，不按额度去等。回 completed 提示，模型名原样回显。
#[tokio::test]
async fn responses_upstream_4xx_is_a_completed_notice() {
    let url = start_gateway(Router::new().route("/*path", any(upstream_400)), None).await;
    let response = reqwest::Client::new()
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json!({ "model": "gpt-5-codex", "input": "hi", "stream": false }).to_string())
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.expect("响应体可读");
    let value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("提示必须是合法 Responses JSON：{err}，原文：{text}"));
    assert_eq!(value["status"], "completed", "会话提示不能是 failed：{value}");
    assert_eq!(value["object"], "response", "必须有 Responses 对象形状：{value}");
    assert_eq!(value["model"], "gpt-5-codex", "必须回显客户端模型名：{value}");
    assert!(value["error"].is_null(), "completed 提示的 error 必须是 null：{value}");
    let output = value["output_text"].as_str().unwrap_or("");
    assert!(
        output.contains("小su提示") && output.contains("上游拒绝"),
        "400 不能说成额度不足：{output}"
    );
}

/// 流式遇上上游 5xx：预算为 0 时立刻回 completed 事件流，不能睡满默认 8 分钟。
#[tokio::test]
async fn responses_streaming_5xx_is_a_completed_notice() {
    let url = start_gateway(Router::new().route("/*path", any(upstream_500)), Some(0)).await;
    let response = reqwest::Client::new()
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json!({ "model": "gpt-5-codex", "input": "hi", "stream": true }).to_string())
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response.text().await.expect("响应体可读");
    assert!(
        content_type.contains("text/event-stream"),
        "流式提示必须保持 SSE Content-Type，实际 {content_type}"
    );
    assert!(
        text.contains("event: response.completed"),
        "流式上游失败必须给 response.completed，原文：{text}"
    );
    assert!(
        !text.contains("response.failed"),
        "预算用尽不能再发 response.failed：{text}"
    );
    assert!(
        text.contains("小su提示") && text.contains("gpt-5-codex"),
        "提示文案和模型名都要在流里：{text}"
    );
}
