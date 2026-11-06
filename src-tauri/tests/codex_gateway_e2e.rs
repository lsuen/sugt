//! Codex /v1/responses 端到端回归：真实 HTTP 请求 + 本地 mock Chat Completions 上游。
//!
//! 覆盖完整两轮工具闭环：
//!   1. Codex 风格请求（function + namespace 工具）→ 网关转发 → 上游返回 tool_calls
//!      → 网关还原成 Responses function_call（而不是 custom_tool_call）
//!   2. 客户端把工具结果回灌 → 网关还原成 Chat role=tool → 上游收到结构化工具结果
//! 作者：sun / 孙文龙

use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use sugt_lib::gateway::{build_router, GatewayState};
use sugt_lib::model::{AppConfig, ProviderConfig, ProviderProtocol};

type Captured = Arc<Mutex<Vec<Value>>>;

async fn upstream_handler(State(captured): State<Captured>, body: Bytes) -> impl IntoResponse {
    let parsed: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let turn = {
        let mut guard = captured.lock().unwrap_or_else(|e| e.into_inner());
        guard.push(parsed);
        guard.len()
    };

    let response = if turn == 1 {
        // 第一轮：上游按 Chat Completions 返回两个工具调用，其中一个是展平名。
        json!({
            "id": "chatcmpl-e2e-1",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [
                        {
                            "id": "call_shell",
                            "type": "function",
                            "function": {
                                "name": "shell_command",
                                "arguments": "{\"command\":\"ls\"}"
                            }
                        },
                        {
                            "id": "call_ns",
                            "type": "function",
                            "function": {
                                "name": "functions__exec_command",
                                "arguments": "{\"cmd\":\"pwd\"}"
                            }
                        }
                    ]
                }
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 8 }
        })
    } else {
        // 第二轮：普通文本回复，验证回灌后仍能得到一次正常响应。
        json!({
            "id": "chatcmpl-e2e-2",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "完成" }
            }],
            "usage": { "prompt_tokens": 30, "completion_tokens": 3 }
        })
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        response.to_string(),
    )
}

fn codex_tools() -> Value {
    json!([
        {
            "type": "function",
            "name": "shell_command",
            "description": "Run a shell command",
            "parameters": {
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }
        },
        {
            "type": "namespace",
            "name": "functions",
            "tools": [{
                "type": "function",
                "name": "exec_command",
                "description": "Run a command",
                "parameters": {
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } }
                }
            }]
        }
    ])
}

const STREAM_UPSTREAM: &str = concat!(
    r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_s","type":"function","function":{"name":"shell_command","arguments":"{\"command\":\"ls\"}"}}]}}]}"#,
    "\n\n",
    r#"data: {"choices":[{"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#,
    "\n\n",
    "data: [DONE]\n\n"
);

async fn streaming_upstream_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/event-stream")],
        STREAM_UPSTREAM,
    )
}

async fn post_responses(client: &reqwest::Client, url: &str, body: &Value) -> Value {
    let response = client
        .post(url)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    let status = response.status();
    let text = response.text().await.expect("响应体可读");
    assert!(status.is_success(), "网关返回 {status}，响应体：{text}");
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("响应不是合法 JSON：{err}，原文：{text}"))
}

