//! 上游失败时的同一次请求重试、换路由，以及预算用尽后的会话保持。
//!
//! Agent 只看到自己请求里的模型名（没有则是 `sutai`）。正文还没转发之前可以重试；
//! 一旦开始吐字就不再重试。预算用尽时回一条正常的助手消息，而不是 `response.failed`，
//! 这样客户端会话还在，只是这一轮工具调用停下来。

use axum::{
    body::Body,
    http::{header, Response, StatusCode},
};
use serde_json::{json, Value};
use std::time::Duration;

pub const MAX_PASSES: u32 = 8;
pub const DEFAULT_BUDGET: Duration = Duration::from_secs(8 * 60);
pub const DEFAULT_SINGLE_CAP: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Responses,
    Chat,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Quota,
    FreeExhausted,
    ContextTooLong,
    Auth,
    BadRequest,
    UpstreamDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryReason {
    Quota,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Retryable {
        retry_after: Duration,
        reason: RetryReason,
    },
    Switch,
    Terminal(NoticeKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassFinish {
    Sleep(Duration),
    Notice(NoticeKind),
}

#[derive(Debug, Clone)]
pub struct RetryBudget {
    request_budget: Duration,
    single_cap: Duration,
    spent: Duration,
}

impl RetryBudget {
    pub fn from_limits(budget: Duration, single_cap: Duration) -> Self {
        Self {
            request_budget: budget,
            single_cap,
            spent: Duration::ZERO,
        }
    }

    pub fn from_config(budget_ms: Option<u64>, single_ms: Option<u64>) -> Self {
        Self::from_limits(
            budget_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_BUDGET),
            single_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_SINGLE_CAP),
        )
    }

    /// 扣掉这一次等待。预算为 0 或已经用尽时返回 None，调用方应结束并回提示。
    pub fn admit(&mut self, want: Duration) -> Option<Duration> {
        if self.request_budget.is_zero() || want.is_zero() {
            return None;
        }
        let want = want.min(self.single_cap);
        let left = self.request_budget.saturating_sub(self.spent);
        if left.is_zero() {
            return None;
        }
        let wait = want.min(left);
        self.spent += wait;
        Some(wait)
    }
}

#[derive(Debug, Clone)]
pub struct FailureMemory {
    is_free: bool,
    sticky_quota: bool,
    sticky_down: bool,
    sticky_switch: bool,
    round_retryable: bool,
    round_after: Option<Duration>,
}

impl FailureMemory {
    pub fn new() -> Self {
        Self {
            is_free: false,
            sticky_quota: false,
            sticky_down: false,
            sticky_switch: false,
            round_retryable: false,
            round_after: None,
        }
    }

    pub fn set_free(&mut self, free: bool) {
        if free {
            self.is_free = true;
        }
    }

    pub fn begin_pass(&mut self) {
        self.round_retryable = false;
        self.round_after = None;
    }

    /// Terminal 交给调用方立刻返回。其余记下来，等这一轮路由走完再决定等不等。
    pub fn observe(&mut self, class: FailureClass) -> Option<NoticeKind> {
        match class {
            FailureClass::Terminal(kind) => Some(kind),
            FailureClass::Switch => {
                self.sticky_switch = true;
                None
            }
            FailureClass::Retryable {
                retry_after,
                reason,
            } => {
                self.round_retryable = true;
                self.round_after = Some(match self.round_after {
                    Some(have) => have.max(retry_after),
                    None => retry_after,
                });
                match reason {
                    RetryReason::Quota => self.sticky_quota = true,
                    RetryReason::Unavailable => self.sticky_down = true,
                }
                None
            }
        }
    }

    pub fn finish(&mut self, budget: &mut RetryBudget, pass: u32) -> PassFinish {
        if self.round_retryable {
            let hinted = self.round_after.unwrap_or(Duration::from_secs(2));
            let want = backoff(pass, hinted, budget.single_cap);
            if let Some(wait) = budget.admit(want) {
                return PassFinish::Sleep(wait);
            }
        }
        PassFinish::Notice(self.notice_kind())
    }

