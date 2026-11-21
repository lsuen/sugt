//! API Key 池端到端回归（PRD v1.1.11 T5.2 / T5.3 的验收证据）。
//!
//! 真实 HTTP 网关 + 本地 mock Chat Completions 上游，验证：
//!   1. roundRobin 按顺序轮换、跳过停用 key，并逐 key 累计请求数与 token；
//!   2. leastUsed 选择请求数最少的 key；
//!   3. 上游 5xx 时把最近错误写回该 key。
//! 每个用例都用独立临时目录，绝不触碰真实 config_dir。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use serde_json::json;
use sugt_lib::api_key_pool::{
    save, ApiKeyEntry, ApiKeyPool, ApiKeyPoolsFile, KeySelection, KeyUsage,
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
    let dir = std::env::temp_dir().join(format!("sugt-keypool-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录可创建");
    dir
}

fn entry(id: &str, key: &str, enabled: bool, request_count: u64) -> ApiKeyEntry {
    ApiKeyEntry {
        id: id.into(),
        key: key.into(),
        enabled,
        usage: KeyUsage {
            request_count,
            total_tokens: 0,
            last_error: None,
        },
        quota: None,
    }
}

fn pool(selection: KeySelection, keys: Vec<ApiKeyEntry>) -> ApiKeyPool {
    ApiKeyPool {
        id: "pool-e2e".into(),
        name: "测试池".into(),
        provider_id: "e2e".into(),
        selection,
        keys,
    }
}

fn write_pool(dir: &PathBuf, pool: ApiKeyPool) {
    save(
        dir,
        &ApiKeyPoolsFile {
            version: 1,
            pools: vec![pool],
        },
    )
    .expect("池文件可写入");
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
        axum::serve(listener, gateway).await.expect("网关运行失败");
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
            "id": "chatcmpl-keypool",
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

/// 失败上游：始终 500，用来验证 last_error 回写。
async fn failing_upstream(State(captured): State<Captured>, headers: HeaderMap) -> impl IntoResponse {
    push_auth(&captured, &headers);
    (StatusCode::INTERNAL_SERVER_ERROR, "upstream boom")
}

async fn chat(client: &reqwest::Client, addr: std::net::SocketAddr) -> (StatusCode, String) {
    let response = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(
            json!({
                "model": "mock-model",
                "messages": [{ "role": "user", "content": "你好" }],
                "stream": false
            })
            .to_string(),
        )
        .send()
        .await
        .expect("网关必须响应");
    let status = response.status();
    (status, response.text().await.unwrap_or_default())
}

fn usage_of(state: &ApiKeyPoolsFile, key: &str) -> KeyUsage {
    state.pools[0]
        .keys
        .iter()
        .find(|entry| entry.key == key)
        .unwrap_or_else(|| panic!("池里应存在 {key}"))
        .usage
        .clone()
}

#[tokio::test]
async fn round_robin_rotates_skips_disabled_and_counts_usage() {
    let dir = temp_dir("rr");
    write_pool(
        &dir,
        pool(
            KeySelection::RoundRobin,
            vec![
                entry("a", "key-a", true, 0),
                entry("b", "key-b", true, 0),
                entry("c", "key-c", false, 0),
            ],
        ),
    );

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    for _ in 0..2 {
        let (status, body) = chat(&client, addr).await;
        assert!(status.is_success(), "网关应返回成功，实际 {status}: {body}");
    }

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 2, "上游应收到两次请求");
    assert!(auths[0].ends_with("key-a"), "第一次应用 key-a：{auths:?}");
    assert!(auths[1].ends_with("key-b"), "第二次应轮到 key-b：{auths:?}");
    assert!(
        auths.iter().all(|auth| !auth.contains("key-c")),
        "停用 key 不允许被选中：{auths:?}"
    );

    let snapshot = state.key_pools_snapshot().await;
    let a = usage_of(&snapshot, "key-a");
    let b = usage_of(&snapshot, "key-b");
    let c = usage_of(&snapshot, "key-c");
    assert_eq!(a.request_count, 1, "key-a 应计 1 次");
    assert_eq!(b.request_count, 1, "key-b 应计 1 次");
    assert_eq!(c.request_count, 0, "停用 key 不应计数");
    assert_eq!(a.total_tokens, 35, "usage 30+5 应累加到 key-a");
    assert_eq!(b.total_tokens, 35, "usage 30+5 应累加到 key-b");
    assert_eq!(c.total_tokens, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn least_used_prefers_the_least_requested_key() {
    let dir = temp_dir("lu");
    write_pool(
        &dir,
        pool(
            KeySelection::LeastUsed,
            vec![
                entry("a", "key-a", true, 5),
                entry("b", "key-b", true, 1),
                entry("c", "key-c", false, 0),
            ],
        ),
    );

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, _state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    for _ in 0..2 {
        let (status, body) = chat(&client, addr).await;
        assert!(status.is_success(), "网关应返回成功，实际 {status}: {body}");
    }

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 2);
    assert!(
        auths.iter().all(|auth| auth.ends_with("key-b")),
        "最少使用策略应一直选请求数最少的 key-b：{auths:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn upstream_failure_records_last_error_on_the_used_key() {
    let dir = temp_dir("err");
    write_pool(
        &dir,
        pool(
            KeySelection::RoundRobin,
            vec![entry("a", "key-a", true, 0)],
        ),
    );

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(failing_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    let _ = chat(&client, addr).await;

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(
        auths.iter().any(|auth| auth.ends_with("key-a")),
        "失败请求也必须真的用过池 key：{auths:?}"
    );

    let snapshot = state.key_pools_snapshot().await;
    let a = usage_of(&snapshot, "key-a");
    assert_eq!(a.request_count, 1, "失败尝试也应计一次请求");
    let last_error = a.last_error.unwrap_or_default();
    assert!(
        last_error.contains("500"),
        "last_error 应记录上游 5xx，实际：{last_error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
