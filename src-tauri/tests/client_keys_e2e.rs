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

/// 默认安装：池里只有一把 primary 本地占位密钥 `sugt-local-key`。
fn write_default_local_key(dir: &PathBuf) {
    save_keys(
        dir,
        &ClientKeysFile {
            version: 1,
            keys: vec![entry("local", "默认", "sugt-local-key", true)],
        },
    )
    .expect("默认入站密钥文件可写入");
}

/// 池里含一把启用主密钥与一把停用密钥，用于验证停用链路。
fn write_disabled_key(dir: &PathBuf) {
    let mut disabled = entry("disabled", "已停用", "sk-disabled", false);
    disabled.enabled = false;
    save_keys(
        dir,
        &ClientKeysFile {
            version: 1,
            keys: vec![entry("active", "在线", "sk-active", true), disabled],
        },
    )
    .expect("含停用键的入站密钥文件可写入");
}

/// 不返回 usage 的上游：正文刻意足够长，旧实现会把字符数估算成 token 冒充实测。
async fn no_usage_upstream(
    State(captured): State<Captured>,
    headers: HeaderMap,
) -> impl IntoResponse {
    push_auth(&captured, &headers);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "id": "chatcmpl-no-usage",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "这是一段刻意足够长的模拟回复正文，用来确认上游没有 usage 字段时网关不会把字符数估算值冒充真实 token 用量写入入站密钥统计。"
                }
            }]
        })
        .to_string(),
    )
}

