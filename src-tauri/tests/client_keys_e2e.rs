//! 入站客户端密钥端到端回归。
//!
//! 真实 HTTP 网关 + 本地 mock Chat Completions 上游（带 usage），验证：
//!   1. 池内 key（含主密钥）每次请求都累计 `request_count`；
//!   2. `X-SUGT-Client` 不同的两次请求形成两个客户端条目；
//!   3. 上游 usage 累加到 `total_tokens`；
//!   4. 错误 key 返回 401，且不产生用量。
//! 每个用例使用独立临时目录，绝不触碰真实 config_dir。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use serde_json::json;
use sugt_lib::client_keys::{
    save as save_keys, ClientKey, ClientKeyUsage, ClientKeysFile, ClientQuota,
};
use sugt_lib::gateway::{build_router, GatewayState};
use sugt_lib::model::{AppConfig, ProviderConfig, ProviderProtocol};

type Captured = Arc<Mutex<Vec<String>>>;

/// 独立临时目录：pid + 纳秒，Windows 也安全。
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("sugt-clientkeys-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录可创建");
    dir
}

fn entry(id: &str, name: &str, key: &str, primary: bool) -> ClientKey {
    ClientKey {
        id: id.into(),
        name: name.into(),
        key: key.into(),
        enabled: true,
        primary,
        quota: ClientQuota::default(),
        usage: ClientKeyUsage::default(),
    }
}

/// 池里已有 primary，`ensure_primary` 不会改动；`sk-team` 是非主密钥。
fn write_keys(dir: &PathBuf) {
    save_keys(
        dir,
        &ClientKeysFile {
            version: 1,
            keys: vec![
                entry("config", "默认", "sk-config", true),
                entry("team", "协作机", "sk-team", false),
            ],
        },
    )
    .expect("入站密钥文件可写入");
}

fn app_config(upstream: std::net::SocketAddr) -> AppConfig {
    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        format!("http://{upstream}/v1"),
        "config-key",
        "mock-model",
    );
    provider.id = "e2e".into();
    provider.protocol = ProviderProtocol::OpenAi;
    AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        providers: vec![provider],
        active_provider_id: Some("e2e".into()),
        gateway_client_api_key: "sk-config".into(),
        upstream_retry_budget_ms: Some(0),
        ..Default::default()
    }
}

/// 启动 mock 上游与真实网关，返回网关地址与同一份网关状态（用于读取内存用量）。
async fn start_gateway(
    dir: &PathBuf,
    upstream: Router,
) -> (std::net::SocketAddr, GatewayState) {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock 上游可绑定");
    let upstream_addr = upstream_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(upstream_listener, upstream)
            .await
            .expect("mock 上游运行失败");
    });

    let state = GatewayState::new(app_config(upstream_addr), Some(dir.join("config.toml")))
        .expect("网关状态可构造");
    let gateway = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("网关可绑定");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            gateway.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("网关运行失败");
    });
    (addr, state)
}

fn push_auth(captured: &Captured, headers: &HeaderMap) {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    captured
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(auth);
}

/// 成功上游：记录 Authorization，按 Chat Completions 返回固定 usage（30 + 5）。
async fn ok_upstream(State(captured): State<Captured>, headers: HeaderMap) -> impl IntoResponse {
    push_auth(&captured, &headers);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "id": "chatcmpl-clientkeys",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "我在" }
            }],
            "usage": { "prompt_tokens": 30, "completion_tokens": 5 }
        })
        .to_string(),
    )
}

async fn chat(
    client: &reqwest::Client,
    addr: std::net::SocketAddr,
    key: &str,
    label: &str,
) -> (StatusCode, String) {
    let mut request = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-sugt-client", label)
        .body(
            json!({
                "model": "mock-model",
                "messages": [{ "role": "user", "content": "你好" }],
                "stream": false
            })
            .to_string(),
        );
    if !key.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
    }
    let response = request.send().await.expect("网关必须响应");
    let status = response.status();
    (status, response.text().await.unwrap_or_default())
}

async fn usage_of(state: &GatewayState, id: &str) -> ClientKeyUsage {
    let file = state.client_keys_snapshot().await;
    file.keys
        .iter()
        .find(|key| key.id == id)
        .unwrap_or_else(|| panic!("池里应存在 {id}"))
        .usage
        .clone()
}

/// token 归集走 `tokio::spawn`，给它一点时间落回内存。
async fn wait_for_tokens(state: &GatewayState, id: &str, expected: u64) -> ClientKeyUsage {
    for _ in 0..60 {
        let usage = usage_of(state, id).await;
        if usage.total_tokens == expected {
            return usage;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    usage_of(state, id).await
}

#[tokio::test]
async fn pool_key_requests_record_clients_and_tokens() {
    let dir = temp_dir("usage");
    write_keys(&dir);

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    for label in ["workstation-a", "workstation-b"] {
        let (status, body) = chat(&client, addr, "sk-team", label).await;
        assert!(status.is_success(), "网关应返回成功，实际 {status}: {body}");
    }
    // 主密钥同样在池内，必须被计数（旧实现会跳过 config key）。
    let (status, body) = chat(&client, addr, "sk-config", "local-admin").await;
    assert!(status.is_success(), "主密钥请求应成功，实际 {status}: {body}");

    let team = wait_for_tokens(&state, "team", 70).await;
    assert_eq!(team.request_count, 2, "两次请求都应计数");
    assert_eq!(team.total_tokens, 70, "usage 30+5 应累加两次");
    assert_eq!(team.clients.len(), 2, "两个 X-SUGT-Client 应是两个条目");
    let labels: Vec<&str> = team.clients.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"workstation-a"), "客户端条目：{labels:?}");
    assert!(labels.contains(&"workstation-b"), "客户端条目：{labels:?}");
    for item in &team.clients {
        assert_eq!(item.request_count, 1, "每个客户端各 1 次请求");
        assert_eq!(item.total_tokens, 35, "每个客户端各 35 tokens");
        assert_eq!(item.ip, "127.0.0.1", "应记录真实连接 IP");
        assert!(!item.last_seen_at.is_empty(), "应记录 lastSeenAt");
    }
    // 按 last_seen_at 倒序。
    assert!(
        team.clients[0].last_seen_at >= team.clients[1].last_seen_at,
        "clients 应按 lastSeenAt 倒序"
    );

    let primary = wait_for_tokens(&state, "config", 35).await;
    assert_eq!(primary.request_count, 1, "主密钥必须计数");
    assert_eq!(primary.total_tokens, 35);
    assert_eq!(primary.clients.len(), 1);
    assert_eq!(primary.clients[0].label, "local-admin");
    assert!(!primary.clients[0].last_seen_at.is_empty());

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 3, "三次请求都应打到上游");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn invalid_key_is_rejected_without_usage() {
    let dir = temp_dir("deny");
    write_keys(&dir);

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    let (status, _body) = chat(&client, addr, "sk-wrong", "intruder").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "错误 key 应 401");

    let team = usage_of(&state, "team").await;
    assert_eq!(team.request_count, 0, "拒绝的请求不应计数");
    assert!(team.clients.is_empty());
    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(auths.is_empty(), "错误 key 不应触达上游");

    let _ = std::fs::remove_dir_all(&dir);
}
