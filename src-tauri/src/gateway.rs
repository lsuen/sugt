use crate::{
    anthropic_adapter, config, error_hint, gateway_stats::{GatewayStatsCollector, RequestRecord},
    model::{
        AnthropicAccessMode, AppConfig, ProviderConfig, ProviderProtocol, ProviderStatus, ProxyHit,
    },
    model_registry::ModelEntry,
    provider_catalog,
    request_rules::{self, RequestRule, RuleContext, RuleEffects},
    strategy_box::{self, StrategyBox, StrategyRuntime, StrategyState},
    workspace_rules::{self, RequestShape},
};
use anyhow::{anyhow, bail, Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
    Json, Router,
};
use bytes::Bytes;
use async_stream::stream;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{oneshot, RwLock},
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};

/// 控制台「测试」的返回：既有状态枚举（前端仍按字符串比较），也有延迟/速度报告。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestOutcome {
    pub status: ProviderStatus,
    pub report: crate::probe::ProbeReport,
}

#[derive(Clone)]
pub struct GatewayState {
    inner: Arc<RwLock<GatewayInner>>,
    client: Client,
    last_hit: Arc<RwLock<Option<ProxyHit>>>,
    stats: GatewayStatsCollector,
    /// 真实在途 LLM 请求数；只有代理入口会计数，桌面精灵据此播放 typing 动作
    active_requests: Arc<AtomicU64>,
    /// 策略盒子配置 + 进程内健康度（PRD v1.1.11 R3）
    strategy: Arc<RwLock<StrategyState>>,
    /// 模型注册表快照（PRD v1.1.11 R2）：盒子成员里被禁用（enabled = false）的模型不参与编排
    registry: Arc<RwLock<Vec<ModelEntry>>>,
    /// 请求改写规则（PRD v1.1.11 R6）：按作用域匹配后改写发往上游的请求
    rules: Arc<RwLock<Vec<RequestRule>>>,
}

/// RAII 计数：请求进入 +1，无论成功、失败还是提前返回都在析构时 -1，避免计数泄漏。
struct ActiveRequestGuard(Arc<AtomicU64>);

impl ActiveRequestGuard {
    fn new(counter: Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 把在途计数守卫挂到响应体生命周期。
///
/// handler 返回时 hyper 才刚开始发送 body，SSE 流可能还要跑几十秒；若守卫随 handler
/// 栈帧析构，`active_requests` 会在响应头到达时就归零，桌面精灵会提前停止 typing。
/// 这里用一层只做转发的 stream 持有守卫：body 读完（或客户端断开导致 body 被丢弃）
/// 时守卫才析构。不缓冲任何字节，不改变流式语义。
fn hold_guard_through_body(
    response: Response<Body>,
    guard: ActiveRequestGuard,
) -> Response<Body> {
    let (parts, body) = response.into_parts();
    let stream = body
        .into_data_stream()
        .map(|item| item.map_err(std::io::Error::other));
    let held = async_stream::stream! {
        // 守卫被移入生成器状态：即使一次都未 poll，也已经不再属于 handler 栈帧
        let _held = guard;
        let mut stream = std::pin::pin!(stream);
        while let Some(chunk) = stream.next().await {
            yield chunk;
        }
    };
    Response::from_parts(parts, Body::from_stream(held))
}

/// 熔断器冷却时间
const CIRCUIT_BREAKER_COOLDOWN: Duration = Duration::from_secs(30);

struct GatewayInner {
    config: AppConfig,
    config_file: Option<PathBuf>,
    config_mtime: Option<std::time::SystemTime>,
    shutdown: Option<oneshot::Sender<()>>,
    running: bool,
    /// 熔断器：provider_id -> 故障发生时间
    circuit_breaker: HashMap<String, Instant>,
    /// 原生 Anthropic 端点不可用的 provider_id 缓存（避免每次请求都先白试一次）
    native_unsupported: std::collections::HashSet<String>,
}

fn maybe_reload_config(inner: &mut GatewayInner) {
    let path = match &inner.config_file {
        Some(p) => p.clone(),
        None => return,
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mtime = match meta.modified() {
        Ok(t) => t,
        Err(_) => return,
    };
    if inner.config_mtime == Some(mtime) {
        return;
    }
    if let Ok((cfg, _)) = config::load_config_file(&path) {
        inner.config = cfg;
        inner.config_mtime = Some(mtime);
        // 配置重载后，之前标记为「原生不支持」的 provider 可能已更换端点，清空缓存
        inner.native_unsupported.clear();
    }
}

impl GatewayState {
    pub fn new(config: AppConfig, config_file: Option<PathBuf>) -> Result<Self> {
        let config_mtime = config_file
            .as_ref()
            .and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        // 流式代理不能绑「整段请求总超时」：Codex 工具环经常远超 10 分钟，
        // 总超时会在半路掐断 SSE，客户端只看到截断。连接超时 + keepalive 即可。
        let client = build_upstream_client()?;

        // 启动时从 config_dir（config.toml 所在目录）加载已保存的策略盒子。
        let boxes = config_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(strategy_box::load_boxes)
            .unwrap_or_default();

        // 模型注册表同样在启动时载入内存；任何读取/解析失败都退化为空表，不 panic。
        let registry = config_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(|dir| crate::model_registry::load(dir).entries)
            .unwrap_or_default();

        // 请求改写规则（R6）：与策略盒子同一 config 目录；文件缺失/损坏时 load_rules 已回退内置。
        let rules = config_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(request_rules::load_rules)
            .unwrap_or_else(request_rules::builtin_rules);

        Ok(Self {
            inner: Arc::new(RwLock::new(GatewayInner {
                config,
                config_file,
                config_mtime,
                shutdown: None,
                running: false,
                circuit_breaker: HashMap::new(),
                native_unsupported: std::collections::HashSet::new(),
            })),
            client,
            last_hit: Arc::new(RwLock::new(None)),
            stats: GatewayStatsCollector::new(),
            active_requests: Arc::new(AtomicU64::new(0)),
            strategy: Arc::new(RwLock::new(StrategyState {
                boxes,
                runtime: StrategyRuntime::default(),
            })),
            registry: Arc::new(RwLock::new(registry)),
            rules: Arc::new(RwLock::new(rules)),
        })
    }

    /// 请求改写规则快照（R6）。
    pub async fn request_rules_snapshot(&self) -> Vec<RequestRule> {
        self.rules.read().await.clone()
    }

    /// 规则写盘后同步内存副本，无需重启网关即可生效。
    pub async fn set_request_rules(&self, rules: Vec<RequestRule>) {
        *self.rules.write().await = rules;
    }

    /// 当前策略盒子快照（不含健康度）。
    pub async fn strategy_boxes(&self) -> Vec<StrategyBox> {
        self.strategy.read().await.boxes.clone()
    }

    /// 用新的盒子配置替换内存副本；健康度与轮换游标保留，不清空。
    pub async fn set_strategy_boxes(&self, boxes: Vec<StrategyBox>) {
        self.strategy.write().await.boxes = boxes;
    }

    /// 供选路阶段读取/写入运行态（健康度、轮换游标）。
    pub fn strategy_state(&self) -> Arc<RwLock<StrategyState>> {
        self.strategy.clone()
    }

    /// 模型注册表快照：选路阶段据此跳过被禁用的成员模型。
    pub async fn model_registry_snapshot(&self) -> Vec<ModelEntry> {
        self.registry.read().await.clone()
    }

    /// 注册表写盘后同步内存副本（禁用状态立即生效，无需等重启）。
    pub async fn set_model_registry_snapshot(&self, entries: Vec<ModelEntry>) {
        *self.registry.write().await = entries;
    }

    /// 盒子成员的一次 attempt 判成功：延迟覆盖、失败计数清零。
    async fn note_member_success(&self, box_id: &str, provider: &ProviderConfig, latency_ms: u64) {
        let key = member_key_of(box_id, provider);
        self.strategy
            .write()
            .await
            .runtime
            .record_success(&key, latency_ms);
    }

    /// 盒子成员的一次 attempt 判失败：失败计数 +1，配置了冷却则进入冷却期。
    async fn note_member_failure(
        &self,
        box_id: &str,
        provider: &ProviderConfig,
        error: &str,
        cooldown_ms: Option<u64>,
    ) {
        let key = member_key_of(box_id, provider);
        self.strategy
            .write()
            .await
            .runtime
            .record_failure(&key, error, cooldown_ms);
    }

    pub fn http_client(&self) -> &Client {
        &self.client
    }

    /// 检查 provider 是否处于熔断冷却期
    async fn is_circuit_open(&self, provider_id: &str) -> bool {
        let inner = self.inner.read().await;
        inner
            .circuit_breaker
            .get(provider_id)
            .map(|tripped_at| tripped_at.elapsed() < CIRCUIT_BREAKER_COOLDOWN)
            .unwrap_or(false)
    }

    /// 将 provider 纳入熔断器
    async fn trip_circuit(&self, provider_id: &str) {
        let mut inner = self.inner.write().await;
        inner
            .circuit_breaker
            .insert(provider_id.to_string(), Instant::now());
    }

    /// 请求成功时清除熔断状态
    async fn reset_circuit(&self, provider_id: &str) {
        let mut inner = self.inner.write().await;
        inner.circuit_breaker.remove(provider_id);
    }

    /// 该 provider 的原生 Anthropic 端点是否已被标记为不可用
    async fn is_native_unsupported(&self, provider_id: &str) -> bool {
        let inner = self.inner.read().await;
        inner.native_unsupported.contains(provider_id)
    }

    /// 标记 provider 的原生 Anthropic 端点不可用（后续请求直接走转换 fallback）
    async fn mark_native_unsupported(&self, provider_id: &str) {
        let mut inner = self.inner.write().await;
        inner.native_unsupported.insert(provider_id.to_string());
    }

    /// 配置变更（新增/编辑/删除 provider）后清除原生端点缓存
    pub async fn clear_native_unsupported_cache(&self) {
        let mut inner = self.inner.write().await;
        inner.native_unsupported.clear();
    }

    pub async fn list_provider_models(
        &self,
        base_url: &str,
        api_key: &str,
        vendor_id: Option<&str>,
    ) -> Result<Vec<String>> {
        if let Some(id) = vendor_id {
            if let Some(entry) = provider_catalog::vendor_by_id(id) {
                match provider_catalog::models_list_base_for_vendor(entry) {
                    Some(list_base) => {
                        return list_openai_models(&self.client, list_base, api_key).await;
                    }
                    None => {
                        bail!("该服务商需手动填写 Model Name，暂无公开模型列表接口");
                    }
                }
            }
        }
        list_openai_models(&self.client, base_url, api_key).await
    }

    pub async fn traffic_stats(&self) -> crate::gateway_stats::TrafficStatsView {
        let mut view = self.stats.snapshot().await;
        // 实时覆盖：/v1/_sugt/traffic 也走这里，detached 网关进程同样能给出真实在途数
        view.active_requests = self.active_requests.load(Ordering::Relaxed);
        view
    }

    /// 标记一个真实 LLM 请求进入；健康检查、看门狗心跳、免费通道探测不调用。
    fn begin_request(&self) -> ActiveRequestGuard {
        ActiveRequestGuard::new(self.active_requests.clone())
    }

    async fn record_request_stats(
        &self,
        success: bool,
        latency_ms: u64,
        headers: &HeaderMap,
        provider: &ProviderConfig,
        path: &str,
    ) {
        let user_agent = headers
            .get(http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        self.stats.record(RequestRecord {
            success,
            latency_ms,
            user_agent,
            provider_name: provider.name.clone(),
            model_name: provider.model_name.clone(),
            path: path.to_string(),
        })
        .await;
    }

    async fn record_failure_stats(
        &self,
        latency_ms: u64,
        headers: &HeaderMap,
        path: &str,
        provider: Option<&ProviderConfig>,
    ) {
        let user_agent = headers
            .get(http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let (provider_name, model_name) = provider
            .map(|p| (p.name.clone(), p.model_name.clone()))
            .unwrap_or_else(|| ("网关".to_string(), "-".to_string()));
        self.stats.record(RequestRecord {
            success: false,
            latency_ms,
            user_agent,
            provider_name,
            model_name,
            path: path.to_string(),
        })
        .await;
    }

    pub async fn last_proxy_hit(&self) -> Option<ProxyHit> {
        self.last_hit.read().await.clone()
    }

    async fn record_proxy_hit(
        &self,
        provider: &ProviderConfig,
        path: &str,
        provider_index: usize,
        latency_ms: u64,
        headers: &HeaderMap,
        mode: &str,
        success: bool,
        workspace_rules_injected: bool,
    ) {
        let client = crate::gateway_stats::client_label_from_headers(headers);
        let hit = ProxyHit {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            path: path.to_string(),
            client: client.clone(),
            mode: mode.to_string(),
            failover: provider_index > 0,
            workspace_rules_injected,
            at: chrono::Utc::now(),
        };
        if success {
            info!(
                client = %client,
                provider = %provider.name,
                model = %provider.model_name,
                path = %path,
                failover = provider_index > 0,
                latency_ms,
                "inbound request ok"
            );
        } else {
            warn!(
                client = %client,
                provider = %provider.name,
                model = %provider.model_name,
                path = %path,
                latency_ms,
                "inbound request failed (non-2xx upstream)"
            );
        }
        *self.last_hit.write().await = Some(hit);
        self.record_request_stats(success, latency_ms, headers, provider, path)
            .await;
    }

    async fn record_inbound_denied(
        &self,
        headers: &HeaderMap,
        path: &str,
        reason: &str,
    ) {
        let client = crate::gateway_stats::client_label_from_headers(headers);
        warn!(client = %client, path = %path, reason = %reason, "inbound request denied");
        self.record_failure_stats(0, headers, path, None).await;
    }

    pub async fn ensure_listening(&self, auto_start: bool) -> Result<(), String> {
        if self.is_running().await {
            return Ok(());
        }
        if auto_start {
            self.start()
                .await
                .map_err(|err| error_hint::format_gateway_start_error(&err))?;
            Ok(())
        } else {
            Err(
                "gateway_not_running:本地网关未运行，请先启动网关或使用「启动网关并接管」".to_string(),
            )
        }
    }

    pub async fn config(&self) -> AppConfig {
        let mut inner = self.inner.write().await;
        maybe_reload_config(&mut inner);
        inner.config.clone()
    }

    pub async fn replace_config(&self, config: AppConfig) {
        let mut inner = self.inner.write().await;
        inner.config = config;
        // 配置变更（新增/编辑/删除 provider）后清除原生端点缓存
        inner.native_unsupported.clear();
        if let Some(path) = &inner.config_file {
            inner.config_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        }
    }

    pub async fn is_running(&self) -> bool {
        self.inner.read().await.running
    }

    pub async fn start(&self) -> Result<String> {
        {
            let inner = self.inner.read().await;
            if inner.running {
                return Ok(inner.config.listen_url());
            }
        }

        let config_snapshot = self.config().await;
        let (listener, bound_addr) =
        crate::gateway_listen::bind_with_fallback(&config_snapshot).await?;
        let bound_port = bound_addr.port();

        let (tx, rx) = oneshot::channel();
        let app = build_router(self.clone());

        {
            let mut inner = self.inner.write().await;
            if bound_port != inner.config.port {
                inner.config.port = bound_port;
            }
            inner.shutdown = Some(tx);
            inner.running = true;
        }

        let state = self.clone();
        tokio::spawn(async move {
            info!(%bound_addr, "SUGT gateway started");
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;

            if let Err(err) = result {
                error!(error = %err, "gateway server error");
            }

            state.inner.write().await.running = false;
            info!("SUGT gateway stopped");
        });

        let url = self.config().await.listen_url();
        Ok(url)
    }

    pub async fn stop(&self) -> Result<()> {
        let shutdown = self.inner.write().await.shutdown.take();
        if let Some(tx) = shutdown {
            let _ = tx.send(());
        }
        self.inner.write().await.running = false;
        Ok(())
    }

    pub async fn active_provider(&self) -> Option<ProviderConfig> {
        let config = self.config().await;
        select_provider(&config).cloned()
    }

    pub async fn test_provider(&self, provider_id: &str) -> Result<ProviderTestOutcome> {
        let provider = {
            let inner = self.inner.read().await;
            inner
                .config
                .providers
                .iter()
                .find(|provider| provider.id == provider_id)
                .cloned()
        }
        .ok_or_else(|| anyhow!("模型配置不存在"))?;

        let report = crate::probe::probe_llm_connectivity(&self.client, &provider).await;
        let error_message = report.error.clone();
        let status = if report.ok {
            ProviderStatus::Available
        } else {
            warn!(
                provider = %provider.name,
                error = error_message.as_deref().unwrap_or_default(),
                "provider test failed"
            );
            ProviderStatus::Unavailable
        };

        let mut inner = self.inner.write().await;
        if let Some(item) = inner
            .config
            .providers
            .iter_mut()
            .find(|item| item.id == provider_id)
        {
            item.status = status.clone();
            item.last_checked_at = Some(chrono::Utc::now());
            item.last_error = error_message;
        }

        Ok(ProviderTestOutcome { status, report })
    }
}

/// 构造网关路由。公开给集成测试，用真实 HTTP 请求验证 /v1/responses 全链路。
pub fn build_router(state: GatewayState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/_sugt/traffic", get(traffic_endpoint))
        .route("/v1/_sugt/workspace-rules", get(workspace_rules_preview))
        .route("/v1/_sugt/notify", post(notify_endpoint))
        .route("/v1/messages", post(proxy_anthropic_messages))
        .route("/*path", any(proxy_openai))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<GatewayState>) -> impl IntoResponse {
    let provider = state.active_provider().await;
    Json(json!({
        "ok": provider.is_some(),
        "service": "SUGT",
        "active_model": provider.map(|item| item.model_name),
    }))
}

async fn traffic_endpoint(State(state): State<GatewayState>) -> impl IntoResponse {
    Json(state.traffic_stats().await)
}

/// 预览当前会注入上游的全局规则文本（便于排查「勾了却没感觉」）。
async fn workspace_rules_preview(State(state): State<GatewayState>) -> impl IntoResponse {
    let config = state.config().await;
    let rules = &config.workspace_rules;
    let inject_text = workspace_rules::build_inject_text(rules, &config.listen_url());
    Json(json!({
        "enabled": rules.enabled,
        "doc_habits": rules.doc_habits,
        "notify_enabled": rules.notify_enabled,
        "has_inject": inject_text.is_some(),
        "inject_text": inject_text,
    }))
}

/// 本地代发 webhook：密钥只留配置里。无 Key 时与本地网关其它接口一致放行；错误 Key 拒绝。
async fn notify_endpoint(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response<Body> {
    let config = state.config().await;
    if crate::gateway_auth::extract_client_api_key(&headers).is_some()
        && !crate::gateway_auth::validate_client_request(&headers, &config)
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "invalid gateway api key" })),
        )
            .into_response();
    }
    let rules = &config.workspace_rules;
    if !rules.notify_enabled {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "ok": false, "error": "webhook notify is disabled" })),
        )
            .into_response();
    }
    let webhook = rules.notify_webhook.trim();
    if webhook.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "ok": false, "error": "webhook is empty" })),
        )
            .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("SUGT");
    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or(title);
    match send_webhook_markdown(state.http_client(), webhook, title, text).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "ok": false, "error": err.to_string() })),
        )
            .into_response(),
    }
}