    pub fn notice_kind(&self) -> NoticeKind {
        if self.is_free && self.sticky_quota {
            NoticeKind::FreeExhausted
        } else if self.sticky_quota {
            NoticeKind::Quota
        } else if self.sticky_down {
            NoticeKind::UpstreamDown
        } else if self.is_free && self.sticky_switch {
            NoticeKind::FreeExhausted
        } else if self.sticky_switch {
            NoticeKind::BadRequest
        } else {
            NoticeKind::UpstreamDown
        }
    }
}

impl Default for FailureMemory {
    fn default() -> Self {
        Self::new()
    }
}

pub fn route_circuit_key(provider_id: &str, base_url: &str, model: &str, is_free: bool) -> String {
    if is_free {
        format!("{provider_id}|{base_url}|{model}")
    } else {
        provider_id.to_string()
    }
}

pub fn classify_network() -> FailureClass {
    FailureClass::Retryable {
        retry_after: Duration::from_secs(2),
        reason: RetryReason::Unavailable,
    }
}

pub fn classify_http(
    status: u16,
    body: &str,
    retry_after_header: Option<&str>,
    own_key: bool,
    is_free: bool,
) -> FailureClass {
    let lower = body.to_lowercase();
    if is_context_length(&lower) {
        return FailureClass::Terminal(NoticeKind::ContextTooLong);
    }
    match status {
        429 => FailureClass::Retryable {
            retry_after: retry_after_hint(retry_after_header, body, Duration::from_secs(5)),
            reason: RetryReason::Quota,
        },
        408 | 425 | 500 | 502 | 503 | 504 => FailureClass::Retryable {
            retry_after: retry_after_hint(retry_after_header, body, Duration::from_secs(2)),
            reason: RetryReason::Unavailable,
        },
        401 | 403 => {
            if own_key && !is_free {
                FailureClass::Terminal(NoticeKind::Auth)
            } else {
                FailureClass::Switch
            }
        }
        404 => {
            if is_free {
                FailureClass::Switch
            } else {
                FailureClass::Terminal(NoticeKind::BadRequest)
            }
        }
        400 | 413 | 422 => {
            if is_free && model_rejected(&lower) {
                FailureClass::Switch
            } else {
                FailureClass::Terminal(NoticeKind::BadRequest)
            }
        }
        code if (500..600).contains(&code) => FailureClass::Retryable {
            retry_after: Duration::from_secs(2),
            reason: RetryReason::Unavailable,
        },
        code if code >= 400 => {
            if is_free {
                FailureClass::Switch
            } else {
                FailureClass::Terminal(NoticeKind::BadRequest)
            }
        }
        _ => FailureClass::Terminal(NoticeKind::UpstreamDown),
    }
}

pub fn merge_failure(left: FailureClass, right: FailureClass) -> FailureClass {
    match (left, right) {
        (FailureClass::Terminal(NoticeKind::ContextTooLong), _)
        | (_, FailureClass::Terminal(NoticeKind::ContextTooLong)) => {
            FailureClass::Terminal(NoticeKind::ContextTooLong)
        }
        (
            FailureClass::Retryable {
                retry_after: a,
                reason: ra,
            },
            FailureClass::Retryable {
                retry_after: b,
                reason: rb,
            },
        ) => FailureClass::Retryable {
            retry_after: a.max(b),
            reason: if ra == RetryReason::Quota || rb == RetryReason::Quota {
                RetryReason::Quota
            } else {
                RetryReason::Unavailable
            },
        },
        (retry @ FailureClass::Retryable { .. }, _)
        | (_, retry @ FailureClass::Retryable { .. }) => retry,
        (FailureClass::Switch, _) | (_, FailureClass::Switch) => FailureClass::Switch,
        (other, _) => other,
    }
}

