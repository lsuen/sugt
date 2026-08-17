use crate::gateway_stats::GatewayStatsCollector;
use anyhow::{Context, Result};
use async_stream::stream;
use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use bytes::Bytes;
use futures_util::{pin_mut, Stream, StreamExt};
use reqwest::Response as ReqwestResponse;
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub fn anthropic_to_openai(body: Bytes, model_name: &str) -> Result<Bytes> {
    let value: Value = serde_json::from_slice(&body).context("Anthropic 请求体不是合法 JSON")?;
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut openai_messages = Vec::new();

    if let Some(system) = value.get("system") {
        let system_text = anthropic_content_to_text(system);
        if !system_text.is_empty() {
            openai_messages.push(json!({ "role": "system", "content": system_text }));
        }
    }

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = message.get("content");

        match content {
            Some(Value::String(text)) => {
                openai_messages.push(json!({ "role": role, "content": text }));
            }
            Some(Value::Array(blocks)) => {
                convert_anthropic_blocks(role, blocks, &mut openai_messages);
            }
            None => {}
            Some(other) => {
                let text = anthropic_content_to_text(other);
                if !text.is_empty() {
                    openai_messages.push(json!({ "role": role, "content": text }));
                }
            }
        }
    }

    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut payload = json!({
        "model": model_name,
        "messages": openai_messages,
        "stream": stream,
    });

    if stream {
        payload["stream_options"] = json!({ "include_usage": true });
    }

    if let Some(max_tokens) = value
        .get("max_tokens")
        .or_else(|| value.get("max_tokens_to_sample"))
    {
        payload["max_tokens"] = max_tokens.clone();
    }
    // 跳过 null 字段：部分 Anthropic 客户端会显式发送 null，直接透传会被部分 OpenAI 服务商拒绝
    if let Some(temperature) = value.get("temperature") {
        if !temperature.is_null() {
            payload["temperature"] = temperature.clone();
        }
    }
    if let Some(top_p) = value.get("top_p") {
        if !top_p.is_null() {
            payload["top_p"] = top_p.clone();
        }
    }
    if let Some(stop_sequences) = value.get("stop_sequences") {
        if !stop_sequences.is_null() {
            payload["stop"] = stop_sequences.clone();
        }
    }

    if let Some(tools) = value.get("tools") {
        payload["tools"] = anthropic_tools_to_openai(tools);
    }
    if let Some(tool_choice) = value.get("tool_choice") {
        payload["tool_choice"] = anthropic_tool_choice_to_openai(tool_choice);
    }

    Ok(Bytes::from(serde_json::to_vec(&payload)?))
}

fn convert_anthropic_blocks(role: &str, blocks: &[Value], openai_messages: &mut Vec<Value>) {
    if role == "assistant" {
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        text_parts.push(text.to_string());
                    }
                }
                Some("tool_use") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or("tool");
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
                        }
                    }));
                }
                Some("thinking") => {
                    if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                        text_parts.push(format!("[thinking]\n{thinking}"));
                    }
                }
                _ => {}
            }
        }

        let mut message = json!({ "role": "assistant" });
        if !text_parts.is_empty() {
            message["content"] = json!(text_parts.join("\n"));
        }
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
            if message.get("content").is_none() {
                message["content"] = Value::Null;
            }
        }
        if message.get("content").is_some() || message.get("tool_calls").is_some() {
            openai_messages.push(message);
        }
        return;
    }

    if role == "user" {
        let mut text_parts = Vec::new();
        let mut image_parts = Vec::new();
        let mut tool_results = Vec::new();

        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        text_parts.push(text.to_string());
                    }
                }
                Some("tool_result") => {
                    let tool_use_id = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let content = block
                        .get("content")
                        .map(anthropic_content_to_text)
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| {
                            block
                                .get("content")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        });
                    tool_results.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    }));
                }
                Some("image") => {
                    // 图片块 → OpenAI image_url（base64 data URL），避免视觉能力丢失
                    if let Some(image) = anthropic_image_block_to_openai(block) {
                        image_parts.push(image);
                    }
                }
                _ => {}
            }
        }

        if !text_parts.is_empty() && image_parts.is_empty() {
            openai_messages.push(json!({ "role": "user", "content": text_parts.join("\n") }));
        } else if !text_parts.is_empty() || !image_parts.is_empty() {
            let mut content_parts = Vec::new();
            if !text_parts.is_empty() {
                content_parts.push(json!({ "type": "text", "text": text_parts.join("\n") }));
            }
            content_parts.extend(image_parts);
            openai_messages.push(json!({ "role": "user", "content": content_parts }));
        }
        openai_messages.extend(tool_results);
        return;
    }

    let text = blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        openai_messages.push(json!({ "role": role, "content": text }));
    }
}

