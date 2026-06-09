use crate::{config, model::{AppConfig, ProviderConfig, ProviderStatus}};
use anyhow::{anyhow, bail, Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{any, get},
    Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::{oneshot, RwLock}};
use tower_http::{cors::{Any, CorsLayer}, trace::TraceLayer};
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct GatewayState {
    inner: Arc<RwLock<GatewayInner>>,
    client: Client,
}

struct GatewayInner {
    config: AppConfig,
    shutdown: Option<oneshot::Sender<()>>,
    running: bool,
}

impl GatewayState {
    pub fn new(config: AppConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            inner: Arc::new(RwLock::new(GatewayInner { config, shutdown: None, running: false })),
            client,
        })
    }

    pub async fn config(&self) -> AppConfig {
        self.inner.read().await.config.clone()
    }

    pub async fn replace_config(&self, config: AppConfig) {
        self.inner.write().await.config = config;
    }

    pub async fn is_running(&self) -> bool {
        self.inner.read().await.running
    }

    pub async fn start(&self) -> Result<String> {
        let (host, port) = {
            let mut inner = self.inner.write().await;
            if inner.running {
                return Ok(format!("http://{}:{}", inner.config.host, inner.config.port));
            }
            (inner.config.host.clone(), inner.config.port)
        };

        let addr: SocketAddr = format!("{}:{}", host, port).parse().context("监听地址无效")?;
        let listener = TcpListener::bind(addr).await.with_context(|| format!("无法监听 {}", addr))?;
        let local_addr = listener.local_addr()?;
        let (tx, rx) = oneshot::channel();
        let app = build_router(self.clone());

        {
            let mut inner = self.inner.write().await;
            inner.shutdown = Some(tx);
            inner.running = true;
        }

        let state = self.clone();
        tokio::spawn(async move {
            info!(%local_addr, "SUGT gateway started");
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

        Ok(format!("http://{}", local_addr))
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
        let inner = self.inner.read().await;
        select_provider(&inner.config).cloned()
    }

    pub async fn test_provider(&self, provider_id: &str) -> Result<ProviderStatus> {
        let provider = {
            let inner = self.inner.read().await;
            inner.config.providers.iter().find(|provider| provider.id == provider_id).cloned()
        }
        .ok_or_else(|| anyhow!("模型配置不存在"))?;

        let status = match test_provider_connection(&self.client, &provider).await {
            Ok(()) => ProviderStatus::Available,
            Err(err) => {
                warn!(provider = %provider.name, error = %err, "provider test failed");
                ProviderStatus::Unavailable
            }
        };

        let mut inner = self.inner.write().await;
        if let Some(item) = inner.config.providers.iter_mut().find(|item| item.id == provider_id) {
            item.status = status.clone();
            item.last_checked_at = Some(chrono::Utc::now());
            item.last_error = if status == ProviderStatus::Unavailable { Some("连接测试失败".to_string()) } else { None };
        }

        Ok(status)
    }
}

fn build_router(state: GatewayState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/*path", any(proxy_openai))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
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

async fn models(State(state): State<GatewayState>) -> impl IntoResponse {
    let config = state.config().await;
    let data: Vec<Value> = config.providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| json!({
            "id": provider.model_name,
            "object": "model",
            "created": 0,
            "owned_by": provider.provider,
        }))
        .collect();

    Json(json!({ "object": "list", "data": data }))
}

async fn proxy_openai(State(state): State<GatewayState>, headers: HeaderMap, request: Request<Body>) -> Response<Body> {
    match proxy_request(state, headers, request).await {
        Ok(response) => response,
        Err(err) => {
            error!(error = %err, "proxy request failed");
            json_error(StatusCode::BAD_GATEWAY, &err.to_string())
        }
    }
}

async fn proxy_request(state: GatewayState, headers: HeaderMap, request: Request<Body>) -> Result<Response<Body>> {
    let path = request.uri().path().trim_start_matches('/');
    let query = request.uri().query().map(|q| format!("?{}", q)).unwrap_or_default();
    let method = request.method().clone();
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await?;
    let providers = ordered_providers(&state.config().await);

    if providers.is_empty() {
        bail!("没有可用模型配置")
    }

    let mut last_error = None;
    for provider in providers {
        let target = format!("{}/{}{}", provider.base_url, path, query);
        let payload = rewrite_model(body_bytes.clone(), &provider.model_name);
        let mut builder = state.client.request(method.clone(), &target);
        builder = copy_headers(builder, &headers, &provider.api_key)?;

        info!(provider = %provider.name, model = %provider.model_name, path = %path, "proxying request");
        match builder.body(payload).send().await {
            Ok(response) if response.status().is_success() || response.status().as_u16() < 500 => {
                return into_axum_response(response).await;
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                warn!(provider = %provider.name, %status, body = %text, "provider returned retryable error");
                last_error = Some(anyhow!("{} 返回 {}", provider.name, status));
            }
            Err(err) => {
                warn!(provider = %provider.name, error = %err, "provider request error");
                last_error = Some(anyhow!("{} 请求失败: {}", provider.name, err));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("所有模型配置均不可用")))
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
        if let Some(provider) = config.providers.iter().find(|provider| provider.enabled && provider.id == *active_id) {
            return Some(provider);
        }
    }
    config.providers.iter().find(|provider| provider.enabled)
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

fn copy_headers(mut builder: reqwest::RequestBuilder, headers: &HeaderMap, api_key: &str) -> Result<reqwest::RequestBuilder> {
    for (name, value) in headers {
        let key = name.as_str().to_ascii_lowercase();
        if matches!(key.as_str(), "host" | "content-length" | "authorization") {
            continue;
        }
        builder = builder.header(name, value);
    }
    let auth = HeaderValue::from_str(&format!("Bearer {}", api_key)).context("API Key 无法写入请求头")?;
    builder = builder.header(http::header::AUTHORIZATION, auth);
    Ok(builder)
}

async fn into_axum_response(response: reqwest::Response) -> Result<Response<Body>> {
    let status = response.status();
    let headers = response.headers().clone();
    let stream = response.bytes_stream().map(|item| item.map_err(std::io::Error::other));
    let mut builder = Response::builder().status(status);

    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(name, value);
    }

    Ok(builder.body(Body::from_stream(stream))?)
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

pub async fn test_provider_connection(client: &Client, provider: &ProviderConfig) -> Result<()> {
    let models_url = format!("{}/models", provider.base_url);
    let response = client
        .get(models_url)
        .bearer_auth(&provider.api_key)
        .timeout(Duration::from_secs(12))
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => return Ok(()),
        Ok(resp) => warn!(provider = %provider.name, status = %resp.status(), "models endpoint test failed; trying chat completion"),
        Err(err) => warn!(provider = %provider.name, error = %err, "models endpoint unavailable; trying chat completion"),
    }

    let chat_url = format!("{}/chat/completions", provider.base_url);
    let response = client
        .post(chat_url)
        .bearer_auth(&provider.api_key)
        .json(&json!({
            "model": provider.model_name,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "stream": false
        }))
        .timeout(Duration::from_secs(20))
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        bail!("连接测试失败，HTTP {}", response.status())
    }
}

pub fn save_runtime_config(paths: &config::AppPaths, config: &AppConfig) -> Result<()> {
    let mut copy = config.clone();
    config::normalize_config(&mut copy);
    config::save_config(paths, &copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_json_model() {
        let body = Bytes::from_static(br#"{"model":"old","messages":[]}"#);
        let rewritten = rewrite_model(body, "new-model");
        let value: Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(value["model"], "new-model");
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