pub fn notice_text(kind: NoticeKind) -> &'static str {
    match kind {
        NoticeKind::Quota => {
            "小su提示：模型额度不足，已自动等待并重试，仍失败。请到 SUGT 切换模型，或等额度恢复后再发一句继续。请把这段话告诉用户，不要再调用工具。"
        }
        NoticeKind::FreeExhausted => {
            "小su提示：免费路由已自动换过并重试，暂时都不可用。可以在 SUGT 里改选一条，或稍后再试。请把这段话告诉用户，不要再调用工具。"
        }
        NoticeKind::ContextTooLong => {
            "小su提示：这次请求的上下文过长，上游拒绝了。请缩短对话或新开一轮后再继续。请把这段话告诉用户，不要再调用工具。"
        }
        NoticeKind::Auth => {
            "小su提示：上游密钥无效或没有权限。请到 SUGT 检查这条模型的 Key。请把这段话告诉用户，不要再调用工具。"
        }
        NoticeKind::BadRequest => {
            "小su提示：上游拒绝了这次请求。请看 SUGT 日志里的状态码和原因，不要原样重试。请把这段话告诉用户，不要再调用工具。"
        }
        NoticeKind::UpstreamDown => {
            "小su提示：上游暂时不可用，已自动重试并切换，仍失败。请稍后再发一句继续，或到 SUGT 切换模型。请把这段话告诉用户，不要再调用工具。"
        }
    }
}

pub fn client_visible_model(body: &[u8], fallback: &str) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("model").and_then(Value::as_str).map(str::trim).map(str::to_string))
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

pub fn request_declares_tools(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("tools").and_then(Value::as_array).map(|tools| !tools.is_empty()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOnlyStep {
    RetrySame,
    NextRoute,
    Forward,
}

/// 只回文字时的次数。第 1 条 3 次，第 2 条 2 次，第 3 条 1 次。再往后不为这件事试。
/// 次数只在这一次请求里累加，这里不保存。交回客户端之后，下一次请求从 3 次重新算。
pub fn text_only_budget(attempt_idx: usize) -> u32 {
    match attempt_idx {
        0 => 3,
        1 => 2,
        2 => 1,
        _ => 0,
    }
}

/// `tries_on_route` 是这条路由上已经拿到的纯文本回复次数，含这一次。
/// `paid_total` 为收费模型的总次数（含第一次）。`None` 走免费通道的 3、2、1。
pub fn text_only_step(
    tries_on_route: u32,
    attempt_idx: usize,
    has_another_route: bool,
    paid_total: Option<u32>,
) -> TextOnlyStep {
    let budget = paid_total.unwrap_or_else(|| text_only_budget(attempt_idx));
    let next_budget = if paid_total.is_some() {
        0
    } else if has_another_route {
        text_only_budget(attempt_idx + 1)
    } else {
        0
    };
    if tries_on_route < budget {
        TextOnlyStep::RetrySame
    } else if next_budget > 0 {
        TextOnlyStep::NextRoute
    } else {
        TextOnlyStep::Forward
    }
}

/// 流式 Chat SSE 里是否已经出现工具调用。只认解析成功的 `data:` 行。
pub fn chat_sse_saw_tool_call(buf: &str) -> bool {
    buf.split('\n').any(|line| {
        let line = line.trim().trim_end_matches('\r');
        let Some(data) = line.strip_prefix("data:") else {
            return false;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return false;
        }
        serde_json::from_str::<Value>(data)
            .ok()
            .map(|value| choice_has_tool_call(&value))
            .unwrap_or(false)
    })
}

fn choice_has_tool_call(value: &Value) -> bool {
    let Some(choice) = value.pointer("/choices/0") else {
        return false;
    };
    if choice.get("finish_reason").and_then(Value::as_str) == Some("tool_calls") {
        return true;
    }
    ["/delta/tool_calls", "/message/tool_calls"].iter().any(|path| {
        choice
            .pointer(path)
            .and_then(Value::as_array)
            .map(|calls| !calls.is_empty())
            .unwrap_or(false)
    })
}

/// 非流式 Chat JSON：声明了 tools，但这条回复既没有 tool_calls，也不是 tool_calls 收尾。
pub fn assistant_text_omits_tools(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let Some(choice) = value.pointer("/choices/0") else {
        return false;
    };
    if choice.get("finish_reason").and_then(Value::as_str) == Some("tool_calls") {
        return false;
    }
    choice
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.is_empty())
        .unwrap_or(true)
}

pub fn notice_response(wire: Wire, model: &str, kind: NoticeKind, stream: bool) -> Response<Body> {
    let text = notice_text(kind);
    let (content_type, body) = if stream {
        let payload = match wire {
            Wire::Responses => responses_notice_sse(model, text),
            Wire::Chat => chat_notice_sse(model, text),
            Wire::Anthropic => anthropic_notice_sse(model, text),
        };
        ("text/event-stream; charset=utf-8", payload)
    } else {
        let payload = match wire {
            Wire::Responses => responses_notice_json(model, text).to_string(),
            Wire::Chat => chat_notice_json(model, text).to_string(),
            Wire::Anthropic => anthropic_notice_json(model, text).to_string(),
        };
        ("application/json", payload)
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

pub fn responses_notice_json(model: &str, text: &str) -> Value {
    json!({
        "id": "sugt-notice",
        "object": "response",
        "status": "completed",
        "model": model,
        "output": [{
            "type": "message",
            "id": "msg_sugt_notice",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text }]
        }],
        "output_text": text,
        "usage": {
            "input_tokens": 0,
            "input_tokens_details": { "cached_tokens": 0 },
            "output_tokens": 0,
            "output_tokens_details": { "reasoning_tokens": 0 },
            "total_tokens": 0
        },
        "parallel_tool_calls": false,
        "error": Value::Null
    })
}

pub fn chat_notice_json(model: &str, text: &str) -> Value {
    json!({
        "id": "chatcmpl-sugt-notice",
        "object": "chat.completion",
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }]
    })
}

