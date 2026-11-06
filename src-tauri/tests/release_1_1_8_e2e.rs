//! 1.1.8 发版回归：/v1/responses 的上游失败出口必须是 Responses 形状。
//! 作者：sun / 孙文龙
//!
//! 之前只有「流式 4xx」和「HTTP 200 异常体」两条路径给了失败形状；
//! attempt 耗尽、5xx、熔断等出口统一落到通用 502 JSON，客户端按 Responses
//! 反序列化失败，报错指向「网关响应格式错误」而不是真实上游故障。

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
async fn start_gateway(upstream: Router) -> String {
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

/// DEF-006：非流式请求遇上上游 4xx，body 必须是 Responses 形状的 failed。
#[tokio::test]
async fn responses_upstream_4xx_is_reported_as_responses_failure() {
    let url = start_gateway(Router::new().route("/*path", any(upstream_400))).await;
    let response = reqwest::Client::new()
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json!({ "model": "gpt-5-codex", "input": "hi", "stream": false }).to_string())
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    let text = response.text().await.expect("响应体可读");
    let value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("失败响应必须是合法 Responses JSON：{err}，原文：{text}"));
    assert_eq!(
        value["status"], "failed",
        "上游失败不能返回通用网关错误体：{value}"
    );
    assert_eq!(
        value["object"], "response",
        "必须有 Responses 对象形状：{value}"
    );
    assert!(
        value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some(),
        "失败原因必须可诊断：{value}"
    );
}

/// DEF-006：流式请求遇上上游 5xx，必须给一段 response.failed 事件流，
/// 而不是让客户端在 SSE 通道里收到一段 JSON。
#[tokio::test]
async fn responses_streaming_5xx_is_reported_as_failed_event() {
    let url = start_gateway(Router::new().route("/*path", any(upstream_500))).await;
    let response = reqwest::Client::new()
        .post(&url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(json!({ "model": "gpt-5-codex", "input": "hi", "stream": true }).to_string())
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = response.text().await.expect("响应体可读");
    assert!(
        text.contains("\"type\":\"response.failed\"") || text.contains("event: response.failed"),
        "流式上游失败必须给 response.failed 事件，不能是通用 JSON：{text}"
    );
    assert!(
        content_type.contains("text/event-stream"),
        "流式失败也必须保持 SSE Content-Type，实际 {content_type}"
    );
}
