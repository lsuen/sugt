//! WebUI HTTP 路由层。
//!
//! 协议（对前端 transport 的契约）：
//! - `POST /api/login`  body `{"key": "..."}` → `{"token":"...","expires_in":43200}`
//! - `GET  /api/health` → `{"ok":true,"service":"SUGT WebUI"}`（无需鉴权，供探针用）
//! - `POST /api/invoke` header `Authorization: Bearer <token>`
//!     body `{"cmd":"<tauri命令名>","args":{...}}`
//!     → `{"ok":true,"data":<该命令返回值>}` 或 `{"ok":false,"error":"..."}`（HTTP 200，错误在 body）
//! - 其它路径 → 内嵌 SPA 静态资源
//!
//! 命令名与 Tauri `invoke` 完全一致，前端零改动命令调用点。

use super::{assets, auth};
use crate::commands::AppRuntime;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct WebUiState {
    pub runtime: AppRuntime,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct InvokeRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Serialize)]
pub struct InvokeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn build_router(runtime: AppRuntime) -> Router {
    let state = WebUiState { runtime };
    Router::new()
        .route("/api/health", get(health))
        .route("/api/login", post(login))
        .route("/api/invoke", post(invoke))
        .fallback(assets::serve_static)
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true, "service": "SUGT WebUI"}))
}

async fn login(
    State(state): State<WebUiState>,
    _headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Response {
    // 把登录页提交的 key 按网关既有格式塞进伪 header，复用网关鉴权语义。
    let mut probe = HeaderMap::new();
    if let Ok(value) = header::HeaderValue::from_str(&req.key) {
        probe.insert("x-api-key", value);
    }
    let config = state.runtime.config.read().await.clone();
    if !auth::validate_webui_key(&probe, &config, &state.runtime).await {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid key"}))).into_response();
    }
    let (token, expires_in) = auth::issue_token(&config);
    Json(json!({"token": token, "expires_in": expires_in})).into_response()
}

async fn invoke(
    State(state): State<WebUiState>,
    headers: HeaderMap,
    Json(req): Json<InvokeRequest>,
) -> Response {
    let config = state.runtime.config.read().await.clone();
    let authorized = auth::bearer_token(&headers)
        .map(|token| auth::verify_token(token, &config))
        .unwrap_or(false);
    if !authorized {
        return (StatusCode::UNAUTHORIZED, Json(json!({"ok": false, "error": "unauthorized"}))).into_response();
    }

    match super::service::dispatch(&state.runtime, &req.cmd, &req.args).await {
        Ok(data) => Json(InvokeResponse {
            ok: true,
            data: Some(data),
            error: None,
        })
        .into_response(),
        Err(err) => Json(InvokeResponse {
            ok: false,
            data: None,
            error: Some(err),
        })
        .into_response(),
    }
}