pub fn anthropic_notice_json(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_sugt_notice",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{ "type": "text", "text": text }],
        "stop_reason": "end_turn",
        "stop_sequence": Value::Null,
        "usage": { "input_tokens": 0, "output_tokens": 0 }
    })
}

fn sse_json(event: &str, value: &Value) -> String {
    format!("event: {event}\ndata: {value}\n\n")
}

pub fn responses_notice_sse(model: &str, text: &str) -> String {
    let response_id = "sugt-notice";
    let item_id = "msg_sugt_notice";
    let created = json!({
        "type": "response.created",
        "sequence_number": 1,
        "response": { "id": response_id, "object": "response", "status": "in_progress", "model": model, "output": [] }
    });
    let progress = json!({
        "type": "response.in_progress",
        "sequence_number": 2,
        "response": { "id": response_id, "object": "response", "status": "in_progress", "model": model, "output": [] }
    });
    let item_added = json!({
        "type": "response.output_item.added",
        "sequence_number": 3,
        "output_index": 0,
        "item": { "type": "message", "id": item_id, "role": "assistant", "status": "in_progress", "content": [] }
    });
    let part_added = json!({
        "type": "response.content_part.added",
        "sequence_number": 4,
        "item_id": item_id,
        "output_index": 0,
        "content_index": 0,
        "part": { "type": "output_text", "text": "" }
    });
    let delta = json!({
        "type": "response.output_text.delta",
        "sequence_number": 5,
        "item_id": item_id,
        "output_index": 0,
        "content_index": 0,
        "delta": text
    });
    let text_done = json!({
        "type": "response.output_text.done",
        "sequence_number": 6,
        "item_id": item_id,
        "output_index": 0,
        "content_index": 0,
        "text": text
    });
    let part_done = json!({
        "type": "response.content_part.done",
        "sequence_number": 7,
        "item_id": item_id,
        "output_index": 0,
        "content_index": 0,
        "part": { "type": "output_text", "text": text }
    });
    let item = json!({
        "type": "message",
        "id": item_id,
        "role": "assistant",
        "status": "completed",
        "content": [{ "type": "output_text", "text": text }]
    });
    let item_done = json!({
        "type": "response.output_item.done",
        "sequence_number": 8,
        "output_index": 0,
        "item": item
    });
    let completed = json!({
        "type": "response.completed",
        "sequence_number": 9,
        "response": {
            "id": response_id,
            "object": "response",
            "status": "completed",
            "model": model,
            "output": [item],
            "output_text": text,
            "usage": {
                "input_tokens": 0,
                "input_tokens_details": { "cached_tokens": 0 },
                "output_tokens": 0,
                "output_tokens_details": { "reasoning_tokens": 0 },
                "total_tokens": 0
            },
            "error": Value::Null
        }
    });
    sse_json("response.created", &created)
        + &sse_json("response.in_progress", &progress)
        + &sse_json("response.output_item.added", &item_added)
        + &sse_json("response.content_part.added", &part_added)
        + &sse_json("response.output_text.delta", &delta)
        + &sse_json("response.output_text.done", &text_done)
        + &sse_json("response.content_part.done", &part_done)
        + &sse_json("response.output_item.done", &item_done)
        + &sse_json("response.completed", &completed)
        + "data: [DONE]\n\n"
}