/// 将 Anthropic image 块转换为 OpenAI image_url 内容块。
///
/// Anthropic 格式：`{"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}`
/// OpenAI 格式：`{"type":"image_url","image_url":{"url":"data:image/png;base64,..."}}`
fn anthropic_image_block_to_openai(block: &Value) -> Option<Value> {
    let source = block.get("source")?;
    let media_type = source.get("media_type").and_then(Value::as_str)?;
    match source.get("type").and_then(Value::as_str)? {
        "base64" => {
            let data = source.get("data").and_then(Value::as_str)?;
            Some(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{media_type};base64,{data}") }
            }))
        }
        // url 类型直接透传
        "url" => {
            let url = source.get("url").and_then(Value::as_str)?;
            Some(json!({
                "type": "image_url",
                "image_url": { "url": url }
            }))
        }
        _ => None,
    }
}

fn anthropic_tools_to_openai(tools: &Value) -> Value {
    let Some(items) = tools.as_array() else {
        return tools.clone();
    };

    Value::Array(
        items
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(Value::as_str)?;
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let schema = tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": schema,
                    }
                }))
            })
            .collect(),
    )
}

fn anthropic_tool_choice_to_openai(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(text) if text == "auto" => json!("auto"),
        Value::String(text) if text == "any" => json!("required"),
        Value::String(text) if text == "none" => json!("none"),
        // 新版 Claude 客户端使用对象形式：{"type":"auto"|"none"|"any"}，需映射为 OpenAI 字符串
        Value::Object(map) => match map.get("type").and_then(Value::as_str) {
            Some("tool") => {
                let name = map.get("name").and_then(Value::as_str).unwrap_or("");
                json!({ "type": "function", "function": { "name": name } })
            }
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            Some("any") => json!("required"),
            _ => tool_choice.clone(),
        },
        other => other.clone(),
    }
}

pub fn anthropic_content_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                Some("text") => item
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                Some("tool_result") => item
                    .get("content")
                    .map(anthropic_content_to_text)
                    .filter(|text| !text.is_empty()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn openai_json_to_anthropic_json(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes).context("OpenAI 响应体不是合法 JSON")?;
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let message = choice.get("message").cloned().unwrap_or_else(|| json!({}));
    let mut content = Vec::new();

    // 推理内容（deepseek 等返回 message.reasoning_content）→ anthropic thinking 块
    if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str) {
        if !reasoning.is_empty() {
            content.push(json!({
                "type": "thinking",
                "thinking": reasoning,
                "signature": ""
            }));
        }
    }

    if let Some(text) = message.get("content") {
        let text = openai_message_content_to_text(Some(text));
        if !text.is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("tool");
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let input = openai_function_arguments_to_json(call.pointer("/function/arguments"));
            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }));
        }
    }

    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }

    let stop_reason = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        Some(_) => "end_turn",
        None => "end_turn",
    };

    Ok(json!({
        "id": value.get("id").and_then(Value::as_str).unwrap_or("msg_sugt"),
        "type": "message",
        "role": "assistant",
        "model": value.get("model").and_then(Value::as_str).unwrap_or("sugt"),
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": value.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            "output_tokens": value.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
        }
    }))
}

fn openai_function_arguments_to_json(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(text)) if text.trim().is_empty() => json!({}),
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| json!({ "raw": text }))
        }
        Some(Value::Object(_)) | Some(Value::Array(_)) => arguments.cloned().unwrap_or(json!({})),
        Some(other) => json!({ "raw": other }),
        None => json!({}),
    }
}

/// 从 OpenAI 流式 delta 中提取文本内容，兼容字符串与数组（多模态服务商常见）。
fn openai_delta_content_text(delta: &Value) -> Option<String> {
    match delta.get("content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Value::Null => None,
        _ => None,
    }
}

fn openai_message_content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn is_streaming_response(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/event-stream"))
}

pub fn request_wants_stream(body: &Bytes) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