pub async fn send_webhook_markdown(
    client: &Client,
    webhook: &str,
    title: &str,
    text: &str,
) -> Result<()> {
    let payload = workspace_rules::webhook_markdown_payload(title, text);
    let response = client
        .post(webhook)
        .json(&payload)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("webhook 请求失败")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("webhook 返回 HTTP {}: {}", status.as_u16(), body);
    }
    if let Ok(v) = serde_json::from_str::<Value>(&body) {
        if let Some(errcode) = v.get("errcode").and_then(|e| e.as_i64()) {
            if errcode != 0 {
                let errmsg = v
                    .get("errmsg")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown");
                bail!("webhook errcode={errcode}: {errmsg}");
            }
        }
    }
    Ok(())
}

fn skip_workspace_rules(headers: &HeaderMap) -> bool {
    let flag = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v == "1"
                    || v.eq_ignore_ascii_case("true")
                    || v.eq_ignore_ascii_case("me")
                    || v.eq_ignore_ascii_case("soul")
            })
            .unwrap_or(false)
    };
    flag("x-sugt-skip-workspace-rules") || flag("x-sugt-client")
}

/// 返回 (body, 是否注入了全局规则)。
fn apply_workspace_rules(
    body: Bytes,
    config: &AppConfig,
    shape: RequestShape,
    headers: &HeaderMap,
) -> (Bytes, bool) {
    if skip_workspace_rules(headers) {
        return (body, false);
    }
    let injected = workspace_rules::build_inject_text(
        &config.workspace_rules,
        &config.listen_url(),
    )
    .is_some();
    let body = workspace_rules::inject_into_request_body(
        body,
        &config.workspace_rules,
        &config.listen_url(),
        shape,
    );
    (body, injected)
}

async fn models(State(state): State<GatewayState>) -> impl IntoResponse {
    let config = state.config().await;
    let mut seen = std::collections::HashSet::new();
    let mut data: Vec<Value> = config
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .filter_map(|provider| {
            let id = provider.public_model_id();
            if !seen.insert(id.clone()) {
                return None;
            }
            Some(json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": provider.provider,
            }))
        })
        .collect();

    // 启用中的策略盒子同样对外暴露一个统一 model id，客户端才能发现并选中它。
    for box_spec in state.strategy_boxes().await.iter().filter(|box_spec| box_spec.enabled) {
        let id = box_spec.public_model_id.trim().to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        data.push(json!({
            "id": id,
            "object": "model",
            "created": 0,
            "owned_by": "strategy",
        }));
    }

    Json(json!({ "object": "list", "data": data }))
}

async fn proxy_openai(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response<Body> {
    let config = state.config().await;
    let path = request.uri().path().trim_start_matches('/').to_string();
    if !crate::gateway_auth::validate_client_request(&headers, &config) {
        state
            .record_inbound_denied(&headers, &path, "invalid gateway api key")
            .await;
        return json_error(StatusCode::UNAUTHORIZED, "网关 API Key 无效或未提供");
    }
    let client = crate::gateway_stats::client_label_from_headers(&headers);
    info!(client = %client, path = %path, "inbound openai-compatible request");
    let active_guard = state.begin_request();
    let started = std::time::Instant::now();
    match proxy_request(state.clone(), headers.clone(), request).await {
        Ok(response) => hold_guard_through_body(response, active_guard),
        Err(err) => {
            error!(client = %client, path = %path, error = %err, "inbound request failed");
            state
                .record_failure_stats(
                    started.elapsed().as_millis() as u64,
                    &headers,
                    &path,
                    None,
                )
                .await;
            json_error(StatusCode::BAD_GATEWAY, &err.to_string())
        }
    }
}

async fn proxy_anthropic_messages(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Response<Body> {
    let config = state.config().await;
    if !crate::gateway_auth::validate_client_request(&headers, &config) {
        state
            .record_inbound_denied(&headers, "v1/messages", "invalid gateway api key")
            .await;
        return anthropic_error(StatusCode::UNAUTHORIZED, "网关 API Key 无效或未提供");
    }
    let client = crate::gateway_stats::client_label_from_headers(&headers);
    info!(client = %client, path = "v1/messages", "inbound anthropic messages request");
    let active_guard = state.begin_request();
    let started = std::time::Instant::now();
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!(client = %client, error = %err, "读取 Anthropic 请求体失败");
            return anthropic_error(StatusCode::BAD_REQUEST, "请求体读取失败");
        }
    };
    let stream = crate::anthropic_adapter::request_wants_stream(&bytes);
    let client_model = client_model_of(&bytes);
    let request = Request::from_parts(parts, Body::from(bytes));
    let run = proxy_anthropic_request(state.clone(), headers.clone(), request);
    let response = if stream {
        let client_model = client_model.clone();
        hold_stream(async move {
            match run.await {
                Ok(response) => response,
                Err(err) => {
                    warn!(error = %err, "anthropic 上游失败，改为会话内提示");
                    session_notice(
                        crate::upstream_resilience::Wire::Anthropic,
                        &client_model,
                        crate::upstream_resilience::NoticeKind::UpstreamDown,
                        true,
                    )
                }
            }
        })
    } else {
        let client_model = client_model.clone();
        let state = state.clone();
        let headers = headers.clone();
        hold_json(async move {
            match run.await {
                Ok(response) => response,
                Err(err) => {
                    error!(client = %client, path = "v1/messages", error = %err, "inbound request failed");
                    state
                        .record_failure_stats(
                            started.elapsed().as_millis() as u64,
                            &headers,
                            "v1/messages",
                            None,
                        )
                        .await;
                    session_notice(
                        crate::upstream_resilience::Wire::Anthropic,
                        &client_model,
                        crate::upstream_resilience::NoticeKind::UpstreamDown,
                        false,
                    )
                }
            }
        })
    };
    hold_guard_through_body(response, active_guard)
}