#[tokio::test]
async fn codex_tool_loop_survives_a_real_http_roundtrip() {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(upstream_handler))
        .with_state(captured.clone());
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
    provider.id = "e2e".into();
    provider.protocol = ProviderProtocol::OpenAi;
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e".into()),
        ..Default::default()
    };
    let state = GatewayState::new(config, None).expect("网关状态可构造");
    let gateway = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("网关可绑定");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, gateway).await.expect("网关运行失败");
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/v1/responses");
    let first_request = json!({
        "model": "gpt-5-codex",
        "instructions": "You are a coding agent.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "列出当前目录" }]
        }],
        "tools": codex_tools(),
        "tool_choice": "auto",
        "stream": false
    });

    let first = post_responses(&client, &url, &first_request).await;
    let output = first["output"].as_array().expect("output 应为数组");
    let calls: Vec<&Value> = output
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect();
    assert_eq!(
        calls.len(),
        2,
        "两个工具调用都应还原成 function_call：{first}"
    );
    assert!(calls.iter().any(|c| c["name"] == "shell_command"));
    assert!(
        calls.iter().any(|c| c["name"] == "exec_command"),
        "namespace 展平名必须还原成客户端原名：{first}"
    );
    assert!(
        output.iter().all(|item| item["type"] != "custom_tool_call"),
        "官方 function 工具不能被误判成 freeform custom：{first}"
    );
    assert_eq!(first["error"], Value::Null);
    assert!(first.get("created_at").is_some());

    // 第二轮：把第一轮的调用与工具结果按 Codex 形态回灌。
    let mut second_input = vec![first_request["input"][0].clone()];
    for call in &calls {
        second_input.push(json!({
            "type": "function_call",
            "call_id": call["call_id"],
            "name": call["name"],
            "arguments": call["arguments"]
        }));
        second_input.push(json!({
            "type": "function_call_output",
            "call_id": call["call_id"],
            "output": "ok"
        }));
    }
    let second_request = json!({
        "model": "gpt-5-codex",
        "input": second_input,
        "tools": codex_tools(),
        "stream": false
    });
    let second = post_responses(&client, &url, &second_request).await;
    assert!(
        second["output_text"]
            .as_str()
            .unwrap_or("")
            .contains("完成"),
        "回灌后仍应拿到正常回复：{second}"
    );

    let guard = captured.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(guard.len(), 2, "mock 上游应收到两轮请求");
    let first_chat = &guard[0];
    let upstream_names: Vec<&str> = first_chat["tools"]
        .as_array()
        .expect("上游请求应包含 tools")
        .iter()
        .filter_map(|t| t.pointer("/function/name").and_then(Value::as_str))
        .collect();
    assert!(upstream_names.contains(&"shell_command"));
    assert!(
        upstream_names.contains(&"functions__exec_command"),
        "namespace 工具应展平后交给上游：{upstream_names:?}"
    );

    let second_messages = guard[1]["messages"]
        .as_array()
        .expect("第二轮应有 messages");
    for call_id in ["call_shell", "call_ns"] {
        assert!(
            second_messages
                .iter()
                .any(|m| m["role"] == "tool" && m["tool_call_id"] == call_id),
            "工具结果 {call_id} 必须以 role=tool 结构化回传：{second_messages:?}"
        );
    }
    assert!(
        second_messages
            .iter()
            .any(|m| m["role"] == "assistant" && m["tool_calls"].is_array()),
        "历史工具调用必须保留：{second_messages:?}"
    );
    // 客户端回灌的是还原后的原名 exec_command；发给上游时必须再展平，
    // 否则多轮后 tools 与历史名不一致，模型易混用 command/cmd 参数。
    let history_names: Vec<&str> = second_messages
        .iter()
        .filter(|m| m["role"] == "assistant")
        .filter_map(|m| m["tool_calls"].as_array())
        .flatten()
        .filter_map(|c| c.pointer("/function/name").and_then(Value::as_str))
        .collect();
    assert!(
        history_names.contains(&"functions__exec_command"),
        "第二轮历史里的 namespace 工具必须展平：{history_names:?} / {second_messages:?}"
    );
    assert!(
        history_names.contains(&"shell_command"),
        "非 namespace 工具名应保持原样：{history_names:?}"
    );
}