/// 默认安装的本地主密钥 `sugt-local-key` 必须正常计入用量。
///
/// 覆盖「接入密钥用量永远 0 次 0」缺陷：旧实现在 inbound_target 里无条件跳过
/// DUMMY key（`sugt-local-key`），导致默认主密钥的请求/token 全部丢失。
#[tokio::test]
async fn default_local_key_usage_is_recorded() {
    let dir = temp_dir("default-local");
    write_default_local_key(&dir);

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    // 请求前：用量全为 0 —— 正是缺陷的起点。
    let before = usage_of(&state, "local").await;
    assert_eq!(
        before.request_count, 0,
        "初始 request_count 期望 0，实际 {}",
        before.request_count
    );
    assert_eq!(
        before.total_tokens, 0,
        "初始 total_tokens 期望 0，实际 {}",
        before.total_tokens
    );
    assert!(
        before.clients.is_empty(),
        "初始 clients 期望为空，实际 {:?}",
        before.clients
    );

    let client = reqwest::Client::new();
    let (status, body) = chat(&client, addr, "sugt-local-key", "default-local").await;
    assert!(
        status.is_success(),
        "默认本地密钥请求期望成功，实际 {status}: {body}"
    );

    let usage = wait_for_tokens(&state, "local", 35).await;
    assert_eq!(
        usage.request_count, 1,
        "默认主密钥 request_count 期望从 0 变 1，实际 {}",
        usage.request_count
    );
    assert_eq!(
        usage.total_tokens, 35,
        "上游真实 usage 30+5 期望累加为 35，实际 {}",
        usage.total_tokens
    );
    assert_eq!(
        usage.clients.len(),
        1,
        "clients 期望 1 条，实际 {} 条",
        usage.clients.len()
    );
    assert_eq!(
        usage.clients[0].label, "default-local",
        "客户端标签期望 default-local，实际 {}",
        usage.clients[0].label
    );
    assert_eq!(
        usage.clients[0].total_tokens, 35,
        "客户端 total_tokens 期望 35，实际 {}",
        usage.clients[0].total_tokens
    );

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 1, "期望 1 次请求打到上游，实际 {}", auths.len());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 上游响应不带 usage 时，不得把字符数估算值当作实测 token 写入。
///
/// 覆盖「估算冒充实测」缺陷：total_tokens 必须保持 0，但 request_count 照常累加。
#[tokio::test]
async fn missing_upstream_usage_does_not_inflate_tokens() {
    let dir = temp_dir("no-usage");
    write_keys(&dir);

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(no_usage_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    let (status, body) = chat(&client, addr, "sk-team", "no-usage-client").await;
    assert!(
        status.is_success(),
        "无 usage 的上游响应仍应成功透传，实际 {status}: {body}"
    );

    // 请求计数是同步落内存的，响应返回即应为 1。
    let usage = usage_of(&state, "team").await;
    assert_eq!(
        usage.request_count, 1,
        "request_count 期望 1，实际 {}",
        usage.request_count
    );
    assert_eq!(
        usage.clients.len(),
        1,
        "clients 期望 1 条，实际 {} 条",
        usage.clients.len()
    );

    // token 归集走 tokio::spawn：持续观察约 1s，确认没有任何估算值被写进来。
    for _ in 0..20 {
        let usage = usage_of(&state, "team").await;
        assert_eq!(
            usage.total_tokens, 0,
            "上游无 usage 时 total_tokens 期望恒为 0（不得把字符数估算当实测），实际 {}",
            usage.total_tokens
        );
        assert_eq!(
            usage.clients[0].total_tokens, 0,
            "客户端 total_tokens 期望恒为 0，实际 {}",
            usage.clients[0].total_tokens
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 1, "期望 1 次请求打到上游，实际 {}", auths.len());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 停用的密钥必须被拒绝，且不产生任何用量。
///
/// 覆盖增删停用链路里「停用后仍能刷用量」的风险。
#[tokio::test]
async fn disabled_key_is_rejected_without_usage() {
    let dir = temp_dir("disabled");
    write_disabled_key(&dir);

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, state) = start_gateway(&dir, upstream).await;

    let client = reqwest::Client::new();
    let (status, _body) = chat(&client, addr, "sk-disabled", "disabled-client").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "停用 key 期望返回 401，实际 {status}"
    );

    let disabled = usage_of(&state, "disabled").await;
    assert_eq!(
        disabled.request_count, 0,
        "停用 key request_count 期望 0，实际 {}",
        disabled.request_count
    );
    assert_eq!(
        disabled.total_tokens, 0,
        "停用 key total_tokens 期望 0，实际 {}",
        disabled.total_tokens
    );
    assert!(
        disabled.clients.is_empty(),
        "停用 key clients 期望为空，实际 {:?}",
        disabled.clients
    );

    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(
        auths.is_empty(),
        "停用 key 不应触达上游，实际 {} 次",
        auths.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 与 `start_gateway` 相同，但额外返回 mock 上游地址——WB-003 需要用它构造
/// `AppRuntime` 的配置，才能在真实 HTTP 链路上验证程序化改 key。
async fn start_gateway_with_upstream(
    dir: &PathBuf,
    upstream: Router,
) -> (std::net::SocketAddr, std::net::SocketAddr, GatewayState) {
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
    (addr, upstream_addr, state)
}

/// WB-003：程序化改主入站 key 必须同步 client-keys 池内 primary。
///
/// 旧实现只改 AppConfig，池里仍是旧 key，导致接入卡显示新 key、客户端带新 key 却 401。
/// 这里直接调用 `update_gateway_settings_core`（与接入卡/Codex 脚本同一条程序化路径），
/// 验证：池内 primary.key 与落盘内容同步更新，新 key 立刻可用且用量记到它，旧 key 立刻 401。
#[tokio::test]
async fn programmatic_primary_key_change_syncs_key_pool() {
    let dir = temp_dir("programmatic-key");
    write_keys(&dir); // primary = sk-config

    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let upstream = Router::new()
        .route("/*path", any(ok_upstream))
        .with_state(captured.clone());
    let (addr, upstream_addr, state) = start_gateway_with_upstream(&dir, upstream).await;

    let mut runtime_config = app_config(upstream_addr);
    runtime_config.codex_takeover_enabled = false; // 测试不得改写真实 ~/.codex
    let runtime = sugt_lib::commands::AppRuntime {
        paths: sugt_lib::config::AppPaths {
            config_dir: dir.clone(),
            config_file: dir.join("config.toml"),
            log_file: dir.join("sugt.log"),
        },
        gateway: state.clone(),
        config: Arc::new(tokio::sync::RwLock::new(runtime_config)),
    };

    // 用 serde 构造输入：GatewaySettingsInput 无 Default，未给字段的 Option 均为 None。
    let input: sugt_lib::commands::GatewaySettingsInput = serde_json::from_value(json!({
        "gatewayClientApiKey": "sk-new"
    }))
    .expect("输入可反序列化");
    let updated = sugt_lib::commands::update_gateway_settings_core(&runtime, input, None)
        .await
        .expect("程序化改 key 应成功");
    assert_eq!(updated.gateway_client_api_key, "sk-new");

    // 池内 primary 必须同步成新 key，且落盘一致。
    let file = state.client_keys_snapshot().await;
    let primary = file.keys.iter().find(|k| k.primary).expect("primary");
    assert_eq!(primary.key, "sk-new", "程序化改 key 必须同步密钥池 primary");
    let on_disk = sugt_lib::client_keys::load(&dir);
    let on_disk_primary = on_disk.keys.iter().find(|k| k.primary).expect("落盘 primary");
    assert_eq!(on_disk_primary.key, "sk-new", "落盘 client-keys.json 必须同步");
    let config_text = std::fs::read_to_string(dir.join("config.toml")).expect("config.toml 可读");
    assert!(
        config_text.contains("sk-new"),
        "config.toml 必须写入新 key"
    );

    let client = reqwest::Client::new();
    // 新 key：200，且 usage 30+5 记到同步后的 primary。
    let (status_new, body_new) = chat(&client, addr, "sk-new", "new-key-client").await;
    assert!(
        status_new.is_success(),
        "新 key 应成功，实际 {status_new}: {body_new}"
    );
    let new_usage = wait_for_tokens(&state, "config", 35).await;
    assert_eq!(
        new_usage.total_tokens, 35,
        "新 key 的 usage 应记到同步后的 primary"
    );
    assert!(
        new_usage.clients.iter().any(|c| c.label == "new-key-client"),
        "用量应归因到新 key 的客户端"
    );

    // 旧 key：401，且不产生任何用量或上游请求。
    let (status_old, _) = chat(&client, addr, "sk-config", "old-key-client").await;
    assert_eq!(status_old, StatusCode::UNAUTHORIZED, "旧 key 必须立即 401");
    let after = usage_of(&state, "config").await;
    assert_eq!(after.request_count, 1, "旧 key 请求不得计数");
    assert_eq!(after.total_tokens, 35, "旧 key 请求不得产生 usage");
    assert!(
        !after.clients.iter().any(|c| c.label == "old-key-client"),
        "旧 key 请求不得产生客户端用量记录"
    );
    let auths = captured.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(auths.len(), 1, "只有新 key 请求打到上游");

    let _ = std::fs::remove_dir_all(&dir);
}