/// 解析 provider 的原生 Anthropic 端点（base url）。
///
/// - Anthropic 协议：直接用自身 base_url
/// - OpenAI 协议：查服务商目录中的 `anthropic_base_url`（deepseek / zhipu / dashscope 等）
/// - 目录未收录：返回 None（不瞎猜，避免误伤）
fn anthropic_native_target(provider: &ProviderConfig) -> Option<String> {
    match provider.protocol {
        ProviderProtocol::Anthropic => Some(provider.base_url.clone()),
        ProviderProtocol::OpenAi => {
            crate::provider_catalog::vendor_by_id(&provider.provider)
                .and_then(|vendor| vendor.anthropic_base_url)
                .map(str::to_string)
        }
    }
}

/// 判断原生 Anthropic 端点请求失败是否为「服务商不支持原生端点」。
///
/// - 404 / 405：端点不存在或方法不允许 → 明确不支持，应降级转换
/// - 其它 4xx（如 401 鉴权失败）：真实错误，应如实返回，避免把鉴权错误误判为不支持
fn is_native_endpoint_unsupported(response: &Response<Body>) -> bool {
    matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    )
}

async fn proxy_anthropic_request(
    state: GatewayState,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await?;
    let config = state.config().await;
    let (effects, _is_codex) = request_rule_effects(&state, &headers, false).await;
    let (body_bytes, rules_injected) =
        apply_workspace_rules(body_bytes, &config, RequestShape::AnthropicMessages, &headers);
    // 用户在控制台选择的接入点协议模式（auto / openai / anthropic）
    let access_mode = config.anthropic_access_mode;
    // 用请求体原始 model 判定策略盒子；命不中时与 ordered_providers 完全一致。
    let client_model = client_model_of(&body_bytes);
    let route = request_route(&state, &config, &client_model).await;
    let box_id = route.box_id;
    let cooldown_ms = route.cooldown_ms;
    let providers = route.providers;

    if providers.is_empty() {
        bail!("没有可用模型配置");
    }

    // 识别调用来源：Claude 系 TUI/GUI 优先走原生 Anthropic 端点（转换作为 fallback）
    let anthropic_client = crate::anthropic_client_detect::detect_anthropic_client(&headers);

    let stream = crate::anthropic_adapter::request_wants_stream(&body_bytes);
    let mut memory = crate::upstream_resilience::FailureMemory::new();
    let mut budget = retry_budget(&config);
    let started = std::time::Instant::now();
    for pass in 0..crate::upstream_resilience::MAX_PASSES {
        memory.begin_pass();
    for (index, provider) in providers.iter().enumerate() {
        let is_free = crate::experimental_free::is_experimental_free(provider);
        memory.set_free(is_free);
        let own_key = !provider.api_key.trim().is_empty();
        if !is_free && state.is_circuit_open(&provider.id).await {
            warn!(provider = %provider.name, "provider 处于熔断冷却期，跳过");
            continue;
        }
        let attempt_started = std::time::Instant::now();

        let native_target = anthropic_native_target(provider);

        // 依据接入点协议模式决定请求路径
        // result 附带实际走的协议模式（mode），供命中记录与状态标识展示
        let (result, mode) = match access_mode {
            // 强制 OpenAI 协议：始终走转换，不尝试原生端点
            AnthropicAccessMode::OpenAi => (
                proxy_anthropic_via_openai(&state, &headers, body_bytes.clone(), provider, &effects)
                    .await,
                "openai_adapter",
            ),
            // 强制 Anthropic 原生协议：不走转换；无原生端点时给出明确错误并尝试下一个
            AnthropicAccessMode::Anthropic => match native_target {
                Some(target) => (
                    proxy_anthropic_native(
                        &state,
                        &headers,
                        body_bytes.clone(),
                        provider,
                        Some(target),
                        &effects,
                    )
                    .await
                    .map_err(|_| crate::upstream_resilience::classify_network()),
                    "anthropic_native",
                ),
                None => {
                    warn!(provider = %provider.name, "force-native mode skipped provider");
                    let _ = memory.observe(crate::upstream_resilience::FailureClass::Terminal(
                        crate::upstream_resilience::NoticeKind::BadRequest,
                    ));
                    continue;
                }
            },
            // 自动模式：识别到 Claude 客户端且服务商有原生端点则优先原生，失败降级转换。
            // 例外：Moonshot/Kimi —— 官方文档未披露 Anthropic 兼容端点，社区实测存在
            // max_tokens 必传、tool_choice 不支持 "any"、不识别 betas 等多处不兼容，
            // Claude Code 走原生端点常表现为空流无回复。因此 OpenAI 协议的 Kimi 在
            // 自动模式下统一走 OpenAI 适配器（与 Try it now 同一已验证路径）；
            // 显式把协议切为 Anthropic 的用户仍可走原生端点。
            AnthropicAccessMode::Auto => {
                let kimi_skips_native = provider.provider.eq_ignore_ascii_case("moonshot")
                    && provider.protocol == ProviderProtocol::OpenAi;
                let prefers_native = !kimi_skips_native
                    && (provider.protocol == ProviderProtocol::Anthropic
                        || (anthropic_client.is_some()
                            && native_target.is_some()
                            && !state.is_native_unsupported(&provider.id).await));

                if prefers_native {
                    match proxy_anthropic_native(
                        &state,
                        &headers,
                        body_bytes.clone(),
                        provider,
                        native_target,
                        &effects,
                    )
                    .await
                    {
                        Ok(response) if is_native_endpoint_unsupported(&response) => {
                            warn!(
                                provider = %provider.name,
                                client = ?anthropic_client.map(|kind| kind.label()),
                                "anthropic native /v1/messages 不支持, fallback to openai adapter"
                            );
                            // 记录缓存：后续该 provider 直接走转换，不再白试原生
                            state.mark_native_unsupported(&provider.id).await;
                            (
                                proxy_anthropic_via_openai(
                                    &state,
                                    &headers,
                                    body_bytes.clone(),
                                    provider,
                                    &effects,
                                )
                                .await,
                                "openai_adapter",
                            )
                        }
                        other => (
                            other.map_err(|_| crate::upstream_resilience::classify_network()),
                            "anthropic_native",
                        ),
                    }
                } else {
                    (
                        proxy_anthropic_via_openai(
                            &state,
                            &headers,
                            body_bytes.clone(),
                            provider,
                            &effects,
                        )
                        .await,
                        "openai_adapter",
                    )
                }
            }
        };

        match result {
            Ok(response) if response.status().is_success() => {
                if let Some(box_id) = box_id.as_deref() {
                    state
                        .note_member_success(
                            box_id,
                            provider,
                            attempt_started.elapsed().as_millis() as u64,
                        )
                        .await;
                }
                state.reset_circuit(&route_key(provider)).await;
                state
                    .record_proxy_hit(
                        provider,
                        "v1/messages",
                        index,
                        started.elapsed().as_millis() as u64,
                        &headers,
                        mode,
                        true,
                        rules_injected,
                    )
                    .await;
                return Ok(response);
            }
            Ok(response) => {
                let status = response.status();
                let retry_after = response
                    .headers()
                    .get(http::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .unwrap_or_default();
                let text = String::from_utf8_lossy(&bytes);
                warn!(provider = %provider.name, %status, body = %text, "provider returned anthropic error");
                if let Some(box_id) = box_id.as_deref() {
                    state
                        .note_member_failure(
                            box_id,
                            provider,
                            &format!("HTTP {} {}", status.as_u16(), text),
                            cooldown_ms,
                        )
                        .await;
                }
                let class = crate::upstream_resilience::classify_http(
                    status.as_u16(),
                    &text,
                    retry_after.as_deref(),
                    own_key,
                    is_free,
                );
                if let Some(kind) = memory.observe(class) {
                    return Ok(session_notice(
                        crate::upstream_resilience::Wire::Anthropic,
                        &client_model,
                        kind,
                        stream,
                    ));
                }
            }
            Err(class) => {
                warn!(provider = %provider.name, "provider anthropic request error");
                if let Some(box_id) = box_id.as_deref() {
                    state
                        .note_member_failure(
                            box_id,
                            provider,
                            &format!("{class:?}"),
                            cooldown_ms,
                        )
                        .await;
                }
                if let Some(kind) = memory.observe(class) {
                    return Ok(session_notice(
                        crate::upstream_resilience::Wire::Anthropic,
                        &client_model,
                        kind,
                        stream,
                    ));
                }
            }
        }
    }
        if let Some(response) = finish_pass(
            &mut memory,
            &mut budget,
            pass,
            &client_model,
            crate::upstream_resilience::Wire::Anthropic,
            stream,
        )
        .await
        {
            return Ok(response);
        }
    }

    Ok(session_notice(
        crate::upstream_resilience::Wire::Anthropic,
        &client_model,
        memory.notice_kind(),
        stream,
    ))
}

async fn proxy_anthropic_native(
    state: &GatewayState,
    headers: &HeaderMap,
    body: Bytes,
    provider: &ProviderConfig,
    native_target: Option<String>,
    effects: &RuleEffects,
) -> Result<Response<Body>> {
    // 原生端点优先用解析结果（可能来自服务商目录的 anthropic_base_url），兜底用 provider.base_url
    let target = build_target_url(
        native_target.as_deref().unwrap_or(&provider.base_url),
        "v1/messages",
        "",
    );
    let client_model = client_model_of(&body);
    let default_max = rule_default_max_tokens(effects, false);
    let payload = apply_rule_body_fields(
        rewrite_model(body, &provider.model_name, default_max),
        effects,
    );
    let mut builder = state.client.post(&target);
    builder = copy_forward_headers(
        builder,
        headers,
        &provider.api_key,
        AuthMode::Anthropic,
        &effects.headers,
    )?;

    info!(
        provider = %provider.name,
        model = %provider.model_name,
        path = "v1/messages",
        mode = "anthropic_native",
        "proxying anthropic messages request"
    );

    let response = builder.body(payload).send().await?;
    into_axum_response(
        response,
        Some(state.stats.clone()),
        Some(token_attr_from(headers, provider)),
        &client_model,
    )
    .await
}

async fn proxy_anthropic_via_openai(
    state: &GatewayState,
    headers: &HeaderMap,
    body: Bytes,
    provider: &ProviderConfig,
    effects: &RuleEffects,
) -> Result<Response<Body>, crate::upstream_resilience::FailureClass> {
    let failover = state.config().await.failover_enabled;
    let attempts = upstream_attempts(provider, failover);
    let client_model = client_model_of(&body);
    let is_free = crate::experimental_free::is_experimental_free(provider);
    let mut worst: Option<crate::upstream_resilience::FailureClass> = None;
    let tools = crate::upstream_resilience::request_declares_tools(&body);
    let stream = crate::anthropic_adapter::request_wants_stream(&body);
    for (attempt_idx, attempt) in attempts.iter().enumerate() {
        let key = route_key(attempt);
        if state.is_circuit_open(&key).await {
            continue;
        }
        let target = build_target_url(&attempt.base_url, "v1/chat/completions", "");
        let default_max = rule_default_max_tokens(effects, false);
        let payload = match anthropic_adapter::anthropic_to_openai_with_default(
            body.clone(),
            &attempt.model_name,
            default_max,
        ) {
            Ok(payload) => apply_rule_body_fields(payload, effects),
            Err(_) => {
                return Err(crate::upstream_resilience::FailureClass::Terminal(
                    crate::upstream_resilience::NoticeKind::BadRequest,
                ))
            }
        };
        let own_key = !attempt.api_key.trim().is_empty();
        let mut text_tries = 0u32;
        let class = loop {
            let mut builder = state.client.post(&target);
            builder = match copy_forward_headers(
                builder,
                headers,
                &attempt.api_key,
                AuthMode::OpenAi,
                &effects.headers,
            ) {
                Ok(builder) => builder,
                Err(_) => return Err(crate::upstream_resilience::classify_network()),
            };
            info!(
                provider = %attempt.name,
                model = %attempt.model_name,
                path = "v1/messages",
                mode = "openai_adapter",
                "proxying anthropic messages request"
            );
            match builder.body(payload.clone()).send().await {
            Ok(response) if response.status().is_success() => {
                let client = crate::gateway_stats::client_label_from_headers(headers);
                if tools {
                    let held = match if stream {
                        hold_chat_sse(response).await
                    } else {
                        hold_chat_json(response).await
                    } {
                        Ok(held) => held,
                        Err(err) => {
                            warn!(provider = %attempt.name, error = %err, "读取上游回复失败");
                            break Some(crate::upstream_resilience::classify_network());
                        }
                    };
                    if !held.saw_tool {
                        text_tries += 1;
                        match crate::upstream_resilience::text_only_step(
                            text_tries,
                            attempt_idx,
                            attempt_idx + 1 < attempts.len(),
                            paid_text_total(attempt, effects),
                        ) {
                            crate::upstream_resilience::TextOnlyStep::RetrySame => {
                                warn!(
                                    provider = %attempt.name,
                                    model = %attempt.model_name,
                                    try_index = text_tries,
                                    "当前模型未返回工具调用，再要一次"
                                );
                                continue;
                            }
                            crate::upstream_resilience::TextOnlyStep::NextRoute => {
                                warn!(
                                    provider = %attempt.name,
                                    model = %attempt.model_name,
                                    try_index = text_tries,
                                    "当前模型多次未返回工具调用，换下一条"
                                );
                                break None;
                            }
                            crate::upstream_resilience::TextOnlyStep::Forward => {
                                warn!(
                                    provider = %attempt.name,
                                    model = %attempt.model_name,
                                    try_index = text_tries,
                                    "多次未返回工具调用，交回文本"
                                );
                            }
                        }
                    }
                    state.reset_circuit(&key).await;
                    let delivered = if stream {
                        let replay = replay_upstream(held.prefix, held.rest);
                        anthropic_adapter::anthropic_stream_response(
                            replay.map(|item| item.map_err(std::io::Error::other)),
                            client_model.clone(),
                            Some(state.stats.clone()),
                            Some(client),
                            Some(attempt.name.clone()),
                        )
                    } else {
                        let mut raw = Vec::new();
                        for chunk in held.prefix {
                            raw.extend_from_slice(&chunk);
                        }
                        anthropic_adapter::anthropic_json_response(
                            Bytes::from(raw),
                            &client_model,
                            Some(state.stats.clone()),
                            Some(client.as_str()),
                            Some(attempt.name.as_str()),
                        )
                        .await
                    };
                    return delivered.map_err(|_| {
                        crate::upstream_resilience::FailureClass::Terminal(
                            crate::upstream_resilience::NoticeKind::UpstreamDown,
                        )
                    });
                }
                state.reset_circuit(&key).await;
                return anthropic_adapter::openai_to_anthropic_response(
                    response,
                    &client_model,
                    Some(state.stats.clone()),
                    Some(client.as_str()),
                    Some(attempt.name.as_str()),
                )
                .await
                .map_err(|_| {
                    crate::upstream_resilience::FailureClass::Terminal(
                        crate::upstream_resilience::NoticeKind::UpstreamDown,
                    )
                });
            }
            Ok(response) => {
                let status = response.status();
                let retry_after = header_str(response.headers(), "retry-after");
                let text = response.text().await.unwrap_or_default();
                warn!(
                    provider = %attempt.name,
                    model = %attempt.model_name,
                    %status,
                    body = %text,
                    "anthropic adapter upstream error"
                );
                break Some(crate::upstream_resilience::classify_http(
                    status.as_u16(),
                    &text,
                    retry_after.as_deref(),
                    own_key,
                    is_free,
                ));
            }
            Err(err) => {
                warn!(provider = %attempt.name, error = %err, "anthropic adapter request error");
                break Some(crate::upstream_resilience::classify_network());
            }
        }
        };
        let Some(class) = class else {
            continue;
        };
        worst = Some(match worst {
            Some(prev) => crate::upstream_resilience::merge_failure(prev, class),
            None => class,
        });
        if matches!(
            class,
            crate::upstream_resilience::FailureClass::Terminal(
                crate::upstream_resilience::NoticeKind::ContextTooLong
            )
        ) {
            return Err(class);
        }
    }
    Err(worst.unwrap_or(crate::upstream_resilience::FailureClass::Terminal(
        crate::upstream_resilience::NoticeKind::UpstreamDown,
    )))
}

async fn proxy_request(
    state: GatewayState,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let path = request.uri().path().trim_start_matches('/').to_string();
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    let method = request.method().clone();
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await?;
    let config = state.config().await;

    if path == "v1/responses" || path.starts_with("v1/responses/") {
        let (body_bytes, rules_injected) =
            apply_workspace_rules(body_bytes, &config, RequestShape::OpenAiResponses, &headers);
        let stream = crate::anthropic_adapter::request_wants_stream(&body_bytes);
        let client_model = client_model_of(&body_bytes);
        let run = {
            let state = state.clone();
            let headers = headers.clone();
            let method = method.clone();
            let body_bytes = body_bytes.clone();
            let query = query.clone();
            let client_model = client_model.clone();
            async move {
                match proxy_responses_via_chat(
                    state,
                    headers,
                    method,
                    body_bytes,
                    &query,
                    rules_injected,
                )
                .await
                {
                    Ok(response) => response,
                    Err(err) => {
                        warn!(error = %err, "codex responses 上游失败，改为会话内提示");
                        session_notice(
                            crate::upstream_resilience::Wire::Responses,
                            &client_model,
                            crate::upstream_resilience::NoticeKind::UpstreamDown,
                            stream,
                        )
                    }
                }
            }
        };
        // 流式请求立刻交出 SSE，等待期间只发注释，避免首包超时把会话掐断。
        if stream {
            return Ok(hold_stream(run));
        }
        return Ok(hold_json(run));
    }

    let (body_bytes, rules_injected) =
        apply_workspace_rules(body_bytes, &config, RequestShape::OpenAiChat, &headers);
    let stream = crate::anthropic_adapter::request_wants_stream(&body_bytes);
    let client_model = client_model_of(&body_bytes);
    let run = proxy_chat_upstream(
        state,
        headers,
        method,
        path,
        query,
        body_bytes,
        rules_injected,
        config,
    );
    if stream {
        let client_model = client_model.clone();
        return Ok(hold_stream(async move {
            match run.await {
                Ok(response) => response,
                Err(err) => {
                    warn!(error = %err, "chat 上游失败，改为会话内提示");
                    session_notice(
                        crate::upstream_resilience::Wire::Chat,
                        &client_model,
                        crate::upstream_resilience::NoticeKind::UpstreamDown,
                        true,
                    )
                }
            }
        }));
    }
    let client_model = client_model.clone();
    return Ok(hold_json(async move {
        match run.await {
            Ok(response) => response,
            Err(err) => {
                warn!(error = %err, "chat 上游失败，改为会话内提示");
                session_notice(
                    crate::upstream_resilience::Wire::Chat,
                    &client_model,
                    crate::upstream_resilience::NoticeKind::UpstreamDown,
                    false,
                )
            }
        }
    }));
}

async fn proxy_chat_upstream(
    state: GatewayState,
    headers: HeaderMap,
    method: http::Method,
    path: String,
    query: String,
    body_bytes: Bytes,
    rules_injected: bool,
    config: AppConfig,
) -> Result<Response<Body>> {
    let (effects, is_codex) = request_rule_effects(&state, &headers, false).await;
    let client_model = client_model_of(&body_bytes);
    let route = request_route(&state, &config, &client_model).await;
    let box_id = route.box_id;
    let cooldown_ms = route.cooldown_ms;
    let providers = route.providers;
    if providers.is_empty() {
        bail!("没有可用模型配置");
    }
    let stream = crate::anthropic_adapter::request_wants_stream(&body_bytes);
    let tools = crate::upstream_resilience::request_declares_tools(&body_bytes);
    let body_bytes = apply_rule_body_fields(body_bytes, &effects);
    let mut memory = crate::upstream_resilience::FailureMemory::new();
    let mut budget = retry_budget(&config);
    let started = std::time::Instant::now();

    for pass in 0..crate::upstream_resilience::MAX_PASSES {
        memory.begin_pass();
        for (index, provider) in providers.iter().enumerate() {
            let is_free = crate::experimental_free::is_experimental_free(provider);
            memory.set_free(is_free);
            if !is_free && state.is_circuit_open(&provider.id).await {
                warn!(provider = %provider.name, "provider 处于熔断冷却期，跳过");
                continue;
            }
            let attempts = upstream_attempts(provider, config.failover_enabled);
            for (attempt_idx, attempt) in attempts.iter().enumerate() {
                let key = route_key(attempt);
                if state.is_circuit_open(&key).await {
                    continue;
                }
                let target = build_target_url(&attempt.base_url, &path, &query);
                let payload = rewrite_model(
                    body_bytes.clone(),
                    &attempt.model_name,
                    rule_default_max_tokens(&effects, is_codex),
                );
                let mut text_tries = 0u32;
                let attempt_started = std::time::Instant::now();
                let class = loop {
                    let mut builder = state.client.request(method.clone(), &target);
                    builder = copy_forward_headers(
                        builder,
                        &headers,
                        &attempt.api_key,
                        AuthMode::OpenAi,
                        &effects.headers,
                    )?;
                    info!(
                        provider = %attempt.name,
                        model = %attempt.model_name,
                        base_url = %attempt.base_url,
                        path = %path,
                        pass,
                        "proxying request"
                    );
                    let own_key = !attempt.api_key.trim().is_empty();
                    match builder.body(payload.clone()).send().await {
                        Ok(response) if response.status().is_success() => {
                            if tools {
                                let held = match if stream {
                                    hold_chat_sse(response).await
                                } else {
                                    hold_chat_json(response).await
                                } {
                                    Ok(held) => held,
                                    Err(err) => {
                                        warn!(provider = %attempt.name, error = %err, "读取上游回复失败");
                                        break Some(crate::upstream_resilience::classify_network());
                                    }
                                };
                                if !held.saw_tool {
                                    text_tries += 1;
                                    match crate::upstream_resilience::text_only_step(
                                        text_tries,
                                        attempt_idx,
                                        attempt_idx + 1 < attempts.len(),
                                        paid_text_total(attempt, &effects),
                                    ) {
                                        crate::upstream_resilience::TextOnlyStep::RetrySame => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "当前模型未返回工具调用，再要一次"
                                            );
                                            continue;
                                        }
                                        crate::upstream_resilience::TextOnlyStep::NextRoute => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "当前模型多次未返回工具调用，换下一条"
                                            );
                                            break None;
                                        }
                                        crate::upstream_resilience::TextOnlyStep::Forward => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "多次未返回工具调用，交回文本"
                                            );
                                        }
                                    }
                                }
                                state.reset_circuit(&key).await;
                                state
                                    .record_proxy_hit(
                                        attempt,
                                        &path,
                                        index,
                                        started.elapsed().as_millis() as u64,
                                        &headers,
                                        "openai_direct",
                                        true,
                                        rules_injected,
                                    )
                                    .await;
                                if let Some(box_id) = box_id.as_deref() {
                                    state
                                        .note_member_success(
                                            box_id,
                                            provider,
                                            attempt_started.elapsed().as_millis() as u64,
                                        )
                                        .await;
                                }
                                return if stream {
                                    chat_stream_from_hold(
                                        held,
                                        Some(state.stats.clone()),
                                        Some(token_attr_from(&headers, attempt)),
                                        &client_model,
                                    )
                                } else {
                                    json_from_hold(held, &client_model)
                                };
                            }
                            state.reset_circuit(&key).await;
                            state
                                .record_proxy_hit(
                                    attempt,
                                    &path,
                                    index,
                                    started.elapsed().as_millis() as u64,
                                    &headers,
                                    "openai_direct",
                                    true,
                                    rules_injected,
                                )
                                .await;
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_success(
                                        box_id,
                                        provider,
                                        attempt_started.elapsed().as_millis() as u64,
                                    )
                                    .await;
                            }
                            return into_axum_response(
                                response,
                                Some(state.stats.clone()),
                                Some(token_attr_from(&headers, attempt)),
                                &client_model,
                            )
                            .await;
                        }
                        Ok(response) => {
                            let status = response.status();
                            let retry_after = header_str(response.headers(), "retry-after");
                            let text = response.text().await.unwrap_or_default();
                            warn!(
                                provider = %attempt.name,
                                model = %attempt.model_name,
                                %status,
                                body = %text,
                                "provider returned error"
                            );
                            break Some(crate::upstream_resilience::classify_http(
                                status.as_u16(),
                                &text,
                                retry_after.as_deref(),
                                own_key,
                                is_free,
                            ));
                        }
                        Err(err) => {
                            warn!(provider = %attempt.name, error = %err, "provider request error");
                            break Some(crate::upstream_resilience::classify_network());
                        }
                    }
                };
                let Some(class) = class else {
                    continue;
                };
                if let Some(box_id) = box_id.as_deref() {
                    state
                        .note_member_failure(box_id, provider, &format!("{class:?}"), cooldown_ms)
                        .await;
                }
                if let Some(kind) = memory.observe(class) {
                    remember_route_failure(&state, attempt).await;
                    return Ok(session_notice(
                        crate::upstream_resilience::Wire::Chat,
                        &client_model,
                        kind,
                        stream,
                    ));
                }
            }
        }
        if let Some(response) = finish_pass(
            &mut memory,
            &mut budget,
            pass,
            &client_model,
            crate::upstream_resilience::Wire::Chat,
            stream,
        )
        .await
        {
            return Ok(response);
        }
    }
    Ok(session_notice(
        crate::upstream_resilience::Wire::Chat,
        &client_model,
        memory.notice_kind(),
        stream,
    ))
}