#[tokio::test]
async fn codex_streaming_tool_call_keeps_the_full_event_sequence() {
    let upstream = Router::new().route("/*path", any(streaming_upstream_handler));
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("流式 mock 上游可绑定");
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("流式 mock 上游运行失败");
    });

    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        format!("http://{upstream_addr}/v1"),
        "test-key",
        "mock-model",
    );
    provider.id = "e2e-stream".into();
    provider.protocol = ProviderProtocol::OpenAi;
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e-stream".into()),
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

    let request = json!({
        "model": "gpt-5-codex",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "列出当前目录" }]
        }],
        "tools": codex_tools(),
        "stream": true
    });
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/responses"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(request.to_string())
        .send()
        .await
        .expect("网关必须响应流式 /v1/responses");
    assert!(response.status().is_success());
    let stream = response.text().await.expect("SSE 响应体可读");

    assert!(stream.contains("event: response.created"), "流：{stream}");
    assert!(
        stream.contains("event: response.in_progress"),
        "流：{stream}"
    );
    assert!(
        stream.contains("event: response.function_call_arguments.delta"),
        "缺参数增量事件：{stream}"
    );
    assert!(
        stream.contains("event: response.function_call_arguments.done"),
        "缺参数结束事件，客户端会一直挂在 in_progress：{stream}"
    );
    assert!(
        stream.contains("\"type\":\"function_call\""),
        "流：{stream}"
    );
    assert!(
        !stream.contains("\"type\":\"custom_tool_call\""),
        "shell_command 不能被判成 freeform：{stream}"
    );
    assert!(stream.contains("event: response.completed"), "流：{stream}");
    assert!(stream.contains("data: [DONE]"), "流：{stream}");
}

/// 上游返回纯文本（非法 JSON）时，网关过去会伪装成一条空 completed 回复，
/// 客户端以为模型「正常地什么都没说」，工具闭环静默断掉且无从排查。
async fn malformed_upstream_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        "upstream returned plain text, not json",
    )
}

#[tokio::test]
async fn malformed_upstream_body_is_reported_as_responses_failure() {
    let upstream = Router::new().route("/*path", any(malformed_upstream_handler));
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("异常 mock 上游可绑定");
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("异常 mock 上游运行失败");
    });

    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        format!("http://{upstream_addr}/v1"),
        "test-key",
        "mock-model",
    );
    provider.id = "e2e-malformed".into();
    provider.protocol = ProviderProtocol::OpenAi;
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e-malformed".into()),
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

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/responses"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(
            json!({
                "model": "gpt-5-codex",
                "input": "hi",
                "stream": false
            })
            .to_string(),
        )
        .send()
        .await
        .expect("网关必须响应 /v1/responses");
    let text = response.text().await.expect("响应体可读");
    let value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("失败响应也必须是合法 Responses JSON：{err}，原文：{text}"));
    assert_eq!(
        value["status"], "failed",
        "异常上游不能伪装成 completed：{value}"
    );
    assert!(
        value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("非法 JSON")),
        "失败原因必须可诊断：{value}"
    );
}

/// 部分兼容层会在 HTTP 200 的正常响应里附带 `"error": null`；
/// 过去 `is_openai_compat_error_body` 只要看到 error 键就判失败，整轮对话被吞掉。
async fn error_null_upstream_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        concat!(
            r#"{"error":null,"choices":[{"index":0,"finish_reason":"stop","#,
            r#""message":{"role":"assistant","content":"正常回复"}}],"#,
            r#""usage":{"prompt_tokens":1,"completion_tokens":2}}"#
        ),
    )
}

#[tokio::test]
async fn error_null_upstream_body_is_not_reported_as_failure() {
    let upstream = Router::new().route("/*path", any(error_null_upstream_handler));
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("error-null mock 上游可绑定");
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("error-null mock 上游运行失败");
    });

    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        format!("http://{upstream_addr}/v1"),
        "test-key",
        "mock-model",
    );
    provider.id = "e2e-error-null".into();
    provider.protocol = ProviderProtocol::OpenAi;
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e-error-null".into()),
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

    let request = json!({
        "model": "gpt-5-codex",
        "input": [{ "type": "message", "role": "user", "content": "hi" }],
        "stream": false
    });
    let value = post_responses(
        &reqwest::Client::new(),
        &format!("http://{addr}/v1/responses"),
        &request,
    )
    .await;
    assert_eq!(
        value["status"], "completed",
        "error: null 是正常回复，不能被误判为失败：{value}"
    );
    assert_eq!(value["error"], Value::Null, "不应带 error 对象：{value}");
    assert!(
        value["output_text"]
            .as_str()
            .unwrap_or("")
            .contains("正常回复"),
        "正文必须透传：{value}"
    );
}
