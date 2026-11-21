//! WebUI 端到端测试：真实起 HTTP 服务，验证鉴权链路与 RPC 通道。
//! 沿用项目 *_e2e.rs 惯例（见 codex_gateway_e2e.rs）。

use serde_json::{json, Value};

fn unique_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static PORT: AtomicU16 = AtomicU16::new(18700);
    PORT.fetch_add(1, Ordering::Relaxed)
}

async fn post(url: &str, body: &Value, token: Option<&str>) -> (u16, Value) {
    let client = reqwest::Client::new();
    let mut req = client.post(url).json(body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.expect("request failed");
    let status = resp.status().as_u16();
    let json: Value = resp.json().await.unwrap_or(Value::Null);
    (status, json)
}

#[tokio::test]
async fn health_is_public() {
    // 只验证 health handler 不需要 token：直接构造状态起服务。
    let runtime = sugt_lib::commands::AppRuntime::load_from_disk().expect("runtime");
    let app = sugt_lib::webui::api::build_router(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/health", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn invoke_requires_token() {
    let runtime = sugt_lib::commands::AppRuntime::load_from_disk().expect("runtime");
    let app = sugt_lib::webui::api::build_router(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("http://{}/api/invoke", addr);
    let (status, body) = post(&url, &json!({"cmd":"get_config","args":{}}), None).await;
    assert_eq!(status, 401);
    assert_eq!(body["ok"], false);

    // 伪造 token 也拒绝
    let (status, _) = post(&url, &json!({"cmd":"get_config"}), Some("forged.token")).await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn token_and_invoke_full_path() {
    let runtime = sugt_lib::commands::AppRuntime::load_from_disk().expect("runtime");
    let key = runtime.config.read().await.gateway_client_api_key.clone();
    let app = sugt_lib::webui::api::build_router(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{}", addr);

    // 1. 错误 key 登录失败
    let (status, _) = post(&format!("{}/api/login", base), &json!({"key":"wrong-key"}), None).await;
    assert_eq!(status, 401);

    // 2. 正确 key 换 token
    let (status, login) = post(&format!("{}/api/login", base), &json!({"key":key}), None).await;
    assert_eq!(status, 200, "login failed: {:?}", login);
    let token = login["token"].as_str().expect("token").to_string();

    // 3. 带 token 调 RPC（只读命令）
    let (status, result) = post(
        &format!("{}/api/invoke", base),
        &json!({"cmd":"get_config","args":{}}),
        Some(&token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(result["ok"], true, "invoke failed: {:?}", result);
    assert!(result["data"].is_object());
}

#[tokio::test]
async fn unknown_command_returns_error_not_panic() {
    let runtime = sugt_lib::commands::AppRuntime::load_from_disk().expect("runtime");
    let key = runtime.config.read().await.gateway_client_api_key.clone();
    let app = sugt_lib::webui::api::build_router(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{}", addr);
    let (_, login) = post(&format!("{}/api/login", base), &json!({"key":key}), None).await;
    let token = login["token"].as_str().unwrap();
    let (status, body) = post(
        &format!("{}/api/invoke", base),
        &json!({"cmd":"__no_such_command__","args":{}}),
        Some(token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().contains("不支持"));
}

#[test]
fn unique_ports_are_distinct() {
    assert_ne!(unique_port(), unique_port());
}