pub fn chat_notice_sse(model: &str, text: &str) -> String {
    let first = json!({
        "id": "chatcmpl-sugt-notice",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{ "index": 0, "delta": { "role": "assistant", "content": text }, "finish_reason": Value::Null }]
    });
    let stop = json!({
        "id": "chatcmpl-sugt-notice",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
    });
    format!("data: {first}\n\ndata: {stop}\n\ndata: [DONE]\n\n")
}

pub fn anthropic_notice_sse(model: &str, text: &str) -> String {
    let start = json!({
        "type": "message_start",
        "message": {
            "id": "msg_sugt_notice",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [],
            "stop_reason": Value::Null,
            "stop_sequence": Value::Null,
            "usage": { "input_tokens": 0, "output_tokens": 0 }
        }
    });
    let block = json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": { "type": "text", "text": "" }
    });
    let delta = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "text_delta", "text": text }
    });
    let block_stop = json!({ "type": "content_block_stop", "index": 0 });
    let message_delta = json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn", "stop_sequence": Value::Null },
        "usage": { "output_tokens": 0 }
    });
    let message_stop = json!({ "type": "message_stop" });
    sse_json("message_start", &start)
        + &sse_json("content_block_start", &block)
        + &sse_json("content_block_delta", &delta)
        + &sse_json("content_block_stop", &block_stop)
        + &sse_json("message_delta", &message_delta)
        + &sse_json("message_stop", &message_stop)
}

/// 按 SSE 事件边界改写 `model`。半个事件留在缓冲里，不在多字节字符中间切开。
pub struct ModelRewriter {
    buf: Vec<u8>,
    model: String,
}

impl ModelRewriter {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            buf: Vec::new(),
            model: model.into(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(end) = find_event_end(&self.buf) {
            let event: Vec<u8> = self.buf.drain(..end).collect();
            out.extend(rewrite_event(&event, &self.model));
        }
        out
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.buf);
        rewrite_event(&rest, &self.model)
    }
}

fn backoff(pass: u32, hinted: Duration, single_cap: Duration) -> Duration {
    let floor = Duration::from_secs(2u64.saturating_mul(u64::from(pass.max(1))));
    hinted.max(floor).min(single_cap).max(Duration::from_millis(1))
}

fn is_context_length(lower: &str) -> bool {
    lower.contains("context_length")
        || lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("too many tokens")
        || lower.contains("max_tokens")
            && lower.contains("context")
}

fn model_rejected(lower: &str) -> bool {
    (lower.contains("model") || lower.contains("not found") || lower.contains("no such"))
        && !is_context_length(lower)
}

fn retry_after_hint(header: Option<&str>, body: &str, default: Duration) -> Duration {
    if let Some(header) = header {
        if let Some(wait) = parse_retry_after_token(header.trim()) {
            return wait;
        }
    }
    if let Some(wait) = retry_after_in_body(body) {
        return wait;
    }
    default
}

fn parse_retry_after_token(token: &str) -> Option<Duration> {
    if token.is_empty() {
        return None;
    }
    if token.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return http_date_delay(token);
    }
    let seconds: f64 = token.parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(seconds))
}

/// `Retry-After` 可以是 IMF-fixdate。解析不了就交给调用方用默认秒数。
fn http_date_delay(token: &str) -> Option<Duration> {
    let when = chrono::DateTime::parse_from_rfc2822(token.trim()).ok()?;
    let wait = when.signed_duration_since(chrono::Utc::now());
    if wait.num_milliseconds() <= 0 {
        return Some(Duration::ZERO);
    }
    wait.to_std().ok()
}

fn retry_after_in_body(body: &str) -> Option<Duration> {
    let value: Value = serde_json::from_str(body).ok()?;
    let number = value
        .pointer("/error/retry_after")
        .or_else(|| value.pointer("/retry_after"))
        .or_else(|| value.pointer("/error/metadata/retry_after"))
        .and_then(Value::as_f64)?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(number))
}