async fn proxy_responses_via_chat(
    state: GatewayState,
    headers: HeaderMap,
    method: http::Method,
    body_bytes: Bytes,
    query: &str,
    rules_injected: bool,
) -> Result<Response<Body>> {
    let config = state.config().await;
    // /v1/responses 只由 Codex 走：is_codex 恒为真
    let (effects, _is_codex) = request_rule_effects(&state, &headers, true).await;
    let client_model = client_model_of(&body_bytes);
    let route = request_route(&state, &config, &client_model).await;
    let box_id = route.box_id;
    let cooldown_ms = route.cooldown_ms;
    let providers = route.providers;
    if providers.is_empty() {
        bail!("没有可用模型配置");
    }

    let stream = serde_json::from_slice::<Value>(&body_bytes)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);
    let mut memory = crate::upstream_resilience::FailureMemory::new();
    let mut budget = retry_budget(&config);
    let started = std::time::Instant::now();

    for pass in 0..crate::upstream_resilience::MAX_PASSES {
        memory.begin_pass();
    for (index, provider) in providers.iter().enumerate() {
        let is_free = crate::experimental_free::is_experimental_free(provider);
        memory.set_free(is_free);
        if !is_free && state.is_circuit_open(&provider.id).await {
            warn!(provider = %provider.name, "provider 处于熔断冷却期，跳过");
            continue;
        }

        let attempts = upstream_attempts(provider, config.failover_enabled);
        for (attempt_idx, attempt) in attempts.iter().enumerate() {
            let key = route_key(attempt);
            if state.is_circuit_open(&key).await {
                continue;
            }
            let own_key = !attempt.api_key.trim().is_empty();
            let attempt_started = std::time::Instant::now();
            // Codex 几乎每轮都带 tools。火山原生 /responses 常「200 纯文本收工」不发
            // tool call；带 tools 时一律走 Chat Completions 桥，保证任意 OpenAI 兼容 LLM
            // 都能跑完工具环，而不是只宣布计划就停。
            let request_has_tools =
                crate::openai_responses_adapter::responses_body_declares_tools(&body_bytes);
            let try_native =
                crate::openai_responses_adapter::should_try_native_responses_for_body(
                    &attempt.base_url,
                    &attempt.model_name,
                    &body_bytes,
                );
            info!(
                provider = %attempt.name,
                base_url = %attempt.base_url,
                model = %attempt.model_name,
                try_native_responses = try_native,
                request_has_tools,
                attempt = attempt_idx,
                "codex responses routing"
            );

            if try_native {
                let native_base = crate::openai_responses_adapter::native_responses_base_url(
                    &attempt.base_url,
                    &attempt.model_name,
                );
                let upstream_body =
                    crate::openai_responses_adapter::rewrite_responses_upstream_model(
                        &body_bytes,
                        &attempt.model_name,
                    )?;
                let upstream_body = apply_rule_body_fields(upstream_body, &effects);
                let target = build_target_url(&native_base, "v1/responses", query);
                let mut builder = state.client.request(method.clone(), &target);
                builder = copy_forward_headers(
                    builder,
                    &headers,
                    &attempt.api_key,
                    AuthMode::OpenAi,
                    &effects.headers,
                )?;

                info!(
                    provider = %attempt.name,
                    model = %attempt.model_name,
                    native_base = %native_base,
                    target_url = %target,
                    path = "v1/responses",
                    stream,
                    mode = "native_responses_passthrough",
                    "proxying codex responses to upstream Responses API"
                );

                match builder.body(upstream_body).send().await {
                    Ok(response) if response.status().is_success() => {
                        state.reset_circuit(&key).await;
                        state
                            .record_proxy_hit(
                                attempt,
                                "v1/responses",
                                index,
                                started.elapsed().as_millis() as u64,
                                &headers,
                                "responses_native",
                                true,
                                rules_injected,
                            )
                            .await;
                        if let Some(box_id) = box_id.as_deref() {
                            state
                                .note_member_success(
                                    box_id,
                                    provider,
                                    attempt_started.elapsed().as_millis() as u64,
                                )
                                .await;
                        }
                        return into_axum_response(
                            response,
                            Some(state.stats.clone()),
                            Some(token_attr_from(&headers, attempt)),
                            &client_model,
                        )
                        .await;
                    }
                    Ok(response) if response.status().as_u16() < 500 && !is_free => {
                        let status = response.status();
                        let retry_after = header_str(response.headers(), "retry-after");
                        let text = response.text().await.unwrap_or_default();
                        let class = crate::upstream_resilience::classify_http(
                            status.as_u16(),
                            &text,
                            retry_after.as_deref(),
                            own_key,
                            is_free,
                        );
                        if let Some(box_id) = box_id.as_deref() {
                            state
                                .note_member_failure(
                                    box_id,
                                    provider,
                                    &format!("HTTP {} {}", status.as_u16(), text),
                                    cooldown_ms,
                                )
                                .await;
                        }
                        if let Some(kind) = memory.observe(class) {
                            return Ok(session_notice(
                                crate::upstream_resilience::Wire::Responses,
                                &client_model,
                                kind,
                                stream,
                            ));
                        }
                        break;
                    }
                    Ok(response) => {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        warn!(
                            provider = %attempt.name,
                            model = %attempt.model_name,
                            %status,
                            body = %text,
                            "native responses upstream error, will try chat/completions"
                        );
                    }
                    Err(err) => {
                        warn!(
                            provider = %attempt.name,
                            error = %err,
                            "native responses request failed, will try chat/completions"
                        );
                    }
                }
            }

            let (chat_body_base, tool_meta_base) =
                crate::openai_responses_adapter::responses_to_chat_with_meta_with_default(
                    &body_bytes,
                    &attempt.model_name,
                    rule_default_max_tokens(&effects, true),
                )?;
            let chat_body_base = apply_rule_body_fields(chat_body_base, &effects);
            let target = build_target_url(&attempt.base_url, "v1/chat/completions", query);
            let has_tools = crate::command_mode::chat_body_has_tools(&chat_body_base);
            // 对齐 cc-switch / 官方 Gateway 要求：先保留 tools 走 Chat Completions 转换，
            // 绝不为「免费通道」抢先扒掉 tools（那会直接毁掉 Codex 工具环）。
            // 仅当上游明确拒 FC（tools_unsupported_hint）时，同 attempt 再降级命令模式一次。
            let mut use_command_mode = false;
            let mut command_mode_retried = false;
            let mut text_tries = 0u32;

            // 同一 attempt 最多两次：原生 tools →（若拒 FC）命令模式重试一次。
            loop {
                let mut tool_meta = tool_meta_base.clone();
                let chat_body = if use_command_mode {
                    tool_meta.command_mode = true;
                    let shell = crate::command_mode::preferred_shell_tool_name(
                        &tool_meta.flattened_names,
                        &tool_meta.custom_names,
                    );
                    crate::command_mode::prepare_chat_body_for_command_mode(
                        &chat_body_base,
                        &shell,
                    )?
                } else {
                    chat_body_base.clone()
                };

                let mut builder = state.client.request(method.clone(), &target);
                builder = copy_forward_headers(
                    builder,
                    &headers,
                    &attempt.api_key,
                    AuthMode::OpenAi,
                    &effects.headers,
                )?;

                info!(
                    provider = %attempt.name,
                    model = %attempt.model_name,
                    base_url = %attempt.base_url,
                    target_url = %target,
                    path = "v1/responses",
                    stream,
                    attempt = attempt_idx,
                    mode = "chat_completions_adapter",
                    command_mode = use_command_mode,
                    custom_tools = tool_meta.custom_names.len(),
                    "proxying codex responses via chat completions"
                );

                let estimated_input = crate::quick_chat::estimate_chat_input_tokens(
                    std::str::from_utf8(&chat_body).unwrap_or(""),
                );

                if stream {
                    match builder.body(chat_body).send().await {
                        Ok(response) if response.status().is_success() => {
                            if has_tools {
                                let held = match hold_chat_sse(response).await {
                                    Ok(held) => held,
                                    Err(err) => {
                                        warn!(
                                            provider = %attempt.name,
                                            error = %err,
                                            "读取 Codex 上游流失败"
                                        );
                                        if let Some(kind) = memory.observe(
                                            crate::upstream_resilience::classify_network(),
                                        ) {
                                            return Ok(session_notice(
                                                crate::upstream_resilience::Wire::Responses,
                                                &client_model,
                                                kind,
                                                stream,
                                            ));
                                        }
                                        break;
                                    }
                                };
                                if !held.saw_tool {
                                    text_tries += 1;
                                    match crate::upstream_resilience::text_only_step(
                                        text_tries,
                                        attempt_idx,
                                        attempt_idx + 1 < attempts.len(),
                                        paid_text_total(attempt, &effects),
                                    ) {
                                        crate::upstream_resilience::TextOnlyStep::RetrySame => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "当前模型未返回工具调用，再要一次"
                                            );
                                            continue;
                                        }
                                        crate::upstream_resilience::TextOnlyStep::NextRoute => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "当前模型多次未返回工具调用，换下一条"
                                            );
                                            break;
                                        }
                                        crate::upstream_resilience::TextOnlyStep::Forward => {
                                            warn!(
                                                provider = %attempt.name,
                                                model = %attempt.model_name,
                                                try_index = text_tries,
                                                "多次未返回工具调用，交回文本"
                                            );
                                        }
                                    }
                                }
                                state.reset_circuit(&key).await;
                                state
                                    .record_proxy_hit(
                                        attempt,
                                        "v1/responses",
                                        index,
                                        started.elapsed().as_millis() as u64,
                                        &headers,
                                        "chat_completions_adapter",
                                        true,
                                        rules_injected,
                                    )
                                    .await;
                                if let Some(box_id) = box_id.as_deref() {
                                    state
                                        .note_member_success(
                                            box_id,
                                            provider,
                                            attempt_started.elapsed().as_millis() as u64,
                                        )
                                        .await;
                                }
                                return responses_stream_from_hold(
                                    held,
                                    client_model.clone(),
                                    tool_meta,
                                    estimated_input,
                                    Some(state.stats.clone()),
                                    Some(token_attr_from(&headers, attempt)),
                                );
                            }
                            state.reset_circuit(&key).await;
                            state
                                .record_proxy_hit(
                                    attempt,
                                    "v1/responses",
                                    index,
                                    started.elapsed().as_millis() as u64,
                                    &headers,
                                    "chat_completions_adapter",
                                    true,
                                    rules_injected,
                                )
                                .await;
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_success(
                                        box_id,
                                        provider,
                                        attempt_started.elapsed().as_millis() as u64,
                                    )
                                    .await;
                            }
                            let model = client_model.clone();
                            let mapped = crate::openai_responses_adapter::chat_sse_to_responses_sse_with_meta(
                                response.bytes_stream(),
                                model,
                                tool_meta,
                                estimated_input,
                            );
                            let tapped = tap_stream_for_token_stats(
                                mapped,
                                Some(state.stats.clone()),
                                Some(token_attr_from(&headers, attempt)),
                            );
                            return event_stream_response(tapped);
                        }
                        Ok(response) if response.status().as_u16() < 500 => {
                            let status = response.status();
                            let retry_after = header_str(response.headers(), "retry-after");
                            let text = response.text().await.unwrap_or_default();
                            if has_tools
                                && !command_mode_retried
                                && crate::command_mode::tools_unsupported_hint(&text)
                            {
                                warn!(
                                    provider = %attempt.name,
                                    model = %attempt.model_name,
                                    %status,
                                    "上游不支持 function calling，改走命令模式重试"
                                );
                                use_command_mode = true;
                                command_mode_retried = true;
                                continue;
                            }
                            let class = crate::upstream_resilience::classify_http(
                                status.as_u16(),
                                &text,
                                retry_after.as_deref(),
                                own_key,
                                is_free,
                            );
                            warn!(
                                provider = %attempt.name,
                                model = %attempt.model_name,
                                %status,
                                body = %text,
                                "codex chat stream upstream error"
                            );
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_failure(
                                        box_id,
                                        provider,
                                        &format!("HTTP {} {}", status.as_u16(), text),
                                        cooldown_ms,
                                    )
                                    .await;
                            }
                            if let Some(kind) = memory.observe(class) {
                                return Ok(session_notice(
                                    crate::upstream_resilience::Wire::Responses,
                                    &client_model,
                                    kind,
                                    stream,
                                ));
                            }
                            break;
                        }
                        Ok(response) => {
                            let status = response.status();
                            let retry_after = header_str(response.headers(), "retry-after");
                            let text = response.text().await.unwrap_or_default();
                            warn!(
                                provider = %attempt.name,
                                model = %attempt.model_name,
                                %status,
                                body = %text,
                                "codex chat stream upstream error, trying next attempt"
                            );
                            let class = crate::upstream_resilience::classify_http(
                                status.as_u16(),
                                &text,
                                retry_after.as_deref(),
                                own_key,
                                is_free,
                            );
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_failure(
                                        box_id,
                                        provider,
                                        &format!("HTTP {} {}", status.as_u16(), text),
                                        cooldown_ms,
                                    )
                                    .await;
                            }
                            if let Some(kind) = memory.observe(class) {
                                return Ok(session_notice(
                                    crate::upstream_resilience::Wire::Responses,
                                    &client_model,
                                    kind,
                                    stream,
                                ));
                            }
                            break;
                        }
                        Err(err) => {
                            warn!(
                                provider = %attempt.name,
                                error = %err,
                                "codex chat stream request error"
                            );
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_failure(
                                        box_id,
                                        provider,
                                        &err.to_string(),
                                        cooldown_ms,
                                    )
                                    .await;
                            }
                            if let Some(kind) = memory.observe(crate::upstream_resilience::classify_network()) {
                                return Ok(session_notice(
                                    crate::upstream_resilience::Wire::Responses,
                                    &client_model,
                                    kind,
                                    stream,
                                ));
                            }
                            break;
                        }
                    }
                } else {
                match builder.body(chat_body).send().await {
                    Ok(response)
                        if response.status().is_success() || response.status().as_u16() < 500 =>
                    {
                        let status = response.status();
                        let retry_after = header_str(response.headers(), "retry-after");
                        let body_text = response.text().await.unwrap_or_default();
                        let looks_error = !status.is_success()
                            || crate::openai_responses_adapter::is_openai_compat_error_body(
                                &body_text,
                            );
                        if looks_error
                            && has_tools
                            && !command_mode_retried
                            && crate::command_mode::tools_unsupported_hint(&body_text)
                        {
                            warn!(
                                provider = %attempt.name,
                                model = %attempt.model_name,
                                %status,
                                "上游不支持 function calling，改走命令模式重试"
                            );
                            use_command_mode = true;
                            command_mode_retried = true;
                            continue;
                        }
                        if looks_error && !status.is_success() {
                            warn!(
                                provider = %attempt.name,
                                model = %attempt.model_name,
                                %status,
                                body = %body_text,
                                "upstream error for codex responses"
                            );
                            let class = crate::upstream_resilience::classify_http(
                                status.as_u16(),
                                &body_text,
                                retry_after.as_deref(),
                                own_key,
                                is_free,
                            );
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_failure(
                                        box_id,
                                        provider,
                                        &format!("HTTP {} {}", status.as_u16(), body_text),
                                        cooldown_ms,
                                    )
                                    .await;
                            }
                            if let Some(kind) = memory.observe(class) {
                                return Ok(session_notice(
                                    crate::upstream_resilience::Wire::Responses,
                                    &client_model,
                                    kind,
                                    stream,
                                ));
                            }
                            break;
                        }
                        if looks_error && is_free {
                            if let Some(box_id) = box_id.as_deref() {
                                state
                                    .note_member_failure(
                                        box_id,
                                        provider,
                                        &format!("HTTP {} {}", status.as_u16(), body_text),
                                        cooldown_ms,
                                    )
                                    .await;
                            }
                            let _ = memory.observe(crate::upstream_resilience::FailureClass::Switch);
                            break;
                        }
                        if status.is_success()
                            && has_tools
                            && crate::upstream_resilience::assistant_text_omits_tools(&body_text)
                        {
                            text_tries += 1;
                            match crate::upstream_resilience::text_only_step(
                                text_tries,
                                attempt_idx,
                                attempt_idx + 1 < attempts.len(),
                                paid_text_total(attempt, &effects),
                            ) {
                                crate::upstream_resilience::TextOnlyStep::RetrySame => {
                                    warn!(
                                        provider = %attempt.name,
                                        model = %attempt.model_name,
                                        try_index = text_tries,
                                        "当前模型未返回工具调用，再要一次"
                                    );
                                    continue;
                                }
                                crate::upstream_resilience::TextOnlyStep::NextRoute => {
                                    warn!(
                                        provider = %attempt.name,
                                        model = %attempt.model_name,
                                        try_index = text_tries,
                                        "当前模型多次未返回工具调用，换下一条"
                                    );
                                    break;
                                }
                                crate::upstream_resilience::TextOnlyStep::Forward => {
                                    warn!(
                                        provider = %attempt.name,
                                        model = %attempt.model_name,
                                        try_index = text_tries,
                                        "多次未返回工具调用，交回文本"
                                    );
                                }
                            }
                        }
                        state.reset_circuit(&key).await;
                        state
                            .record_proxy_hit(
                                attempt,
                                "v1/responses",
                                index,
                                started.elapsed().as_millis() as u64,
                                &headers,
                                "chat_completions_adapter",
                                status.is_success(),
                                rules_injected,
                            )
                            .await;
                        if status.is_success() {
                            let (input, output) = parse_usage_from_bytes(body_text.as_bytes())
                                .unwrap_or_else(|| {
                                    (
                                        estimated_input,
                                        crate::quick_chat::estimate_output_tokens(&body_text),
                                    )
                                });
                            if input > 0 || output > 0 {
                                let attr = token_attr_from(&headers, attempt);
                                state
                                    .stats
                                    .record_tokens_attributed(
                                        input,
                                        output,
                                        Some(attr.0.as_str()),
                                        Some(attr.1.as_str()),
                                    )
                                    .await;
                            }
                        }
                        let bytes = if status.is_success() {
                            crate::openai_responses_adapter::chat_json_to_responses_body_with_meta(
                                &body_text,
                                &client_model,
                                &tool_meta,
                            )?
                        } else {
                            Bytes::from(body_text)
                        };
                        if let Some(box_id) = box_id.as_deref() {
                            state
                                .note_member_success(
                                    box_id,
                                    provider,
                                    attempt_started.elapsed().as_millis() as u64,
                                )
                                .await;
                        }
                        return Ok(Response::builder()
                            .status(status)
                            .header(http::header::CONTENT_TYPE, "application/json")
                            .body(Body::from(bytes))?);
                    }
                    Ok(response) => {
                        let status = response.status();
                        let retry_after = header_str(response.headers(), "retry-after");
                        let text = response.text().await.unwrap_or_default();
                        let class = crate::upstream_resilience::classify_http(
                            status.as_u16(),
                            &text,
                            retry_after.as_deref(),
                            own_key,
                            is_free,
                        );
                        if let Some(box_id) = box_id.as_deref() {
                            state
                                .note_member_failure(
                                    box_id,
                                    provider,
                                    &format!("HTTP {} {}", status.as_u16(), text),
                                    cooldown_ms,
                                )
                                .await;
                        }
                        if let Some(kind) = memory.observe(class) {
                            return Ok(session_notice(
                                crate::upstream_resilience::Wire::Responses,
                                &client_model,
                                kind,
                                stream,
                            ));
                        }
                        break;
                    }
                    Err(err) => {
                        warn!(provider = %attempt.name, error = %err, "codex responses request error");
                        if let Some(box_id) = box_id.as_deref() {
                            state
                                .note_member_failure(
                                    box_id,
                                    provider,
                                    &err.to_string(),
                                    cooldown_ms,
                                )
                                .await;
                        }
                        if let Some(kind) =
                            memory.observe(crate::upstream_resilience::classify_network())
                        {
                            return Ok(session_notice(
                                crate::upstream_resilience::Wire::Responses,
                                &client_model,
                                kind,
                                stream,
                            ));
                        }
                        break;
                    }
                }
                } // else non-stream
            } // command_mode retry loop
        }

    }
        if let Some(response) = finish_pass(
            &mut memory,
            &mut budget,
            pass,
            &client_model,
            crate::upstream_resilience::Wire::Responses,
            stream,
        )
        .await
        {
            return Ok(response);
        }
    }

    Ok(session_notice(
        crate::upstream_resilience::Wire::Responses,
        &client_model,
        memory.notice_kind(),
        stream,
    ))
}

