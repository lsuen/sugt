use crate::{
    anthropic_adapter, config, error_hint, gateway_stats::{GatewayStatsCollector, RequestRecord},
    model::{
        AnthropicAccessMode, AppConfig, ProviderConfig, ProviderProtocol, ProviderStatus, ProxyHit,
    },
    provider_catalog,
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
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
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

#[derive(Clone)]
pub struct GatewayState {
    inner: Arc<RwLock<GatewayInner>>,
    client: Client,
    last_hit: Arc<RwLock<Option<ProxyHit>>>,
    stats: GatewayStatsCollector,
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
    if let Ok(cfg) = config::load_config_file(&path) {
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
        let client = build_upstream_client(Duration::from_secs(600))?;

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
        })
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
        self.stats.snapshot().await
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
    ) {
        let hit = ProxyHit {
            provider_id: provider.id.clone(),
            provider_name: provider.name.clone(),
            path: path.to_string(),
            failover: provider_index > 0,
            at: chrono::Utc::now(),
        };
        let client = crate::gateway_stats::client_label_from_headers(headers);
        info!(
            client = %client,
            provider = %provider.name,
            model = %provider.model_name,
            path = %path,
            failover = provider_index > 0,
            latency_ms,
            "inbound request ok"
        );
        *self.last_hit.write().await = Some(hit);
        self.record_request_stats(true, latency_ms, headers, provider, path).await;
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

    pub async fn test_provider(&self, provider_id: &str) -> Result<ProviderStatus> {
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

        let test_result = test_provider_connection(&self.client, &provider).await;
        let status = match &test_result {
            Ok(()) => ProviderStatus::Available,
            Err(err) => {
                warn!(provider = %provider.name, error = %err, "provider test failed");
                ProviderStatus::Unavailable
            }
        };
        let error_message = test_result.err().map(|err| err.to_string());

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

        Ok(status)
    }
}