fn find_event_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|pair| pair == b"\n\n").map(|at| at + 2)
}

fn rewrite_event(event: &[u8], model: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(event) else {
        return event.to_vec();
    };
    if text.trim_start().starts_with('{') {
        return rewrite_json_model(event, model);
    }
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let payload = line
            .trim_end_matches(['\n', '\r'])
            .strip_prefix("data:")
            .map(str::trim);
        if let Some(payload) = payload {
            if payload != "[DONE]" {
                if let Ok(value) = serde_json::from_str::<Value>(payload) {
                    if value.get("model").is_some() {
                        let rewritten = rewrite_json_value(value, model);
                        out.push_str("data: ");
                        out.push_str(&rewritten.to_string());
                        if line.ends_with('\n') {
                            out.push('\n');
                        }
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
    }
    out.into_bytes()
}

pub fn rewrite_json_model(bytes: &[u8], model: &str) -> Vec<u8> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return bytes.to_vec();
    };
    if value.get("model").is_none() {
        return bytes.to_vec();
    }
    serde_json::to_vec(&rewrite_json_value(value, model)).unwrap_or_else(|_| bytes.to_vec())
}

fn rewrite_json_value(mut value: Value, model: &str) -> Value {
    if let Some(object) = value.as_object_mut() {
        if object.contains_key("model") {
            object.insert("model".to_string(), Value::String(model.to_string()));
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_on_free_route_is_retryable_and_uses_retry_after() {
        let class = classify_http(429, r#"{"retry_after":2}"#, Some("30"), false, true);
        assert_eq!(
            class,
            FailureClass::Retryable {
                retry_after: Duration::from_secs(30),
                reason: RetryReason::Quota,
            }
        );
    }

    #[test]
    fn own_key_401_stops_without_waiting() {
        let class = classify_http(401, "invalid api key", None, true, false);
        assert_eq!(class, FailureClass::Terminal(NoticeKind::Auth));
    }

    #[test]
    fn context_length_is_terminal_even_on_429_wording() {
        let class = classify_http(
            400,
            "maximum context length exceeded",
            None,
            true,
            false,
        );
        assert_eq!(class, FailureClass::Terminal(NoticeKind::ContextTooLong));
    }

    #[test]
    fn zero_budget_does_not_sleep() {
        let mut budget = RetryBudget::from_limits(Duration::ZERO, DEFAULT_SINGLE_CAP);
        let mut memory = FailureMemory::new();
        assert!(memory
            .observe(classify_http(500, "down", None, true, false))
            .is_none());
        assert_eq!(
            memory.finish(&mut budget, 1),
            PassFinish::Notice(NoticeKind::UpstreamDown)
        );
    }

    #[test]
    fn free_quota_after_budget_uses_router_notice() {
        let mut memory = FailureMemory::new();
        memory.set_free(true);
        memory.observe(classify_http(429, "", None, false, true));
        let mut budget = RetryBudget::from_limits(Duration::ZERO, DEFAULT_SINGLE_CAP);
        assert_eq!(
            memory.finish(&mut budget, 1),
            PassFinish::Notice(NoticeKind::FreeExhausted)
        );
        assert!(notice_text(NoticeKind::FreeExhausted).contains("免费路由"));
        assert!(notice_text(NoticeKind::Quota).contains("额度不足"));
    }

    #[test]
    fn client_model_is_echoed_and_fallback_is_stable() {
        assert_eq!(
            client_visible_model(br#"{"model":"sugt-sutai"}"#, "sutai"),
            "sugt-sutai"
        );
        assert_eq!(client_visible_model(br#"{"input":"hi"}"#, "sutai"), "sutai");
    }

    #[test]
    fn notice_is_a_completed_response_not_a_failure() {
        let value = responses_notice_json("sugt-sutai", notice_text(NoticeKind::Quota));
        assert_eq!(value["status"], "completed");
        assert_eq!(value["model"], "sugt-sutai");
        assert!(value["output_text"].as_str().unwrap().contains("小su提示"));
        assert!(value.get("error").unwrap().is_null());
        let sse = responses_notice_sse("sugt-sutai", "小su提示");
        assert!(sse.contains("response.completed"));
        assert!(!sse.contains("response.failed"));
    }

    #[test]
    fn model_rewriter_keeps_partial_events_until_boundary() {
        let mut rewriter = ModelRewriter::new("sutai");
        let first = rewriter.push(br#"data: {"model":"openrouter/free","choices":[]}"#);
        assert!(first.is_empty(), "没有事件结尾不能把半包交出去");
        let second = rewriter.push(b"\n\n");
        let text = String::from_utf8(second).unwrap();
        assert!(text.contains("\"model\":\"sutai\""));
        assert!(!text.contains("openrouter/free"));
    }

    #[test]
    fn plain_json_model_is_rewritten_at_finish() {
        let mut rewriter = ModelRewriter::new("sutai");
        assert!(rewriter.push(br#"{"model":"kilo-auto/free","choices":[]}"#).is_empty());
        let text = String::from_utf8(rewriter.finish()).unwrap();
        assert!(text.contains("\"model\":\"sutai\""));
    }

    #[test]
    fn text_only_tapers_three_then_two_then_one() {
        assert_eq!(text_only_step(1, 0, true, None), TextOnlyStep::RetrySame);
        assert_eq!(text_only_step(2, 0, true, None), TextOnlyStep::RetrySame);
        assert_eq!(text_only_step(3, 0, true, None), TextOnlyStep::NextRoute);
        assert_eq!(text_only_step(3, 0, false, None), TextOnlyStep::Forward);
        assert_eq!(text_only_step(1, 1, true, None), TextOnlyStep::RetrySame);
        assert_eq!(text_only_step(2, 1, true, None), TextOnlyStep::NextRoute);
        assert_eq!(text_only_step(1, 2, true, None), TextOnlyStep::Forward);
        assert_eq!(text_only_step(1, 0, false, Some(3)), TextOnlyStep::RetrySame);
        assert_eq!(text_only_step(3, 0, false, Some(3)), TextOnlyStep::Forward);
        assert_eq!(text_only_step(1, 0, false, Some(1)), TextOnlyStep::Forward);
    }

    #[test]
    fn chat_sse_tool_call_is_visible_before_done() {
        let partial = "data: {\"choices\":[{\"delta\":{\"content\":\"让我看看\"}}]}\n\n";
        assert!(!chat_sse_saw_tool_call(partial));
        let with_tool = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\"}]}}]}\n\n";
        assert!(chat_sse_saw_tool_call(with_tool));
    }

    #[test]
    fn tool_omission_detects_a_text_only_answer() {
        assert!(assistant_text_omits_tools(
            r#"{"choices":[{"message":{"role":"assistant","content":"好的"},"finish_reason":"stop"}]}"#
        ));
        assert!(!assistant_text_omits_tools(
            r#"{"choices":[{"message":{"tool_calls":[{"id":"1"}]},"finish_reason":"tool_calls"}]}"#
        ));
    }

    #[test]
    fn free_switch_without_quota_uses_router_notice() {
        let mut memory = FailureMemory::new();
        memory.set_free(true);
        memory.observe(FailureClass::Switch);
        assert_eq!(memory.notice_kind(), NoticeKind::FreeExhausted);
    }

    #[test]
    fn http_date_retry_after_becomes_a_wait() {
        let when = chrono::Utc::now() + chrono::Duration::seconds(40);
        let header = when.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let class = classify_http(429, "", Some(&header), false, false);
        let FailureClass::Retryable { retry_after, .. } = class else {
            panic!("日期头应可重试：{class:?}");
        };
        let secs = retry_after.as_secs();
        assert!(
            (20..70).contains(&secs),
            "应落在大约 40 秒附近，实际 {secs}"
        );
        let ignored = classify_http(429, "", Some("soon"), false, false);
        assert_eq!(
            ignored,
            FailureClass::Retryable {
                retry_after: Duration::from_secs(5),
                reason: RetryReason::Quota,
            }
        );
    }

    #[test]
    fn single_wait_is_capped() {
        let wait = backoff(1, Duration::from_secs(300), Duration::from_secs(60));
        assert_eq!(wait, Duration::from_secs(60));
    }
}