pub async fn openai_to_anthropic_response(
    response: ReqwestResponse,
    model: &str,
    stats: Option<GatewayStatsCollector>,
) -> Result<Response<Body>> {
    let status = response.status();
    let headers = response.headers().clone();

    if !status.is_success() {
        let bytes = response.bytes().await?;
        // 把 OpenAI 错误体转换为 Anthropic 错误格式，避免 Claude 客户端解析失败
        let anthropic_error = openai_error_to_anthropic_error(&bytes);
        return passthrough_response(status, headers, Bytes::from(anthropic_error));
    }

    if is_streaming_response(&headers) {
        let stream = response
            .bytes_stream()
            .map(|item| item.map_err(std::io::Error::other));
        let converted = openai_sse_to_anthropic_sse(stream, model.to_string(), stats);
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(
                http::header::CONTENT_TYPE,
                "text/event-stream; charset=utf-8",
            )
            .header(http::header::CACHE_CONTROL, "no-cache")
            .header(http::header::CONNECTION, "keep-alive")
            .body(Body::from_stream(converted))?);
    }

    let bytes = response.bytes().await?;
    let anthropic = openai_json_to_anthropic_json(&bytes)?;
    if let Some(collector) = stats {
        let input = anthropic
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = anthropic
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if input > 0 || output > 0 {
            collector.record_tokens(input, output).await;
        }
    }
    Ok(Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(anthropic.to_string()))?)
}

/// 把 OpenAI 错误体转换为 Anthropic 错误格式。
///
/// Claude 客户端期望的响应是 `{"type":"error","error":{"type":...,"message":...}}`，
/// 若直接透传 OpenAI 的 `{"error":{"message":...}}` 会导致客户端解析失败或显示异常。
fn openai_error_to_anthropic_error(body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            // 非 JSON 或结构不符时，尝试用原始文本截断作为错误信息
            String::from_utf8_lossy(body)
                .trim()
                .chars()
                .take(500)
                .collect()
        });

    json!({
        "type": "error",
        "error": {
            "type": "api_error",
            "message": message,
        }
    })
    .to_string()
}

fn passthrough_response(
    status: StatusCode,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Response<Body>> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(name, value);
    }
    Ok(builder.body(Body::from(bytes))?)
}

pub fn anthropic_auth_headers(api_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", api_key.parse()?);
    headers.insert("anthropic-version", ANTHROPIC_VERSION.parse()?);
    Ok(headers)
}

fn openai_sse_to_anthropic_sse(
    input: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    model: String,
    stats: Option<GatewayStatsCollector>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    stream! {
        let mut parser = SseLineParser::new();
        let mut adapter = AnthropicStreamAdapter::new(&model);
        pin_mut!(input);

        while let Some(chunk) = input.next().await {
            let chunk = chunk?;
            for line in parser.push(&chunk) {
                for event in adapter.handle_openai_line(&line) {
                    yield Ok(Bytes::from(event));
                }
            }
        }

        for event in adapter.finish() {
            yield Ok(Bytes::from(event));
        }

        if let Some(collector) = stats {
            let input_tokens = adapter.input_tokens;
            let output_tokens = adapter.output_tokens;
            if input_tokens > 0 || output_tokens > 0 {
                collector.record_tokens(input_tokens, output_tokens).await;
            }
        }
    }
}

struct SseLineParser {
    buffer: String,
}

impl SseLineParser {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    fn push(&mut self, chunk: &Bytes) -> Vec<String> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut lines = Vec::new();
        loop {
            let Some(pos) = self.buffer.find('\n') else {
                break;
            };
            let mut line = self.buffer.drain(..=pos).collect::<String>();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        lines
    }
}

struct ToolCallStreamState {
    block_index: u32,
    id: String,
    name: String,
    arguments: String,
    block_started: bool,
}

struct AnthropicStreamAdapter {
    message_id: String,
    model: String,
    started: bool,
    finished: bool,
    input_tokens: u64,
    output_tokens: u64,
    next_block_index: u32,
    text_block_index: Option<u32>,
    thinking_block_index: Option<u32>,
    tool_calls: HashMap<usize, ToolCallStreamState>,
}

impl AnthropicStreamAdapter {
    fn new(model: &str) -> Self {
        Self {
            message_id: format!("msg_{}", Uuid::new_v4()),
            model: model.to_string(),
            started: false,
            finished: false,
            input_tokens: 0,
            output_tokens: 0,
            next_block_index: 0,
            text_block_index: None,
            thinking_block_index: None,
            tool_calls: HashMap::new(),
        }
    }