fn client_model_of(body: &[u8]) -> String {
    crate::upstream_resilience::client_visible_model(body, crate::model::CLIENT_MODEL_ID)
}

fn retry_budget(config: &AppConfig) -> crate::upstream_resilience::RetryBudget {
    crate::upstream_resilience::RetryBudget::from_config(
        config.upstream_retry_budget_ms,
        config.upstream_retry_single_ms,
    )
}

fn route_key(provider: &ProviderConfig) -> String {
    crate::upstream_resilience::route_circuit_key(
        &provider.id,
        &provider.base_url,
        &provider.model_name,
        crate::experimental_free::is_experimental_free(provider),
    )
}

fn header_str(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn session_notice(
    wire: crate::upstream_resilience::Wire,
    model: &str,
    kind: crate::upstream_resilience::NoticeKind,
    stream: bool,
) -> Response<Body> {
    crate::upstream_resilience::notice_response(wire, model, kind, stream)
}

async fn remember_route_failure(state: &GatewayState, attempt: &ProviderConfig) {
    state.trip_circuit(&route_key(attempt)).await;
}

/// 这一轮路由都失败了：还有预算就等待后返回 None，调用方再跑一轮；否则给出提示。
/// 不在这里记熔断。下一条用户消息应再打上游，而不是 30 秒内直接拿到提示。
async fn finish_pass(
    memory: &mut crate::upstream_resilience::FailureMemory,
    budget: &mut crate::upstream_resilience::RetryBudget,
    pass: u32,
    client_model: &str,
    wire: crate::upstream_resilience::Wire,
    stream: bool,
) -> Option<Response<Body>> {
    match memory.finish(budget, pass) {
        crate::upstream_resilience::PassFinish::Sleep(wait) => {
            info!(
                wait_ms = wait.as_millis() as u64,
                pass,
                "上游失败，同一次请求内等待后重试"
            );
            tokio::time::sleep(wait).await;
            None
        }
        crate::upstream_resilience::PassFinish::Notice(kind) => {
            Some(session_notice(wire, client_model, kind, stream))
        }
    }
}

fn hold_json<F>(fut: F) -> Response<Body>
where
    F: std::future::Future<Output = Response<Body>> + Send + 'static,
{
    hold_until_ready(fut, b"\n", "application/json")
}

fn hold_stream<F>(fut: F) -> Response<Body>
where
    F: std::future::Future<Output = Response<Body>> + Send + 'static,
{
    hold_until_ready(fut, b": ping\n\n", "text/event-stream; charset=utf-8")
}

fn hold_until_ready<F>(
    fut: F,
    keepalive: &'static [u8],
    content_type: &'static str,
) -> Response<Body>
where
    F: std::future::Future<Output = Response<Body>> + Send + 'static,
{
    let stream = async_stream::stream! {
        tokio::pin!(fut);
        loop {
            tokio::select! {
                biased;
                ready = &mut fut => {
                    let mut inner = ready.into_body().into_data_stream();
                    while let Some(chunk) = inner.next().await {
                        yield chunk.map_err(std::io::Error::other);
                    }
                    break;
                }
                _ = tokio::time::sleep(Duration::from_secs(15)) => {
                    yield Ok::<Bytes, std::io::Error>(Bytes::from_static(keepalive));
                }
            }
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn rewrite_model_stream<S>(
    stream: S,
    model: String,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send,
{
    async_stream::stream! {
        let mut rewriter = crate::upstream_resilience::ModelRewriter::new(model);
        tokio::pin!(stream);
        while let Some(item) = stream.next().await {
            match item {
                Ok(bytes) => {
                    let out = rewriter.push(&bytes);
                    if !out.is_empty() {
                        yield Ok(Bytes::from(out));
                    }
                }
                Err(err) => yield Err(err),
            }
        }
        let tail = rewriter.finish();
        if !tail.is_empty() {
            yield Ok(Bytes::from(tail));
        }
    }
}

fn ordered_providers(config: &AppConfig) -> Vec<ProviderConfig> {
    let mut providers = Vec::new();
    if let Some(active) = select_provider(config) {
        providers.push(active.clone());
    }

    if config.failover_enabled {
        for provider in &config.providers {
            if provider.enabled && !providers.iter().any(|item| item.id == provider.id) {
                providers.push(provider.clone());
            }
        }
    }

    providers
}

fn select_provider(config: &AppConfig) -> Option<&ProviderConfig> {
    if let Some(active_id) = &config.active_provider_id {
        if let Some(provider) = config
            .providers
            .iter()
            .find(|provider| provider.enabled && provider.id == *active_id)
        {
            return Some(provider);
        }
    }
    config.providers.iter().find(|provider| provider.enabled)
}

/// 一次请求的选路结果：attempt 链 + 命中盒子时的 id 与冷却配置。
struct Route {
    providers: Vec<ProviderConfig>,
    box_id: Option<String>,
    cooldown_ms: Option<u64>,
}

/// 触发键：客户端请求体 model 优先，其次当前激活 provider 的 model_name。
///
/// 只认 `enabled` 的盒子；两边都不命中返回 None，调用方回退 `ordered_providers`。
fn resolve_box<'a>(
    boxes: &'a [StrategyBox],
    client_model: &str,
    active_model: Option<&str>,
) -> Option<&'a StrategyBox> {
    let enabled = || boxes.iter().filter(|box_spec| box_spec.enabled);
    let client_model = client_model.trim();
    if !client_model.is_empty() {
        if let Some(hit) = enabled().find(|box_spec| box_spec.public_model_id.trim() == client_model) {
            return Some(hit);
        }
    }
    if let Some(active_model) = active_model.map(str::trim).filter(|model| !model.is_empty()) {
        if let Some(hit) = enabled().find(|box_spec| box_spec.public_model_id.trim() == active_model) {
            return Some(hit);
        }
    }
    None
}

/// 成员 → `ProviderConfig` 克隆：跳过停用 provider、不存在的 provider 与注册表禁用的模型。
/// `model_name` 覆盖成成员自己的，保持传入的成员顺序。
fn expand_members(
    config: &AppConfig,
    members: &[strategy_box::StrategyMember],
    registry: &[ModelEntry],
) -> Vec<ProviderConfig> {
    let mut out = Vec::new();
    for member in members {
        let Some(provider) = config
            .providers
            .iter()
            .find(|provider| provider.enabled && provider.id == member.provider_id)
        else {
            continue;
        };
        // R2 注册表里显式禁用的模型不参与编排；没有条目的模型按「未登记」放行。
        if let Some(entry) = registry.iter().find(|entry| {
            entry.provider_id == member.provider_id && entry.model_name == member.model_name
        }) {
            if !entry.enabled {
                continue;
            }
        }
        let mut expanded = provider.clone();
        expanded.model_name = member.model_name.clone();
        out.push(expanded);
    }
    out
}

/// 按盒子模式排好成员再展开成 attempt 链（纯函数，便于单测）。
fn expand_box_chain(
    config: &AppConfig,
    box_spec: &StrategyBox,
    registry: &[ModelEntry],
    health: &HashMap<String, strategy_box::MemberHealth>,
    rotation: u64,
) -> Vec<ProviderConfig> {
    let ordered = strategy_box::order_members(
        &box_spec.id,
        &box_spec.members,
        box_spec.mode,
        box_spec.speed_threshold_ms,
        health,
        rotation,
    );
    expand_members(config, &ordered, registry)
}

/// 健康度键的反推：`{box_id}::{provider_id}::{model_name}`。
fn member_key_of(box_id: &str, provider: &ProviderConfig) -> String {
    format!("{box_id}::{}::{}", provider.id, provider.model_name)
}

/// 选路入口：命中策略盒子时用盒子成员链，否则与既有 `ordered_providers` 完全一致。
async fn request_route(state: &GatewayState, config: &AppConfig, client_model: &str) -> Route {
    let active_model = select_provider(config).map(|provider| provider.model_name.clone());
    let registry = state.model_registry_snapshot().await;
    let strategy = state.strategy_state();

    // 先用读锁判定是否命中：非盒子请求（绝大多数）不争写锁。
    let hit = {
        let guard = strategy.read().await;
        resolve_box(&guard.boxes, client_model, active_model.as_deref()).cloned()
    };
    let Some(box_spec) = hit else {
        return Route {
            providers: ordered_providers(config),
            box_id: None,
            cooldown_ms: None,
        };
    };

    // 命中后才推进轮换游标并读取健康度。
    let providers = {
        let mut guard = strategy.write().await;
        let rotation = guard.runtime.next_rotation(&box_spec.id);
        expand_box_chain(config, &box_spec, &registry, &guard.runtime.health, rotation)
    };
    if providers.is_empty() {
        // 成员全部无效：回退既有链路，不把请求判死。
        return Route {
            providers: ordered_providers(config),
            box_id: None,
            cooldown_ms: None,
        };
    }
    Route {
        providers,
        box_id: Some(box_spec.id),
        cooldown_ms: box_spec.cooldown_ms,
    }
}

pub(crate) fn build_target_url(base_url: &str, path: &str, query: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    let relative = if crate::model::openai_base_has_version_suffix(base) {
        path.strip_prefix("v1/")
            .unwrap_or(path)
            .trim_start_matches('/')
    } else {
        path
    };
    format!("{}/{}{}", base, relative, query)
}

/// 供「直接体验」等场景：按协议构造上游 chat/completions 地址。
pub fn build_chat_url_for_provider(provider: &ProviderConfig) -> String {
    match provider.protocol {
        ProviderProtocol::OpenAi => build_target_url(&provider.base_url, "v1/chat/completions", ""),
        ProviderProtocol::Anthropic => {
            // Anthropic 原生无 chat/completions；体验时仍走 OpenAI 兼容路径（多数网关兼容层）。
            // 若 base 是 anthropic 原生，改走 messages。
            if provider.base_url.contains("anthropic.com") {
                build_target_url(&provider.base_url, "v1/messages", "")
            } else {
                build_target_url(&provider.base_url, "v1/chat/completions", "")
            }
        }
    }
}

/// 规则不得覆盖的请求头：鉴权与 Anthropic 协议协商由网关自身决定。
const RULE_PROTECTED_HEADERS: &[&str] =
    &["authorization", "x-api-key", "anthropic-version", "anthropic-beta"];

/// 计算本次请求生效的规则效果（R6），同时返回是否为 Codex 语境。
///
/// `is_responses_path` 为真时视作 Codex 语境（`/v1/responses` 只由 Codex 走）。
async fn request_rule_effects(
    state: &GatewayState,
    headers: &HeaderMap,
    is_responses_path: bool,
) -> (RuleEffects, bool) {
    let rules = state.request_rules_snapshot().await;
    let client_label = crate::gateway_stats::client_label_from_headers(headers);
    let is_codex = is_responses_path || client_label.trim().eq_ignore_ascii_case("codex");
    let effects = request_rules::collect_effects(
        &rules,
        &RuleContext {
            client_label: &client_label,
            is_codex,
        },
    );
    (effects, is_codex)
}

/// 默认 max_tokens：规则关闭（仅 codex 语境）时返回 None，表示「不填」。
fn rule_default_max_tokens(effects: &RuleEffects, is_codex: bool) -> Option<u64> {
    if is_codex && effects.max_tokens_suppressed {
        return None;
    }
    Some(
        effects
            .max_tokens
            .map(u64::from)
            .unwrap_or(crate::openai_responses_adapter::DEFAULT_CODEX_MAX_TOKENS),
    )
}

/// 把规则的 body_fields 写进出站 JSON 对象；值先按 JSON 解析（数字/布尔/对象），
/// 解析失败再按字符串写入。请求体不是 JSON 对象时跳过并告警。
fn apply_rule_body_fields(body: Bytes, effects: &RuleEffects) -> Bytes {
    if body.is_empty() || effects.body_fields.is_empty() {
        return body;
    }
    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "请求体不是合法 JSON，跳过规则 body_fields");
            return body;
        }
    };
    let Some(object) = value.as_object_mut() else {
        warn!("请求体不是 JSON 对象，跳过规则 body_fields");
        return body;
    };
    for (key, raw) in &effects.body_fields {
        let parsed = serde_json::from_str::<Value>(raw)
            .unwrap_or_else(|_| Value::String(raw.clone()));
        object.insert(key.clone(), parsed);
    }
    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

fn rewrite_model(body: Bytes, model_name: &str, default_max: Option<u64>) -> Bytes {
    if body.is_empty() {
        return body;
    }

    match serde_json::from_slice::<Value>(&body) {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.insert("model".to_string(), Value::String(model_name.to_string()));
                let has_max = object
                    .get("max_tokens")
                    .or_else(|| object.get("max_completion_tokens"))
                    .is_some_and(|value| !value.is_null());
                if !has_max {
                    if let Some(default_max) = default_max {
                        object.insert("max_tokens".to_string(), json!(default_max));
                        warn!(
                            default_max,
                            "Chat 请求未指定 max_tokens，网关填充默认值以防上游截断"
                        );
                    }
                }
            }
            Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
        }
        Err(_) => body,
    }
}