fn build_router(state: GatewayState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/_sugt/traffic", get(traffic_endpoint))
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

async fn models(State(state): State<GatewayState>) -> impl IntoResponse {
    let config = state.config().await;
    let data: Vec<Value> = config
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| {
            json!({
                "id": provider.public_model_id(),
                "object": "model",
                "created": 0,
                "owned_by": provider.provider,
            })
        })
        .collect();

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
    let started = std::time::Instant::now();
    match proxy_request(state.clone(), headers.clone(), request).await {
        Ok(response) => response,
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
    let started = std::time::Instant::now();
    match proxy_anthropic_request(state.clone(), headers.clone(), request).await {
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
            anthropic_error(StatusCode::BAD_GATEWAY, &err.to_string())
        }
    }
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
    // 用户在控制台选择的接入点协议模式（auto / openai / anthropic）
    let access_mode = config.anthropic_access_mode;
    let providers = ordered_providers(&config);

    if providers.is_empty() {
        bail!("没有可用模型配置");
    }

    // 识别调用来源：Claude 系 TUI/GUI 优先走原生 Anthropic 端点（转换作为 fallback）
    let anthropic_client = crate::anthropic_client_detect::detect_anthropic_client(&headers);

    let mut last_error = None;
    let started = std::time::Instant::now();
    for (index, provider) in providers.iter().enumerate() {
        // 熔断器：跳过冷却期内的故障 provider
        if state.is_circuit_open(&provider.id).await {
            warn!(provider = %provider.name, "provider 处于熔断冷却期，跳过");
            last_error = Some(anyhow!("{} 处于熔断冷却期，已跳过", provider.name));
            continue;
        }

        let native_target = anthropic_native_target(provider);

        // 依据接入点协议模式决定请求路径
        let result = match access_mode {
            // 强制 OpenAI 协议：始终走转换，不尝试原生端点
            AnthropicAccessMode::OpenAi => {
                proxy_anthropic_via_openai(&state, &headers, body_bytes.clone(), provider).await
            }
            // 强制 Anthropic 原生协议：不走转换；无原生端点时给出明确错误并尝试下一个
            AnthropicAccessMode::Anthropic => match native_target {
                Some(target) => {
                    proxy_anthropic_native(&state, &headers, body_bytes.clone(), provider, Some(target))
                        .await
                }
                None => {
                    let err = anyhow!(
                        "{} 未提供 Anthropic 原生端点，无法以 Anthropic 模式访问，请改用 Auto 或 OpenAI 模式",
                        provider.name
                    );
                    warn!(provider = %provider.name, error = %err, "force-native mode skipped provider");
                    last_error = Some(err);
                    continue;
                }
            },
            // 自动模式：识别到 Claude 客户端且服务商有原生端点则优先原生，失败降级转换
            AnthropicAccessMode::Auto => {
                let prefers_native = provider.protocol == ProviderProtocol::Anthropic
                    || (anthropic_client.is_some()
                        && native_target.is_some()
                        && !state.is_native_unsupported(&provider.id).await);

                if prefers_native {
                    match proxy_anthropic_native(
                        &state,
                        &headers,
                        body_bytes.clone(),
                        provider,
                        native_target,
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
                            proxy_anthropic_via_openai(
                                &state,
                                &headers,
                                body_bytes.clone(),
                                provider,
                            )
                            .await
                        }
                        other => other,
                    }
                } else {
                    proxy_anthropic_via_openai(&state, &headers, body_bytes.clone(), provider)
                        .await
                }
            }
        };

        match result {
            Ok(response) if response.status().is_success() || response.status().as_u16() < 500 => {
                state.reset_circuit(&provider.id).await;
                state
                    .record_proxy_hit(
                        provider,
                        "v1/messages",
                        index,
                        started.elapsed().as_millis() as u64,
                        &headers,
                    )
                    .await;
                return Ok(response);
            }
            Ok(response) => {
                let status = response.status();
                warn!(provider = %provider.name, %status, "provider returned retryable anthropic error");
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 返回 {}", provider.name, status));
            }
            Err(err) => {
                warn!(provider = %provider.name, error = %err, "provider anthropic request error");
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 请求失败: {}", provider.name, err));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("所有模型配置均不可用")))
}

async fn proxy_anthropic_native(
    state: &GatewayState,
    headers: &HeaderMap,
    body: Bytes,
    provider: &ProviderConfig,
    native_target: Option<String>,
) -> Result<Response<Body>> {
    // 原生端点优先用解析结果（可能来自服务商目录的 anthropic_base_url），兜底用 provider.base_url
    let target = build_target_url(
        native_target.as_deref().unwrap_or(&provider.base_url),
        "v1/messages",
        "",
    );
    let payload = rewrite_model(body, &provider.model_name);
    let mut builder = state.client.post(&target);
    builder = copy_forward_headers(builder, headers, &provider.api_key, AuthMode::Anthropic)?;

    info!(
        provider = %provider.name,
        model = %provider.model_name,
        path = "v1/messages",
        mode = "anthropic_native",
        "proxying anthropic messages request"
    );

    let response = builder.body(payload).send().await?;
    into_axum_response(response, Some(state.stats.clone())).await
}

async fn proxy_anthropic_via_openai(
    state: &GatewayState,
    headers: &HeaderMap,
    body: Bytes,
    provider: &ProviderConfig,
) -> Result<Response<Body>> {
    let target = build_target_url(&provider.base_url, "v1/chat/completions", "");
    let payload = anthropic_adapter::anthropic_to_openai(body, &provider.model_name)?;
    let mut builder = state.client.post(&target);
    builder = copy_forward_headers(builder, headers, &provider.api_key, AuthMode::OpenAi)?;
    builder = maybe_zen_headers(builder, provider);

    info!(
        provider = %provider.name,
        model = %provider.model_name,
        path = "v1/messages",
        mode = "openai_adapter",
        "proxying anthropic messages request"
    );

    let response = builder.body(payload).send().await?;
    anthropic_adapter::openai_to_anthropic_response(
        response,
        &provider.model_name,
        Some(state.stats.clone()),
    )
    .await
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

    if path == "v1/responses" || path.starts_with("v1/responses/") {
        return proxy_responses_via_chat(state, headers, method, body_bytes, &query).await;
    }

    let providers = ordered_providers(&state.config().await);

    if providers.is_empty() {
        bail!("没有可用模型配置");
    }

    let mut last_error = None;
    let started = std::time::Instant::now();
    for (index, provider) in providers.iter().enumerate() {
        // 熔断器：跳过冷却期内的故障 provider
        if state.is_circuit_open(&provider.id).await {
            warn!(
                provider = %provider.name,
                "provider 处于熔断冷却期，跳过"
            );
            last_error = Some(anyhow!("{} 处于熔断冷却期，已跳过", provider.name));
            continue;
        }

        let target = build_target_url(&provider.base_url, &path, &query);
        let payload = rewrite_model(body_bytes.clone(), &provider.model_name);
        let mut builder = state.client.request(method.clone(), &target);
        builder = copy_forward_headers(builder, &headers, &provider.api_key, AuthMode::OpenAi)?;
        builder = maybe_zen_headers(builder, provider);

        info!(
            provider = %provider.name,
            model = %provider.model_name,
            path = %path,
            "proxying request"
        );
        match builder.body(payload).send().await {
            Ok(response) if response.status().is_success() || response.status().as_u16() < 500 => {
                // 成功时清除熔断状态
                state.reset_circuit(&provider.id).await;
                state
                    .record_proxy_hit(
                        provider,
                        &path,
                        index,
                        started.elapsed().as_millis() as u64,
                        &headers,
                    )
                    .await;
                return into_axum_response(response, Some(state.stats.clone())).await;
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                warn!(provider = %provider.name, %status, body = %text, "provider returned retryable error");
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 返回 {}", provider.name, status));
            }
            Err(err) => {
                warn!(provider = %provider.name, error = %err, "provider request error");
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 请求失败: {}", provider.name, err));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("所有模型配置均不可用")))
}

async fn proxy_responses_via_chat(
    state: GatewayState,
    headers: HeaderMap,
    method: http::Method,
    body_bytes: Bytes,
    query: &str,
) -> Result<Response<Body>> {
    let providers = ordered_providers(&state.config().await);
    if providers.is_empty() {
        bail!("没有可用模型配置");
    }

    let stream = serde_json::from_slice::<Value>(&body_bytes)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false);

    let mut last_error = None;
    let started = std::time::Instant::now();
    for (index, provider) in providers.iter().enumerate() {
        // 熔断器：跳过冷却期内的故障 provider
        if state.is_circuit_open(&provider.id).await {
            warn!(provider = %provider.name, "provider 处于熔断冷却期，跳过");
            last_error = Some(anyhow!("{} 处于熔断冷却期，已跳过", provider.name));
            continue;
        }

        let try_native = crate::openai_responses_adapter::should_try_native_responses(
            &provider.base_url,
            &provider.model_name,
        );
        info!(
            provider = %provider.name,
            base_url = %provider.base_url,
            model = %provider.model_name,
            try_native_responses = try_native,
            "codex responses routing"
        );

        if try_native {
            let native_base =
                crate::openai_responses_adapter::native_responses_base_url(
                    &provider.base_url,
                    &provider.model_name,
                );
            let upstream_body = crate::openai_responses_adapter::rewrite_responses_upstream_model(
                &body_bytes,
                &provider.model_name,
            )?;
            let target = build_target_url(&native_base, "v1/responses", query);
            let mut builder = state.client.request(method.clone(), &target);
            builder =
                copy_forward_headers(builder, &headers, &provider.api_key, AuthMode::OpenAi)?;
            builder = maybe_zen_headers(builder, provider);

            info!(
                provider = %provider.name,
                model = %provider.model_name,
                base_url = %provider.base_url,
                native_base = %native_base,
                target_url = %target,
                path = "v1/responses",
                stream,
                mode = "native_responses_passthrough",
                "proxying codex responses to upstream Responses API"
            );

            let native_result = builder.body(upstream_body).send().await;
            match native_result {
                Ok(response)
                    if response.status().is_success() || response.status().as_u16() < 500 =>
                {
                    state.reset_circuit(&provider.id).await;
                    state
                        .record_proxy_hit(
                            provider,
                            "v1/responses",
                            index,
                            started.elapsed().as_millis() as u64,
                            &headers,
                        )
                        .await;
                    return into_axum_response(response, Some(state.stats.clone())).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    warn!(
                        provider = %provider.name,
                        %status,
                        body = %text,
                        "native responses upstream error, will try chat/completions fallback"
                    );
                }
                Err(err) => {
                    warn!(
                        provider = %provider.name,
                        error = %err,
                        "native responses request failed, will try chat/completions fallback"
                    );
                }
            }
        }

        let chat_body =
            crate::openai_responses_adapter::responses_to_chat(&body_bytes, &provider.model_name)?;
        let target = build_target_url(&provider.base_url, "v1/chat/completions", query);
        let mut builder = state.client.request(method.clone(), &target);
        builder = copy_forward_headers(builder, &headers, &provider.api_key, AuthMode::OpenAi)?;
        builder = maybe_zen_headers(builder, provider);

        info!(
            provider = %provider.name,
            model = %provider.model_name,
            base_url = %provider.base_url,
            target_url = %target,
            path = "v1/responses",
            stream,
            auto_adapt = provider.auto_adapt_base_url,
            mode = "chat_completions_adapter",
            "proxying codex responses via chat completions"
        );

        if stream {
            let response = builder.body(chat_body).send().await?;
            if response.status().is_success() || response.status().as_u16() < 500 {
                state.reset_circuit(&provider.id).await;
                state
                    .record_proxy_hit(
                        provider,
                        "v1/responses",
                        index,
                        started.elapsed().as_millis() as u64,
                        &headers,
                    )
                    .await;
                let status = response.status();
                let model = provider.public_model_id();
                let mapped = crate::openai_responses_adapter::chat_sse_to_responses_sse(
                    response.bytes_stream(),
                    model,
                );
                return Ok(Response::builder()
                    .status(status)
                    .header(http::header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
                    .header(http::header::CACHE_CONTROL, "no-cache")
                    .body(Body::from_stream(mapped))?);
            }
            state.trip_circuit(&provider.id).await;
            last_error = Some(anyhow!("{} 返回 {}", provider.name, response.status()));
            continue;
        }

        match builder.body(chat_body).send().await {
            Ok(response) if response.status().is_success() || response.status().as_u16() < 500 => {
                state.reset_circuit(&provider.id).await;
                let status = response.status();
                let body_text = response.text().await.unwrap_or_default();
                if status.is_success()
                    && crate::openai_responses_adapter::is_openai_compat_error_body(&body_text)
                {
                    warn!(
                        provider = %provider.name,
                        body = %body_text,
                        "upstream returned error JSON for codex responses"
                    );
                    last_error =
                        Some(anyhow!("{} 上游错误: {}", provider.name, body_text));
                    continue;
                }
                state
                    .record_proxy_hit(
                        provider,
                        "v1/responses",
                        index,
                        started.elapsed().as_millis() as u64,
                        &headers,
                    )
                    .await;
                let bytes = if status.is_success() {
                    crate::openai_responses_adapter::chat_json_to_responses_body(
                        &body_text,
                        &provider.public_model_id(),
                    )?
                } else {
                    Bytes::from(body_text)
                };
                return Ok(Response::builder()
                    .status(status)
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(bytes))?);
            }
            Ok(response) => {
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 返回 {}", provider.name, response.status()));
            }
            Err(err) => {
                state.trip_circuit(&provider.id).await;
                last_error = Some(anyhow!("{} 请求失败: {}", provider.name, err));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("Codex Responses 代理失败")))
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

fn build_target_url(base_url: &str, path: &str, query: &str) -> String {
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

fn rewrite_model(body: Bytes, model_name: &str) -> Bytes {
    if body.is_empty() {
        return body;
    }

    match serde_json::from_slice::<Value>(&body) {
        Ok(mut value) => {
            if let Some(object) = value.as_object_mut() {
                object.insert("model".to_string(), Value::String(model_name.to_string()));
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
) -> Result<reqwest::RequestBuilder> {
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
        ) {
            continue;
        }
        builder = builder.header(name, value);
    }

    match auth_mode {
        AuthMode::OpenAi => {
            let auth = HeaderValue::from_str(&format!("Bearer {}", api_key))
                .context("API Key 无法写入请求头")?;
            builder = builder.header(http::header::AUTHORIZATION, auth);
        }
        AuthMode::Anthropic => {
            builder = builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
    }

    Ok(builder)
}

/// 上游 HTTP 客户端：优先 IPv4，避免 Windows 上 AAAA 不可达时卡满 connect_timeout。
pub fn build_upstream_client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(8))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(4)
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .build()?)
}

fn apply_zen_client_headers(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{:x}", d.as_millis()))
        .unwrap_or_else(|_| "0".into());
    let rnd = uuid::Uuid::new_v4().simple().to_string();
    let request_id = format!("msg_{ts}{}", &rnd[..12]);
    let session_id = format!("ses_{ts}{}", &rnd[12..24.min(rnd.len())]);
    builder
        .header(
            http::header::USER_AGENT,
            "opencode/1.15.0 ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.13",
        )
        .header("x-opencode-client", "cli")
        .header("x-opencode-project", "global")
        .header("x-opencode-request", request_id)
        .header("x-opencode-session", session_id)
}

fn maybe_zen_headers(
    builder: reqwest::RequestBuilder,
    provider: &ProviderConfig,
) -> reqwest::RequestBuilder {
    if crate::experimental_zen::is_zen_upstream(provider) {
        apply_zen_client_headers(builder)
    } else {
        builder
    }
}

async fn into_axum_response(
    response: reqwest::Response,
    stats: Option<GatewayStatsCollector>,
) -> Result<Response<Body>> {
    let status = response.status();
    let headers = response.headers().clone();
    let stream = response.bytes_stream();
    let mapped = stream.map(move |item| {
        let bytes = item.map_err(std::io::Error::other)?;
        if let Some((input, output)) = parse_usage_from_bytes(&bytes) {
            if let Some(collector) = stats.clone() {
                tokio::spawn(async move {
                    collector.record_tokens(input, output).await;
                });
            }
        }
        Ok::<Bytes, std::io::Error>(bytes)
    });
    let mut builder = Response::builder().status(status);

    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(name, value);
    }

    Ok(builder.body(Body::from_stream(mapped))?)
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

pub async fn test_provider_connection(client: &Client, provider: &ProviderConfig) -> Result<()> {
    match provider.protocol {
        ProviderProtocol::Anthropic => test_anthropic_connection(client, provider).await,
        ProviderProtocol::OpenAi => test_openai_connection(client, provider).await,
    }
}

async fn test_openai_connection(client: &Client, provider: &ProviderConfig) -> Result<()> {
    let chat_url = build_target_url(&provider.base_url, "v1/chat/completions", "");
    let zen = crate::experimental_zen::is_zen_upstream(provider);
    tracing::info!(
        provider = %provider.name,
        base_url = %provider.base_url,
        chat_url = %chat_url,
        auto_adapt = provider.auto_adapt_base_url,
        zen,
        "test_openai_connection"
    );

    // Zen：流式探测，收到首包即成功（对齐 OpenCode 首字体验）；其它源仍用短非流式。
    let body = if zen {
        json!({
            "model": provider.model_name,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 8,
            "stream": true
        })
    } else {
        json!({
            "model": provider.model_name,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "stream": false
        })
    };
    let timeout = if zen {
        Duration::from_secs(25)
    } else {
        Duration::from_secs(20)
    };
    let mut req = client
        .post(&chat_url)
        .bearer_auth(&provider.api_key)
        .json(&body)
        .timeout(timeout);
    if zen {
        req = apply_zen_client_headers(req);
    }
    let response = match req.send().await {
        Ok(response) => response,
        Err(err) => bail!(error_hint::classify_request_error(&err)),
    };

    if response.status().is_success() {
        if zen {
            return wait_zen_stream_first_byte(response).await;
        }
        return Ok(());
    }

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    warn!(provider = %provider.name, %status, body = %text, "chat completion test failed");

    let models_url = build_target_url(&provider.base_url, "v1/models", "");
    let mut models_req = client
        .get(&models_url)
        .bearer_auth(&provider.api_key)
        .timeout(if zen {
            Duration::from_secs(12)
        } else {
            Duration::from_secs(12)
        });
    if zen {
        models_req = apply_zen_client_headers(models_req);
    }
    match models_req.send().await {
        Ok(resp) if resp.status().is_success() => {
            bail!(
                "{}；models 接口可访问",
                error_hint::classify_http_error(status, &text)
            )
        }
        Ok(resp) => {
            let models_status = resp.status();
            bail!(
                "{}；models 接口 {}",
                error_hint::classify_http_error(status, &text),
                error_hint::classify_http_error(models_status, "")
            )
        }
        Err(err) => bail!(
            "{}；models 接口 {}",
            error_hint::classify_http_error(status, &text),
            error_hint::classify_request_error(&err)
        ),
    }
}

/// 等 SSE 首包（含 data: / error），避免等完整生成。
async fn wait_zen_stream_first_byte(response: reqwest::Response) -> Result<()> {
    let mut stream = response.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut buffered = String::new();
    loop {
        let next = tokio::time::timeout_at(deadline, stream.next()).await;
        match next {
            Ok(Some(Ok(chunk))) => {
                buffered.push_str(&String::from_utf8_lossy(&chunk));
                let lower = buffered.to_ascii_lowercase();
                if lower.contains("\"error\"") && lower.contains("message") {
                    bail!("Zen 返回错误：{}", buffered.chars().take(240).collect::<String>());
                }
                if buffered.contains("data:")
                    || buffered.contains("delta")
                    || buffered.contains("choices")
                {
                    return Ok(());
                }
                if buffered.len() > 4096 {
                    return Ok(());
                }
            }
            Ok(Some(Err(err))) => bail!(error_hint::classify_request_error(&err)),
            Ok(None) => {
                if buffered.trim().is_empty() {
                    bail!("Zen 流式响应为空");
                }
                return Ok(());
            }
            Err(_) => bail!("连接超时：请检查网络、代理或 Base URL 是否可达"),
        }
    }
}

async fn test_anthropic_connection(client: &Client, provider: &ProviderConfig) -> Result<()> {
    let messages_url = build_target_url(&provider.base_url, "v1/messages", "");
    let response = match client
        .post(&messages_url)
        .header("x-api-key", &provider.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": provider.model_name,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => bail!(error_hint::classify_request_error(&err)),
    };

    if response.status().is_success() {
        return Ok(());
    }

    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status == StatusCode::NOT_FOUND && test_openai_connection(client, provider).await.is_ok() {
        bail!(
            "上游无 Anthropic /v1/messages（404），但 OpenAI chat 可用。请将协议改为 OpenAI Compatible"
        );
    }

    bail!(error_hint::classify_http_error(status, &text))
}

pub fn save_runtime_config(paths: &config::AppPaths, config: &AppConfig) -> Result<()> {
    let mut copy = config.clone();
    config::normalize_config(&mut copy);
    config::save_config(paths, &copy)
}

#[cfg(test)]
mod tests {
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
        assert_eq!(target, "https://ark.cn-beijing.volces.com/api/v3/chat/completions");
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
    }
}