    fn handle_openai_line(&mut self, line: &str) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }

        let Some(payload) = line.strip_prefix("data: ") else {
            return Vec::new();
        };

        if payload.trim() == "[DONE]" {
            return self.finish();
        }

        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return Vec::new();
        };

        let choice = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .cloned()
            .unwrap_or_else(|| json!({}));

        let delta = choice.get("delta").cloned().unwrap_or_else(|| json!({}));
        let mut events = Vec::new();

        if !self.started {
            self.started = true;
            events.push(self.message_start_event());
            events.push(format_sse("ping", json!({ "type": "ping" })));
        }

        // 处理 content：兼容字符串与数组（部分服务商流式返回数组 content）
        if let Some(text) = openai_delta_content_text(&delta) {
            if !text.is_empty() {
                events.extend(self.ensure_text_block());
                self.output_tokens += text.chars().count() as u64;
                events.push(format_sse(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.text_block_index.unwrap_or(0),
                        "delta": { "type": "text_delta", "text": text }
                    }),
                ));
            }
        }

        // 推理内容（deepseek 等返回 delta.reasoning_content）→ anthropic thinking 块
        if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
            if !reasoning.is_empty() {
                events.extend(self.ensure_thinking_block());
                events.push(format_sse(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.thinking_block_index.unwrap_or(0),
                        "delta": { "type": "thinking_delta", "thinking": reasoning }
                    }),
                ));
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                events.extend(self.handle_tool_call_delta(call));
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            let stop_reason = match reason {
                "length" => "max_tokens",
                "tool_calls" => "tool_use",
                _ => "end_turn",
            };
            events.extend(self.finish_with_reason(stop_reason));
        } else if let Some(usage) = value.get("usage") {
            if let Some(prompt) = usage
                .get("prompt_tokens")
                .or_else(|| usage.get("input_tokens"))
                .and_then(Value::as_u64)
            {
                self.input_tokens = prompt;
            }
            if let Some(completion) = usage
                .get("completion_tokens")
                .or_else(|| usage.get("output_tokens"))
                .and_then(Value::as_u64)
            {
                self.output_tokens = completion;
            }
        }

        events
    }

    fn message_start_event(&self) -> String {
        format_sse(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                }
            }),
        )
    }

    fn ensure_text_block(&mut self) -> Vec<String> {
        if self.text_block_index.is_some() {
            return Vec::new();
        }

        let index = self.next_block_index;
        self.next_block_index += 1;
        self.text_block_index = Some(index);

        vec![format_sse(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": { "type": "text", "text": "" }
            }),
        )]
    }

    fn ensure_thinking_block(&mut self) -> Vec<String> {
        if self.thinking_block_index.is_some() {
            return Vec::new();
        }

        let index = self.next_block_index;
        self.next_block_index += 1;
        self.thinking_block_index = Some(index);

        vec![format_sse(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": { "type": "thinking", "thinking": "", "signature": "" }
            }),
        )]
    }

    fn close_text_block_if_open(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if let Some(index) = self.text_block_index.take() {
            events.push(format_sse(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": index }),
            ));
        }
        if let Some(index) = self.thinking_block_index.take() {
            events.push(format_sse(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": index }),
            ));
        }
        events
    }

    fn handle_tool_call_delta(&mut self, call: &Value) -> Vec<String> {
        let openai_index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let id = call.get("id").and_then(Value::as_str).map(str::to_string);
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let args_fragment = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        if !self.tool_calls.contains_key(&openai_index) {
            self.tool_calls.insert(
                openai_index,
                ToolCallStreamState {
                    block_index: 0,
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                    block_started: false,
                },
            );
        }

        let state = self.tool_calls.get_mut(&openai_index).expect("tool state");
        if let Some(id) = id {
            state.id = id;
        }
        if let Some(name) = name {
            state.name = name;
        }

        let should_start = !state.block_started && (!state.id.is_empty() || !state.name.is_empty());
        let mut events = Vec::new();

        if should_start {
            events.extend(self.close_text_block_if_open());
            let state = self.tool_calls.get_mut(&openai_index).expect("tool state");
            state.block_index = self.next_block_index;
            self.next_block_index += 1;
            state.block_started = true;

            let tool_id = if state.id.is_empty() {
                format!("tool_{openai_index}")
            } else {
                state.id.clone()
            };
            let tool_name = if state.name.is_empty() {
                "tool".to_string()
            } else {
                state.name.clone()
            };

            events.push(format_sse(
                "content_block_start",
                json!({
                    "type": "content_block_start",
                    "index": state.block_index,
                    "content_block": {
                        "type": "tool_use",
                        "id": tool_id,
                        "name": tool_name,
                        "input": {}
                    }
                }),
            ));
        }

        if !args_fragment.is_empty() {
            let state = self.tool_calls.get_mut(&openai_index).expect("tool state");
            state.arguments.push_str(&args_fragment);
            if state.block_started {
                let block_index = state.block_index;
                events.push(format_sse(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": args_fragment
                        }
                    }),
                ));
            }
        }

        events
    }

    fn close_open_blocks(&mut self) -> Vec<String> {
        let mut events = self.close_text_block_if_open();
        let mut tool_indexes: Vec<_> = self
            .tool_calls
            .values()
            .filter(|state| state.block_started)
            .map(|state| state.block_index)
            .collect();
        tool_indexes.sort_unstable();
        for index in tool_indexes {
            events.push(format_sse(
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": index }),
            ));
        }
        events
    }

    fn finish(&mut self) -> Vec<String> {
        self.finish_with_reason("end_turn")
    }

    fn finish_with_reason(&mut self, stop_reason: &str) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;

        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            events.push(self.message_start_event());
        }

        events.extend(self.close_open_blocks());
        events.push(format_sse(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": Value::Null },
                "usage": { "output_tokens": self.output_tokens }
            }),
        ));
        events.push(format_sse(
            "message_stop",
            json!({ "type": "message_stop" }),
        ));
        events
    }
}