enum AuthMode {
    OpenAi,
    Anthropic,
}

fn copy_forward_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
    api_key: &str,
    auth_mode: AuthMode,
    rule_headers: &[(String, String)],
) -> Result<reqwest::RequestBuilder> {
    // 先合并成一张表：客户端转发头 → 规则头同名覆盖（追加同名会让上游收到两个值）。
    let mut forwarded = HeaderMap::new();
    for (name, value) in headers {
        let key = name.as_str().to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "host"
                | "content-length"
                | "authorization"
                | "x-api-key"
                | "anthropic-version"
                | "anthropic-beta"
                | "accept-encoding"
        ) {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
    }
    for (key, value) in rule_headers {
        let lower = key.to_ascii_lowercase();
        if RULE_PROTECTED_HEADERS.contains(&lower.as_str()) {
            warn!(header = %key, "请求改写规则试图覆盖鉴权/协议头，已忽略");
            continue;
        }
        let name = match http::HeaderName::from_bytes(key.as_bytes()) {
            Ok(name) => name,
            Err(err) => {
                warn!(header = %key, error = %err, "规则请求头名称非法，已忽略");
                continue;
            }
        };
        let parsed = match HeaderValue::from_str(value) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(header = %key, error = %err, "规则请求头值非法，已忽略");
                continue;
            }
        };
        forwarded.insert(name, parsed);
    }
    for (name, value) in &forwarded {
        builder = builder.header(name.clone(), value.clone());
    }

    match auth_mode {
        AuthMode::OpenAi => {
            // 实验免费通道等 keyless：空 Key 不发 Authorization
            if !api_key.trim().is_empty() {
                let auth = HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .context("API Key 无法写入请求头")?;
                builder = builder.header(http::header::AUTHORIZATION, auth);
            }
        }
        AuthMode::Anthropic => {
            builder = builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
    }

    Ok(builder)
}

/// 网关主客户端：不设整段 timeout（Codex 流式工具环经常远超 10 分钟）。
pub fn build_upstream_client() -> Result<Client> {
    build_upstream_client_inner(None)
}

/// 短请求（探测 / 快捷对话）可带总超时；流式代理请用 [`build_upstream_client`]。
pub fn build_upstream_client_with_timeout(timeout: Duration) -> Result<Client> {
    build_upstream_client_inner(Some(timeout))
}

/// 上游 HTTP 客户端：优先 IPv4，避免 Windows 上 AAAA 不可达时卡满 connect_timeout。
fn build_upstream_client_inner(timeout: Option<Duration>) -> Result<Client> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Duration::from_secs(30))
        .tcp_nodelay(true)
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    Ok(builder.build()?)
}

type UpstreamChunkStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

struct ChatHold {
    prefix: Vec<Bytes>,
    rest: Option<UpstreamChunkStream>,
    saw_tool: bool,
}

async fn hold_chat_sse(response: reqwest::Response) -> Result<ChatHold, reqwest::Error> {
    let mut incoming = response.bytes_stream();
    let mut prefix = Vec::new();
    let mut acc = String::new();
    while let Some(item) = incoming.next().await {
        let bytes = item?;
        acc.push_str(&String::from_utf8_lossy(&bytes));
        let saw_tool = crate::upstream_resilience::chat_sse_saw_tool_call(&acc);
        prefix.push(bytes);
        if saw_tool {
            return Ok(ChatHold {
                prefix,
                rest: Some(Box::pin(incoming)),
                saw_tool: true,
            });
        }
    }
    Ok(ChatHold {
        prefix,
        rest: None,
        saw_tool: false,
    })
}