fn format_sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn converts_anthropic_tool_messages_to_openai() {
        let body = Bytes::from_static(
            br#"{"system":"sys","messages":[{"role":"user","content":[{"type":"text","text":"hi"}]},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"bash","input":{"cmd":"ls"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}],"max_tokens":100,"stream":true}"#,
        );
        let payload = anthropic_to_openai(body, "target").unwrap();
        let value: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["stream"], true);
        assert_eq!(
            value["messages"][2]["tool_calls"][0]["function"]["name"],
            "bash"
        );
        assert_eq!(value["messages"][3]["role"], "tool");
    }

    #[test]
    fn converts_openai_json_with_tool_calls() {
        let response = br#"{"id":"chatcmpl-1","model":"m","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"t1","type":"function","function":{"name":"bash","arguments":"{\"cmd\":\"ls\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#;
        let value = openai_json_to_anthropic_json(response).unwrap();
        assert_eq!(value["content"][0]["type"], "tool_use");
        assert_eq!(value["stop_reason"], "tool_use");
    }

    #[test]
    fn converts_openai_json_with_object_arguments() {
        let response = br#"{"id":"chatcmpl-1","model":"m","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"t1","type":"function","function":{"name":"bash","arguments":{"cmd":"ls"}}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#;
        let value = openai_json_to_anthropic_json(response).unwrap();
        assert_eq!(value["content"][0]["input"]["cmd"], "ls");
    }

    #[test]
    fn maps_anthropic_tool_choice_object_to_openai() {
        assert_eq!(anthropic_tool_choice_to_openai(&json!({"type": "auto"})), json!("auto"));
        assert_eq!(anthropic_tool_choice_to_openai(&json!({"type": "none"})), json!("none"));
        assert_eq!(anthropic_tool_choice_to_openai(&json!({"type": "any"})), json!("required"));
        assert_eq!(
            anthropic_tool_choice_to_openai(&json!({"type": "tool", "name": "bash"})),
            json!({"type": "function", "function": {"name": "bash"}})
        );
        assert_eq!(anthropic_tool_choice_to_openai(&json!("auto")), json!("auto"));
        assert_eq!(anthropic_tool_choice_to_openai(&json!("any")), json!("required"));
    }

    #[test]
    fn skips_null_fields_when_converting_request() {
        let body = Bytes::from_static(
            br#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":100,"temperature":null,"top_p":null,"stop_sequences":null,"stream":false}"#,
        );
        let payload = anthropic_to_openai(body, "target").unwrap();
        let value: Value = serde_json::from_slice(&payload).unwrap();
        assert!(value.get("temperature").is_none(), "temperature 不应透传 null");
        assert!(value.get("top_p").is_none(), "top_p 不应透传 null");
        assert!(value.get("stop").is_none(), "stop 不应透传 null");
    }

    #[test]
    fn converts_openai_error_to_anthropic_error_format() {
        let error_body = br#"{"error":{"message":"invalid api key","type":"authentication_error"}}"#;
        let converted = openai_error_to_anthropic_error(error_body);
        let value: Value = serde_json::from_str(&converted).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["error"]["type"], "api_error");
        assert_eq!(value["error"]["message"], "invalid api key");
    }

    #[test]
    fn converts_non_json_error_body_gracefully() {
        let converted = openai_error_to_anthropic_error(b"plain text error");
        let value: Value = serde_json::from_str(&converted).unwrap();
        assert_eq!(value["error"]["message"], "plain text error");
    }

    #[test]
    fn converts_anthropic_image_block_to_openai_image_url() {
        let block = json!({
            "type": "image",
            "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="}
        });
        let converted = anthropic_image_block_to_openai(&block).unwrap();
        assert_eq!(converted["type"], "image_url");
        assert_eq!(converted["image_url"]["url"], "data:image/png;base64,aGVsbG8=");
    }

    #[test]
    fn converts_image_block_in_user_message() {
        let body = Bytes::from(
            r#"{"messages":[{"role":"user","content":[{"type":"text","text":"看图"},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"AAAA"}}]}],"max_tokens":100,"stream":false}"#,
        );
        let payload = anthropic_to_openai(body, "target").unwrap();
        let value: Value = serde_json::from_slice(&payload).unwrap();
        let content = &value["messages"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/jpeg;base64,AAAA");
    }

    #[test]
    fn maps_non_streaming_reasoning_content_to_thinking_block() {
        let response = br#"{"id":"chatcmpl-1","model":"m","choices":[{"message":{"role":"assistant","content":"answer","reasoning_content":"think step by step"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#;
        let value = openai_json_to_anthropic_json(response).unwrap();
        assert_eq!(value["content"][0]["type"], "thinking");
        assert_eq!(value["content"][0]["thinking"], "think step by step");
        assert_eq!(value["content"][1]["type"], "text");
    }

    #[tokio::test]
    async fn converts_stream_delta_array_content() {
        let chunks = vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"text\",\"text\":\"Hi \"},{\"type\":\"text\",\"text\":\"there\"}]}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ];
        let stream = futures_util::stream::iter(chunks);
        let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None);
        pin_mut!(out);
        let mut merged = String::new();
        while let Some(item) = out.next().await {
            merged.push_str(&String::from_utf8_lossy(&item.unwrap()));
        }
        assert!(merged.contains(r#""text":"Hi there""#));
    }

    #[tokio::test]
    async fn converts_stream_reasoning_content_to_thinking_delta() {
        let chunks = vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"deep thought\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ];
        let stream = futures_util::stream::iter(chunks);
        let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None);
        pin_mut!(out);
        let mut merged = String::new();
        while let Some(item) = out.next().await {
            merged.push_str(&String::from_utf8_lossy(&item.unwrap()));
        }
        assert!(merged.contains(r#""type":"thinking""#));
        assert!(merged.contains(r#""thinking":"deep thought""#));
        assert!(merged.contains("thinking_delta"));
    }

    #[tokio::test]
    async fn converts_openai_sse_stream_with_tool_calls() {
        let chunks = vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ];
        let stream = futures_util::stream::iter(chunks);
        let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None);
        pin_mut!(out);
        let mut merged = String::new();
        while let Some(item) = out.next().await {
            merged.push_str(&String::from_utf8_lossy(&item.unwrap()));
        }
        assert!(merged.contains("event: message_start"));
        assert!(merged.contains(r#""type":"tool_use""#));
        assert!(merged.contains(r#""name":"bash""#));
        assert!(merged.contains("input_json_delta"));
        assert!(merged.contains(r#""stop_reason":"tool_use""#));
        assert!(merged.contains("event: message_stop"));
    }

    #[tokio::test]
    async fn converts_openai_sse_stream_to_anthropic() {
        let chunks = vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ];
        let stream = futures_util::stream::iter(chunks);
        let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None);
        pin_mut!(out);
        let mut merged = String::new();
        while let Some(item) = out.next().await {
            merged.push_str(&String::from_utf8_lossy(&item.unwrap()));
        }
        assert!(merged.contains("event: message_start"));
        assert!(merged.contains("text_delta"));
        assert!(merged.contains(r#""text":"Hi""#));
        assert!(merged.contains("event: message_stop"));
    }
}