async fn hold_chat_json(response: reqwest::Response) -> Result<ChatHold, reqwest::Error> {
    let text = response.text().await?;
    let saw_tool = !crate::upstream_resilience::assistant_text_omits_tools(&text);
    Ok(ChatHold {
        prefix: vec![Bytes::from(text)],
        rest: None,
        saw_tool,
    })
}

fn replay_upstream(prefix: Vec<Bytes>, rest: Option<UpstreamChunkStream>) -> UpstreamChunkStream {
    Box::pin(stream! {
        for chunk in prefix {
            yield Ok(chunk);
        }
        if let Some(mut rest) = rest {
            while let Some(item) = rest.next().await {
                yield item;
            }
        }
    })
}

fn event_stream_response(
    body: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
) -> Result<Response<Body>> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(http::header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(body))?)
}

fn chat_stream_from_hold(
    held: ChatHold,
    stats: Option<GatewayStatsCollector>,
    token_attr: Option<(String, String)>,
    client_model: &str,
) -> Result<Response<Body>> {
    let replay = replay_upstream(held.prefix, held.rest);
    let mapped = tap_stream_for_token_stats(
        replay.map(|item| item.map_err(std::io::Error::other)),
        stats,
        token_attr,
    );
    let rewritten = rewrite_model_stream(mapped, client_model.to_string());
    event_stream_response(rewritten)
}

fn json_from_hold(held: ChatHold, client_model: &str) -> Result<Response<Body>> {
    let mut raw = Vec::new();
    for chunk in held.prefix {
        raw.extend_from_slice(&chunk);
    }
    let rewritten = crate::upstream_resilience::rewrite_json_model(&raw, client_model);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(rewritten))?)
}

fn responses_stream_from_hold(
    held: ChatHold,
    model: String,
    tool_meta: crate::openai_responses_adapter::ChatToolMeta,
    estimated_input: u64,
    stats: Option<GatewayStatsCollector>,
    token_attr: Option<(String, String)>,
) -> Result<Response<Body>> {
    let replay = replay_upstream(held.prefix, held.rest);
    let mapped = crate::openai_responses_adapter::chat_sse_to_responses_sse_with_meta(
        replay,
        model,
        tool_meta,
        estimated_input,
    );
    let tapped = tap_stream_for_token_stats(mapped, stats, token_attr);
    event_stream_response(tapped)
}

/// 收费模型的总次数（含第一次）。免费通道返回 None，走 3、2、1。
///
/// provider 级 `tool_text_retry_enabled` 仍是主开关（用户显式意图），规则只能调整次数。
fn paid_text_total(provider: &ProviderConfig, effects: &RuleEffects) -> Option<u32> {
    if crate::experimental_free::is_experimental_free(provider) {
        return None;
    }
    if provider.tool_text_retry_enabled {
        let extra = effects
            .tool_retry_extra
            .unwrap_or(provider.tool_text_retry_extra);
        Some(extra.clamp(1, 5).saturating_add(1))
    } else {
        Some(1)
    }
}

/// 克隆并绑定实验免费通道路由；用户自定义源原样返回。
pub(crate) fn prepare_upstream_provider(provider: &ProviderConfig) -> ProviderConfig {
    let mut p = provider.clone();
    if crate::experimental_free::is_experimental_free(&p) {
        crate::experimental_free::apply_route(&mut p);
    }
    p
}

/// 实验通道内部 failover 尝试序列；自定义源仅自身一次。
/// `failover_enabled == false` 时只打当前模型（便于定点排查）。
fn upstream_attempts(provider: &ProviderConfig, failover_enabled: bool) -> Vec<ProviderConfig> {
    let primary = prepare_upstream_provider(provider);
    if !failover_enabled || !crate::experimental_free::is_experimental_free(provider) {
        return vec![primary];
    }
    let mut out = vec![primary.clone()];
    for (base, model) in crate::experimental_free::failover_routes(&primary.model_name) {
        let mut next = primary.clone();
        next.base_url = base.to_string();
        next.model_name = model.to_string();
        out.push(next);
    }
    out
}

async fn into_axum_response(
    response: reqwest::Response,
    stats: Option<GatewayStatsCollector>,
    token_attr: Option<(String, String)>,
    client_model: &str,
) -> Result<Response<Body>> {
    let status = response.status();
    let headers = response.headers().clone();
    let mapped = tap_stream_for_token_stats(
        response.bytes_stream().map(|item| item.map_err(std::io::Error::other)),
        stats,
        token_attr,
    );
    let rewritten = rewrite_model_stream(mapped, client_model.to_string());
    let mut builder = Response::builder().status(status);

    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(name, value);
    }

    Ok(builder.body(Body::from_stream(rewritten))?)
}

/// 从响应字节流中拾取 usage 并记入请求统计（Chat SSE / Responses SSE 均可）。
fn tap_stream_for_token_stats<S>(
    stream: S,
    stats: Option<GatewayStatsCollector>,
    token_attr: Option<(String, String)>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    stream.map(move |item| {
        let bytes = item?;
        if let Some((input, output)) = parse_usage_from_bytes(&bytes) {
            if let Some(collector) = stats.clone() {
                let attr = token_attr.clone();
                tokio::spawn(async move {
                    let (client, provider) = attr
                        .as_ref()
                        .map(|(c, p)| (Some(c.as_str()), Some(p.as_str())))
                        .unwrap_or((None, None));
                    collector
                        .record_tokens_attributed(input, output, client, provider)
                        .await;
                });
            }
        }
        Ok::<Bytes, std::io::Error>(bytes)
    })
}

fn token_attr_from(headers: &HeaderMap, provider: &ProviderConfig) -> (String, String) {
    (
        crate::gateway_stats::client_label_from_headers(headers),
        provider.name.clone(),
    )
}

pub async fn list_openai_models(
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let models_url = provider_catalog::resolve_models_list_url(base_url);
    let response = match client
        .get(&models_url)
        .bearer_auth(api_key)
        .timeout(Duration::from_secs(20))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => bail!(error_hint::classify_request_error(&err)),
    };

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        bail!(error_hint::classify_http_error(status, &text));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|err| anyhow!("解析模型列表失败: {}", err))?;

    let mut models = payload
        .get("data")
        .and_then(|data| data.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if models.is_empty() {
        bail!("接口未返回可用模型，请手动填写 Model Name");
    }

    models.sort();
    Ok(models)
}

fn parse_usage_from_bytes(bytes: &[u8]) -> Option<(u64, u64)> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if let Some(usage) = extract_usage_from_json(&value) {
            return Some(usage);
        }
    }

    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let payload = line
            .trim()
            .strip_prefix("data:")
            .map(|part| part.trim())
            .filter(|part| !part.is_empty() && *part != "[DONE]");
        if let Some(payload) = payload {
            if let Ok(value) = serde_json::from_str::<Value>(payload) {
                if let Some(usage) = extract_usage_from_json(&value) {
                    return Some(usage);
                }
            }
        }
    }

    None
}

fn extract_usage_from_json(value: &Value) -> Option<(u64, u64)> {
    let usage = value.get("usage")?;
    let input = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 {
        None
    } else {
        Some((input, output))
    }
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    let body = json!({
        "error": {
            "message": message,
            "type": "sugt_gateway_error",
            "code": status.as_u16(),
        }
    });
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn anthropic_error(status: StatusCode, message: &str) -> Response<Body> {
    let body = json!({
        "type": "error",
        "error": {
            "type": "api_error",
            "message": message,
        }
    });
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

/// 探测上游是否可用，只保留 `Result` 形态（`GatewayState::test_provider` 现在直接用
/// `probe::probe_llm_connectivity` 拿到完整报告，这里留给只需要成败的调用方）。
pub async fn test_provider_connection(client: &Client, provider: &ProviderConfig) -> Result<()> {
    let report = crate::probe::probe_llm_connectivity(client, provider).await;
    if report.ok {
        Ok(())
    } else {
        bail!(
            report
                .error
                .unwrap_or_else(|| "上游未返回可读的回复内容".to_string())
        )
    }
}

pub fn save_runtime_config(paths: &config::AppPaths, config: &AppConfig) -> Result<()> {
    let mut copy = config.clone();
    config::normalize_config(&mut copy);
    config::save_config(paths, &copy)
}

#[cfg(test)]
mod tests;
