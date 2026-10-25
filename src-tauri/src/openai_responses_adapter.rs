use anyhow::Result;
use async_stream::stream;
use bytes::Bytes;
use futures_util::{pin_mut, Stream, StreamExt};
use reqwest::Response;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use tracing::{info, warn};

const CUSTOM_TOOL_INPUT_FIELD: &str = "input";

/// Chat 回落时保留的工具元数据，用于把 tool_calls 还原成 Codex 期望的 item 类型。
#[derive(Debug, Clone, Default)]
pub struct ChatToolMeta {
    /// 由 Responses `custom` 转成的 Chat function 名（如 exec / apply_patch）
    pub custom_names: HashSet<String>,
}

/// 将 OpenAI Responses 请求转为 Chat Completions（Codex → 仅 Chat 的上游）。
pub fn responses_to_chat(body: &Bytes, upstream_model: &str) -> Result<Bytes> {
    Ok(responses_to_chat_with_meta(body, upstream_model)?.0)
}

pub fn responses_to_chat_with_meta(
    body: &Bytes,
    upstream_model: &str,
) -> Result<(Bytes, ChatToolMeta)> {
    let value: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body)?
    };

    let mut meta = ChatToolMeta::default();
    let chat_tools = collect_chat_tools_from_responses(&value, &mut meta);

    let mut messages: Vec<Value> = input_to_messages(value.get("input"))
        .as_array()
        .cloned()
        .unwrap_or_default();

    if let Some(instr) = value
        .get("instructions")
        .and_then(|i| i.as_str())
        .filter(|s| !s.is_empty())
    {
        messages.insert(0, json!({ "role": "system", "content": instr }));
    }

    if messages.is_empty() {
        messages.push(json!({ "role": "user", "content": "ping" }));
    }

    let stream = value
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let mut out = json!({
        "model": upstream_model,
        "messages": messages,
        "stream": stream,
    });

    if let Some(obj) = value.as_object() {
        if let Some(out_obj) = out.as_object_mut() {
            for key in [
                "temperature",
                "max_tokens",
                "max_completion_tokens",
                "top_p",
                "tool_choice",
                "response_format",
                "parallel_tool_calls",
            ] {
                if let Some(v) = obj.get(key) {
                    out_obj.insert(key.to_string(), v.clone());
                }
            }
            if !out_obj.contains_key("max_tokens") {
                if let Some(v) = obj.get("max_completion_tokens") {
                    out_obj.insert("max_tokens".to_string(), v.clone());
                }
            }

            if chat_tools.is_empty() {
                // 严格上游：有 tool_choice 无 tools 会 400；空 tools 也不要发
                out_obj.remove("tool_choice");
                out_obj.remove("parallel_tool_calls");
                out_obj.remove("tools");
            } else {
                out_obj.insert("tools".to_string(), Value::Array(chat_tools.clone()));
            }

            // 流式时请求 usage 块，供控制台 Token 入/出统计
            if stream {
                out_obj.insert(
                    "stream_options".to_string(),
                    json!({ "include_usage": true }),
                );
            }
        }
    }

    let tool_names: Vec<&str> = chat_tools
        .iter()
        .filter_map(|t| t.pointer("/function/name").and_then(|n| n.as_str()))
        .collect();
    let has_apply_patch = tool_names.iter().any(|n| *n == "apply_patch");
    let has_exec_command = tool_names
        .iter()
        .any(|n| matches!(*n, "exec_command" | "exec" | "shell"));
    // 文件类工具名便于对照 Trae（write_file 等）vs Codex（apply_patch / exec_command）
    let fileish: Vec<&str> = tool_names
        .iter()
        .copied()
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            l.contains("patch")
                || l.contains("write")
                || l.contains("edit")
                || l.contains("file")
                || l.contains("exec")
                || l.contains("shell")
        })
        .collect();
    info!(
        upstream_model = upstream_model,
        message_count = messages.len(),
        tool_count = chat_tools.len(),
        custom_tools = meta.custom_names.len(),
        has_apply_patch,
        has_exec_command,
        fileish_tools = ?fileish,
        stream,
        "responses_to_chat converted request"
    );

    Ok((Bytes::from(serde_json::to_vec(&out)?), meta))
}

fn collect_chat_tools_from_responses(body: &Value, meta: &mut ChatToolMeta) -> Vec<Value> {
    let mut chat_tools = Vec::new();
    let mut seen = HashSet::new();

    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        for tool in tools {
            push_response_tool_as_chat(tool, &mut chat_tools, &mut seen, meta);
        }
    }
    if let Some(input) = body.get("input") {
        collect_input_declared_tools(input, &mut chat_tools, &mut seen, meta);
    }
    chat_tools
}

fn collect_input_declared_tools(
    value: &Value,
    chat_tools: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    meta: &mut ChatToolMeta,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_input_declared_tools(item, chat_tools, seen, meta);
            }
        }
        Value::Object(obj) => {
            let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(typ, "additional_tools" | "tool_search_output") {
                if let Some(tools) = obj.get("tools").and_then(|v| v.as_array()) {
                    for tool in tools {
                        push_response_tool_as_chat(tool, chat_tools, seen, meta);
                    }
                }
            }
            for child in obj.values() {
                collect_input_declared_tools(child, chat_tools, seen, meta);
            }
        }
        _ => {}
    }
}

fn push_response_tool_as_chat(
    tool: &Value,
    chat_tools: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    meta: &mut ChatToolMeta,
) {
    match tool.get("type").and_then(|v| v.as_str()) {
        Some("function") => {
            if let Some(chat) = responses_function_to_chat_tool(tool) {
                let name = chat
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !name.is_empty() && seen.insert(name) {
                    chat_tools.push(chat);
                }
            }
        }
        Some("custom") => {
            if let Some((name, chat)) = responses_custom_to_chat_tool(tool) {
                if seen.insert(name.clone()) {
                    meta.custom_names.insert(name);
                    chat_tools.push(chat);
                }
            }
        }
        Some("namespace") => {
            let children = tool
                .get("tools")
                .or_else(|| tool.get("children"))
                .and_then(|v| v.as_array());
            if let Some(children) = children {
                for child in children {
                    // Codex 把 exec/apply_patch 等 custom 挂在 functions namespace 下
                    push_response_tool_as_chat(child, chat_tools, seen, meta);
                }
            }
        }
        Some("tool_search") => {
            let name = "tool_search".to_string();
            if seen.insert(name.clone()) {
                chat_tools.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": tool.get("description").cloned().unwrap_or_else(|| {
                            json!("Search and load tools for the current task.")
                        }),
                        "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({
                            "type": "object",
                            "properties": {
                                "query": { "type": "string" }
                            },
                            "required": ["query"]
                        })),
                    }
                }));
            }
        }
        _ => {}
    }
}

fn responses_function_to_chat_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name").and_then(|v| v.as_str())?;
    if name.is_empty() {
        return None;
    }
    let description = tool
        .get("description")
        .cloned()
        .unwrap_or_else(|| json!(""));
    let parameters = tool
        .get("parameters")
        .or_else(|| tool.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    }))
}

fn responses_custom_to_chat_tool(tool: &Value) -> Option<(String, Value)> {
    let name = tool.get("name").and_then(|v| v.as_str())?.to_string();
    if name.is_empty() {
        return None;
    }
    let mut description = tool
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if let Ok(def) = serde_json::to_string(tool) {
        if !description.is_empty() {
            description.push_str("\n\n");
        }
        description.push_str("Original tool definition:\n");
        description.push_str(&def);
    }
    let chat = json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": {
                    CUSTOM_TOOL_INPUT_FIELD: {
                        "type": "string",
                        "description": "Raw string input for the original custom tool."
                    }
                },
                "required": [CUSTOM_TOOL_INPUT_FIELD]
            }
        }
    });
    Some((name, chat))
}

fn custom_tool_input_from_chat_arguments(arguments: &str) -> String {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) {
        if let Some(input) = map.get(CUSTOM_TOOL_INPUT_FIELD).and_then(|v| v.as_str()) {
            return input.to_string();
        }
        if let Some(input) = map.get(CUSTOM_TOOL_INPUT_FIELD) {
            return input.to_string();
        }
    }
    trimmed.to_string()
}

fn is_custom_tool_name(name: &str, meta: &ChatToolMeta) -> bool {
    if meta.custom_names.contains(name) {
        return true;
    }
    // 无 meta 时的兜底：Codex 常见 freeform 工具
    matches!(
        name,
        "exec" | "apply_patch" | "shell_command" | "write_stdin" | "shell"
    )
}

/// ???????? OpenAI Responses API??? Coding /api/coding/v3?/api/v3 ??
pub fn provider_supports_native_responses(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/').to_lowercase();
    let is_volc = base.contains("volces.com") || base.contains("volcengine");
    let has_api_path = base.contains("/api/coding")
        || base.ends_with("/api/v3")
        || base.contains("/coding/v3");
    is_volc && has_api_path
}

/// Codex ?? ark-code ??? Coding ?????? Base URL ??????? Responses
pub fn should_try_native_responses(base_url: &str, model: &str) -> bool {
    provider_supports_native_responses(base_url) || is_volcengine_coding_model(model)
}

pub fn is_volcengine_coding_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("ark-code") || m == "ark-code-latest"
}

/// ?? Responses ??????????? base?ark-code ??????? Coding Plan ??
pub fn native_responses_base_url(base_url: &str, model: &str) -> String {
    if provider_supports_native_responses(base_url) {
        base_url.trim().trim_end_matches('/').to_string()
    } else if is_volcengine_coding_model(model) {
        warn!(
            configured_base = base_url,
            model = model,
            "ark-code model detected but base_url is not volcengine coding/v3; using default Coding Plan endpoint"
        );
        "https://ark.cn-beijing.volces.com/api/coding/v3".to_string()
    } else {
        base_url.trim().trim_end_matches('/').to_string()
    }
}

/// ?? Responses ?????? upstream model??? Codex ?? tools/input ??
pub fn rewrite_responses_upstream_model(body: &Bytes, upstream_model: &str) -> Result<Bytes> {
    let mut value: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body)?
    };
    if let Some(obj) = value.as_object_mut() {
        obj.insert("model".to_string(), json!(upstream_model));
    }
    Ok(Bytes::from(serde_json::to_vec(&value)?))
}

fn output_value_to_string(output: &Value) -> String {
    match output {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn input_to_messages(input: Option<&Value>) -> Value {
    match input {
        None => json!([{"role": "user", "content": "ping"}]),
        Some(Value::String(text)) => json!([{"role": "user", "content": text}]),
        Some(Value::Array(items)) => {
            if items.is_empty() {
                json!([{"role": "user", "content": "ping"}])
            } else {
                let messages: Vec<Value> = items
                    .iter()
                    .filter_map(|item| normalize_response_item(item))
                    .collect();
                if messages.is_empty() {
                    json!([{"role": "user", "content": "ping"}])
                } else {
                    json!(messages)
                }
            }
        }
        Some(other) => json!([{"role": "user", "content": other.to_string()}]),
    }
}

/// Responses API ??? ? Chat Completions messages??? input_text / output_text?
fn normalize_response_item(item: &Value) -> Option<Value> {
    if let Some(typ) = item.get("type").and_then(|t| t.as_str()) {
        match typ {
            "message" | "input_message" => {
                let role = item
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user");
                let role = match role {
                    "developer" => "system",
                    other => other,
                };
                let content = item
                    .get("content")
                    .map(normalize_message_content)
                    .unwrap_or(json!(""));
                Some(json!({ "role": role, "content": content }))
            }
            "function_call_output" | "custom_tool_call_output" => {
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("call");
                let output = item
                    .get("output")
                    .map(output_value_to_string)
                    .unwrap_or_default();
                Some(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }))
            }
            "function_call" => {
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("call");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("function");
                let arguments = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                Some(json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": arguments },
                    }],
                }))
            }
            "custom_tool_call" => {
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("call");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("custom");
                let input = item.get("input").and_then(|i| i.as_str()).unwrap_or("");
                let arguments = serde_json::to_string(&json!({ "input": input }))
                    .unwrap_or_else(|_| input.to_string());
                Some(json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": arguments },
                    }],
                }))
            }
            // 工具声明载体：已在 collect_chat_tools 提起，不能当成 user 文本
            "item_reference" | "reasoning" | "additional_tools" | "tool_search_output" => None,
            _ => Some(json!({ "role": "user", "content": item.to_string() })),
        }
    } else if let Some(obj) = item.as_object() {
        let role = obj.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = obj
            .get("content")
            .map(normalize_message_content)
            .unwrap_or(json!(""));
        Some(json!({ "role": role, "content": content }))
    } else if let Some(s) = item.as_str() {
        Some(json!({ "role": "user", "content": s }))
    } else {
        Some(json!({ "role": "user", "content": item.to_string() }))
    }
}

fn normalize_message_content(content: &Value) -> Value {
    match content {
        Value::String(s) => json!(s),
        Value::Array(parts) => {
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|p| {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        Some(t.to_string())
                    } else if let Some(s) = p.as_str() {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                json!("")
            } else {
                json!(texts.join("\n"))
            }
        }
        other => other.clone(),
    }
}

/// ????Chat Completions JSON ? Responses API JSON
pub async fn chat_to_responses(response: Response, model: &str) -> Result<Bytes> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Ok(Bytes::from(body));
    }
    Ok(chat_json_to_responses_body(&body, model)?)
}

/// ?? HTTP 200 ? body ??? JSON????????
pub fn is_openai_compat_error_body(body: &str) -> bool {
    let value: Value = serde_json::from_str(body).unwrap_or(json!({}));
    if value.get("error").is_some() {
        return true;
    }
    if value.get("code").is_some()
        && value.get("choices").is_none()
        && value.get("output").is_none()
    {
        return true;
    }
    false
}

pub fn chat_json_to_responses_body(body: &str, model: &str) -> Result<Bytes> {
    chat_json_to_responses_body_with_meta(body, model, &ChatToolMeta::default())
}

pub fn chat_json_to_responses_body_with_meta(
    body: &str,
    model: &str,
    meta: &ChatToolMeta,
) -> Result<Bytes> {
    if is_openai_compat_error_body(body) {
        return Ok(Bytes::from(body.to_string()));
    }

    let chat: Value = serde_json::from_str(body).unwrap_or(json!({}));
    let message = chat
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .cloned()
        .unwrap_or(json!({}));

    let mut text = message
        .get("content")
        .and_then(content_value_to_text)
        .unwrap_or_default();
    let mut reasoning = String::new();
    for key in ["reasoning_content", "reasoning"] {
        if let Some(r) = message.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            reasoning = r.to_string();
            break;
        }
    }

    let id = chat
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("sugt-resp");

    let mut output: Vec<Value> = Vec::new();
    if !reasoning.is_empty() {
        output.push(json!({
            "type": "reasoning",
            "id": format!("rs_{id}"),
            "summary": [{ "type": "summary_text", "text": reasoning }],
        }));
    }
    // 只有推理没有正文时，兜底把推理放进 message，避免空白回复
    if text.is_empty() && !reasoning.is_empty() {
        warn!(
            reasoning_len = reasoning.len(),
            "codex non-stream: only reasoning_content present; also emitting as output_text fallback"
        );
        text = reasoning;
    }
    if !text.is_empty() {
        output.push(json!({
            "type": "message",
            "id": format!("msg_{id}"),
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text }],
        }));
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for (idx, call) in tool_calls.iter().enumerate() {
            let call_id = call
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("call")
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("function")
                .to_string();
            let arguments = call
                .pointer("/function/arguments")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "{}".to_string());
            if is_custom_tool_name(&name, meta) {
                output.push(json!({
                    "type": "custom_tool_call",
                    "id": format!("ctc_{call_id}_{idx}"),
                    "call_id": call_id,
                    "name": name,
                    "input": custom_tool_input_from_chat_arguments(&arguments),
                    "status": "completed",
                }));
            } else {
                output.push(json!({
                    "type": "function_call",
                    "id": format!("fc_{call_id}_{idx}"),
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments,
                    "status": "completed",
                }));
            }
        }
    }

    if output.is_empty() {
        output.push(json!({
            "type": "message",
            "id": format!("msg_{id}"),
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": "" }],
        }));
    }

    let usage = chat
        .get("usage")
        .map(normalize_chat_usage_to_responses)
        .unwrap_or_else(|| responses_usage(0, 0));
    let out = json!({
        "id": id,
        "object": "response",
        "status": "completed",
        "model": model,
        "output": output,
        "usage": usage,
    });
    Ok(Bytes::from(serde_json::to_vec(&out)?))
}

/// Codex / Responses API 要求 usage 含 `total_tokens`（缺了会 ResponseCompleted 解析失败）。
fn responses_usage(input_tokens: u64, output_tokens: u64) -> Value {
    json!({
        "input_tokens": input_tokens,
        "input_tokens_details": { "cached_tokens": 0 },
        "output_tokens": output_tokens,
        "output_tokens_details": { "reasoning_tokens": 0 },
        "total_tokens": input_tokens.saturating_add(output_tokens),
    })
}

fn normalize_chat_usage_to_responses(usage: &Value) -> Value {
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    responses_usage(input, output)
}

struct SseMapState {
    response_id: String,
    item_id: String,
    reasoning_id: String,
    model: String,
    text_buffer: String,
    reasoning_buffer: String,
    sequence: u32,
    completed: bool,
    created_sent: bool,
    message_prelude_sent: bool,
    reasoning_prelude_sent: bool,
    tool_calls: HashMap<usize, ToolCallAccum>,
    upstream_samples: Vec<String>,
    /// 下一个可用 output_index（reasoning / message / tool 递增分配）
    output_index: u32,
    message_output_index: u32,
    reasoning_output_index: u32,
    tool_meta: ChatToolMeta,
    /// 上游 Chat SSE usage（stream_options.include_usage）
    input_tokens: u64,
    output_tokens: u64,
    /// 无上游 usage 时的入站粗估（来自请求体）
    estimated_input_tokens: u64,
    /// finalize 时是否走了字数兜底（上游未返回 usage）
    usage_estimated: bool,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
    item_added: bool,
    output_index: u32,
    is_custom: bool,
}

fn new_ids() -> (String, String, String) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    (
        format!("sugt-resp-{millis}"),
        format!("msg_{millis}"),
        format!("rs_{millis}"),
    )
}

fn format_sse_data(value: &Value) -> String {
    let typ = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ.is_empty() {
        format!("data: {}\n\n", value)
    } else {
        format!("event: {}\ndata: {}\n\n", typ, value)
    }
}

fn created_event(state: &SseMapState) -> String {
    let created = json!({
        "type": "response.created",
        "sequence_number": state.sequence,
        "response": {
            "id": state.response_id,
            "object": "response",
            "status": "in_progress",
            "model": state.model,
            "output": [],
        }
    });
    format_sse_data(&created)
}

fn message_prelude_events(state: &mut SseMapState) -> String {
    state.message_output_index = state.output_index;
    state.output_index += 1;
    let item_added = json!({
        "type": "response.output_item.added",
        "sequence_number": state.sequence,
        "output_index": state.message_output_index,
        "item": {
            "type": "message",
            "id": state.item_id,
            "role": "assistant",
            "status": "in_progress",
            "content": [],
        }
    });
    state.sequence += 1;
    let part_added = json!({
        "type": "response.content_part.added",
        "sequence_number": state.sequence,
        "item_id": state.item_id,
        "output_index": state.message_output_index,
        "content_index": 0,
        "part": { "type": "output_text", "text": "" },
    });
    state.sequence += 1;
    format_sse_data(&item_added) + &format_sse_data(&part_added)
}

fn ensure_message_prelude(state: &mut SseMapState) -> String {
    if state.message_prelude_sent {
        return String::new();
    }
    state.message_prelude_sent = true;
    message_prelude_events(state)
}

fn reasoning_prelude_events(state: &mut SseMapState) -> String {
    state.reasoning_output_index = state.output_index;
    state.output_index += 1;
    let item_added = json!({
        "type": "response.output_item.added",
        "sequence_number": state.sequence,
        "output_index": state.reasoning_output_index,
        "item": {
            "type": "reasoning",
            "id": state.reasoning_id,
            "summary": [],
        }
    });
    state.sequence += 1;
    let part_added = json!({
        "type": "response.reasoning_summary_part.added",
        "sequence_number": state.sequence,
        "item_id": state.reasoning_id,
        "output_index": state.reasoning_output_index,
        "summary_index": 0,
        "part": { "type": "summary_text", "text": "" },
    });
    state.sequence += 1;
    format_sse_data(&item_added) + &format_sse_data(&part_added)
}

fn ensure_reasoning_prelude(state: &mut SseMapState) -> String {
    if state.reasoning_prelude_sent {
        return String::new();
    }
    state.reasoning_prelude_sent = true;
    reasoning_prelude_events(state)
}

fn apply_usage_fallback(state: &mut SseMapState) {
    let had_upstream = state.input_tokens > 0 || state.output_tokens > 0;
    if state.output_tokens == 0 {
        let mut out = String::new();
        out.push_str(&state.text_buffer);
        out.push_str(&state.reasoning_buffer);
        for tool in state.tool_calls.values() {
            out.push_str(&tool.name);
            out.push_str(&tool.arguments);
        }
        if !out.is_empty() {
            state.output_tokens = crate::quick_chat::estimate_output_tokens(&out);
        }
    }
    if state.input_tokens == 0 && state.estimated_input_tokens > 0 {
        state.input_tokens = state.estimated_input_tokens;
    }
    state.usage_estimated = !had_upstream
        && (state.input_tokens > 0 || state.output_tokens > 0);
}

fn finalize_events(state: &mut SseMapState) -> String {
    if state.completed {
        return String::new();
    }
    state.completed = true;
    apply_usage_fallback(state);
    let mut out = String::new();
    let mut completed_output: Vec<Value> = Vec::new();

    // 仅有推理、没有正文/工具时：把推理兜底进正文，避免 Codex 空白气泡
    if state.text_buffer.is_empty()
        && state.tool_calls.is_empty()
        && !state.reasoning_buffer.is_empty()
    {
        warn!(
            reasoning_len = state.reasoning_buffer.len(),
            "codex stream: upstream only returned reasoning_content; promoting to output_text as fallback"
        );
        out += &ensure_message_prelude(state);
        state.text_buffer = state.reasoning_buffer.clone();
        state.sequence += 1;
        let event = json!({
            "type": "response.output_text.delta",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": state.message_output_index,
            "content_index": 0,
            "delta": state.text_buffer.clone(),
        });
        out += &format_sse_data(&event);
    }

    if state.reasoning_prelude_sent || !state.reasoning_buffer.is_empty() {
        if !state.reasoning_prelude_sent {
            out += &ensure_reasoning_prelude(state);
        }
        let summary_done = json!({
            "type": "response.reasoning_summary_text.done",
            "sequence_number": state.sequence,
            "item_id": state.reasoning_id,
            "output_index": state.reasoning_output_index,
            "summary_index": 0,
            "text": state.reasoning_buffer,
        });
        state.sequence += 1;
        out += &format_sse_data(&summary_done);

        let part_done = json!({
            "type": "response.reasoning_summary_part.done",
            "sequence_number": state.sequence,
            "item_id": state.reasoning_id,
            "output_index": state.reasoning_output_index,
            "summary_index": 0,
            "part": { "type": "summary_text", "text": state.reasoning_buffer },
        });
        state.sequence += 1;
        out += &format_sse_data(&part_done);

        let item = json!({
            "type": "reasoning",
            "id": state.reasoning_id,
            "summary": [{
                "type": "summary_text",
                "text": state.reasoning_buffer,
            }],
        });
        let item_done = json!({
            "type": "response.output_item.done",
            "sequence_number": state.sequence,
            "output_index": state.reasoning_output_index,
            "item": item.clone(),
        });
        state.sequence += 1;
        out += &format_sse_data(&item_done);
        completed_output.push(item);
    }

    if state.message_prelude_sent || !state.text_buffer.is_empty() {
        let text_done = json!({
            "type": "response.output_text.done",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": state.message_output_index,
            "content_index": 0,
            "text": state.text_buffer,
        });
        state.sequence += 1;
        out += &format_sse_data(&text_done);

        let part_done = json!({
            "type": "response.content_part.done",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": state.message_output_index,
            "content_index": 0,
            "part": { "type": "output_text", "text": state.text_buffer },
        });
        state.sequence += 1;
        out += &format_sse_data(&part_done);

        let item_done = json!({
            "type": "response.output_item.done",
            "sequence_number": state.sequence,
            "output_index": state.message_output_index,
            "item": {
                "type": "message",
                "id": state.item_id,
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": state.text_buffer,
                }],
            }
        });
        state.sequence += 1;
        out += &format_sse_data(&item_done);
        completed_output.push(json!({
            "type": "message",
            "id": state.item_id,
            "role": "assistant",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": state.text_buffer,
            }],
        }));
    }

    let mut tools: Vec<&ToolCallAccum> = state.tool_calls.values().collect();
    tools.sort_by_key(|t| t.output_index);
    for tool in tools {
        let item = if tool.is_custom {
            json!({
                "type": "custom_tool_call",
                "id": format!("ctc_{}", tool.id),
                "call_id": tool.id,
                "name": tool.name,
                "input": custom_tool_input_from_chat_arguments(&tool.arguments),
                "status": "completed",
            })
        } else {
            json!({
                "type": "function_call",
                "id": format!("fc_{}", tool.id),
                "call_id": tool.id,
                "name": tool.name,
                "arguments": tool.arguments,
                "status": "completed",
            })
        };
        let item_done = json!({
            "type": "response.output_item.done",
            "sequence_number": state.sequence,
            "output_index": tool.output_index,
            "item": item.clone(),
        });
        state.sequence += 1;
        out += &format_sse_data(&item_done);
        completed_output.push(item);
    }

    let usage = responses_usage(state.input_tokens, state.output_tokens);
    let response_obj = json!({
        "id": state.response_id,
        "object": "response",
        "status": "completed",
        "model": state.model,
        "output": completed_output,
        "usage": usage.clone(),
    });
    let completed = json!({
        "type": "response.completed",
        "sequence_number": state.sequence,
        "response": response_obj,
        // 顶层也放一份，便于网关 parse_usage_from_bytes 直接拾取
        "usage": usage,
    });
    state.sequence += 1;
    out += &format_sse_data(&completed);
    out += "data: [DONE]\n\n";
    out
}

fn failed_events(state: &mut SseMapState, message: &str) -> String {
    if state.completed {
        return String::new();
    }
    state.completed = true;
    let failed = json!({
        "type": "response.failed",
        "sequence_number": state.sequence,
        "response": {
            "id": state.response_id,
            "object": "response",
            "status": "failed",
            "model": state.model,
            "error": { "message": message },
        }
    });
    format_sse_data(&failed) + "data: [DONE]\n\n"
}

fn content_value_to_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

fn extract_chat_message_text(parsed: &Value) -> Option<String> {
    let choice = parsed.get("choices").and_then(|c| c.get(0))?;
    if let Some(msg) = choice.get("message") {
        if let Some(text) = msg.get("content").and_then(content_value_to_text) {
            return Some(text);
        }
    }
    extract_content_delta(parsed)
}

fn extract_chat_message_reasoning(parsed: &Value) -> Option<String> {
    let choice = parsed.get("choices").and_then(|c| c.get(0))?;
    if let Some(msg) = choice.get("message") {
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = msg.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                return Some(text.to_string());
            }
        }
    }
    extract_reasoning_delta(parsed)
}

/// 仅提取可见正文（不含 reasoning_*）。
fn extract_content_delta(parsed: &Value) -> Option<String> {
    let choice = parsed.get("choices").and_then(|c| c.get(0))?;

    if let Some(delta_obj) = choice.get("delta") {
        if let Some(text) = delta_obj.get("content").and_then(content_value_to_text) {
            return Some(text);
        }
        if let Some(s) = delta_obj.get("text").and_then(|c| c.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }

    if let Some(s) = choice.get("text").and_then(|c| c.as_str()) {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }

    if let Some(msg) = choice.get("message") {
        if let Some(text) = msg.get("content").and_then(content_value_to_text) {
            return Some(text);
        }
    }

    None
}

/// 仅提取推理字段（reasoning_content / reasoning）。
fn extract_reasoning_delta(parsed: &Value) -> Option<String> {
    let choice = parsed.get("choices").and_then(|c| c.get(0))?;
    let delta_obj = choice.get("delta")?;
    for key in ["reasoning_content", "reasoning"] {
        if let Some(s) = delta_obj.get(key).and_then(|c| c.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}


fn upstream_error_message(parsed: &Value) -> Option<String> {
    if let Some(err) = parsed.get("error") {
        if let Some(msg) = err.get("message").and_then(|m| m.as_str()) {
            return Some(msg.to_string());
        }
        return Some(err.to_string());
    }
    if let Some(msg) = parsed.get("message").and_then(|m| m.as_str()) {
        if parsed.get("choices").is_none() {
            return Some(msg.to_string());
        }
    }
    None
}

fn process_tool_call_delta(state: &mut SseMapState, choice: &Value) -> String {
    let delta = choice.get("delta").unwrap_or(choice);
    let tool_calls = delta
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| {
            choice
                .get("message")
                .and_then(|m| m.get("tool_calls"))
                .and_then(|v| v.as_array())
                .cloned()
        });
    if tool_calls.is_none() {
        return String::new();
    }

    let mut out = String::new();
    for call in tool_calls.unwrap_or_default() {
        let index = call
            .get("index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let entry = state.tool_calls.entry(index).or_insert_with(|| {
            let output_index = if state.message_prelude_sent || !state.text_buffer.is_empty() {
                state.output_index + 1
            } else {
                state.output_index
            };
            state.output_index = output_index + 1;
            ToolCallAccum {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
                item_added: false,
                output_index,
                is_custom: false,
            }
        });

        if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
            entry.id = id.to_string();
        }
        if let Some(name) = call
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
        {
            entry.name = name.to_string();
            entry.is_custom = is_custom_tool_name(name, &state.tool_meta);
        }
        if let Some(args) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            entry.arguments.push_str(args);
        }

        if entry.id.is_empty() {
            entry.id = format!("call_{}_{}", state.response_id, index);
        }

        if !entry.item_added && !entry.name.is_empty() {
            entry.item_added = true;
            state.sequence += 1;
            let item = if entry.is_custom {
                json!({
                    "type": "custom_tool_call",
                    "id": format!("ctc_{}", entry.id),
                    "call_id": entry.id,
                    "name": entry.name,
                    "input": "",
                    "status": "in_progress",
                })
            } else {
                json!({
                    "type": "function_call",
                    "id": format!("fc_{}", entry.id),
                    "call_id": entry.id,
                    "name": entry.name,
                    "arguments": "",
                    "status": "in_progress",
                })
            };
            let item_added = json!({
                "type": "response.output_item.added",
                "sequence_number": state.sequence,
                "output_index": entry.output_index,
                "item": item,
            });
            out += &format_sse_data(&item_added);
        }

        if let Some(args) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            // custom 工具的 Chat 参数是 JSON 分片，不能逐片 parse；完整 input 在 item.done 给出
            if !args.is_empty() && !entry.is_custom {
                state.sequence += 1;
                let event = json!({
                    "type": "response.function_call_arguments.delta",
                    "sequence_number": state.sequence,
                    "item_id": format!("fc_{}", entry.id),
                    "output_index": entry.output_index,
                    "delta": args,
                });
                out += &format_sse_data(&event);
            }
        }
    }
    out
}

fn upstream_finished(parsed: &Value) -> bool {
    let finish = parsed
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("finish_reason"))
        .and_then(|f| f.as_str());
    finish.is_some() && finish != Some("null")
}

fn record_upstream_sample(state: &mut SseMapState, payload: &str) {
    if state.upstream_samples.len() >= 5 {
        return;
    }
    let sample = if payload.len() > 500 {
        format!("{}...", &payload[..500])
    } else {
        payload.to_string()
    };
    state.upstream_samples.push(sample);
}

fn capture_usage_from_chat_payload(state: &mut SseMapState, parsed: &Value) {
    let Some(usage) = parsed.get("usage") else {
        return;
    };
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
    if input > 0 || output > 0 {
        state.input_tokens = input;
        state.output_tokens = output;
    }
}

fn process_upstream_payload(state: &mut SseMapState, payload: &str) -> String {
    if payload == "[DONE]" {
        return finalize_events(state);
    }

    record_upstream_sample(state, payload);

    let parsed: Value = serde_json::from_str(payload).unwrap_or(json!({}));
    if let Some(err) = upstream_error_message(&parsed) {
        warn!(error = %err, "upstream SSE returned error JSON");
        return failed_events(state, &err);
    }

    capture_usage_from_chat_payload(state, &parsed);

    let mut out = String::new();

    if let Some(choice) = parsed.get("choices").and_then(|c| c.get(0)) {
        out += &process_tool_call_delta(state, choice);
    }

    // 推理先于正文：单独走 reasoning item，不要折进 output_text
    if let Some(delta) = extract_reasoning_delta(&parsed) {
        out += &ensure_reasoning_prelude(state);
        state.reasoning_buffer += &delta;
        state.sequence += 1;
        let event = json!({
            "type": "response.reasoning_summary_text.delta",
            "sequence_number": state.sequence,
            "item_id": state.reasoning_id,
            "output_index": state.reasoning_output_index,
            "summary_index": 0,
            "delta": delta,
        });
        out += &format_sse_data(&event);
    }

    if let Some(delta) = extract_content_delta(&parsed) {
        out += &ensure_message_prelude(state);
        state.text_buffer += &delta;
        state.sequence += 1;
        let event = json!({
            "type": "response.output_text.delta",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": state.message_output_index,
            "content_index": 0,
            "delta": delta,
        });
        out += &format_sse_data(&event);
    }

    if upstream_finished(&parsed) {
        // 只补全文，不在此 finalize：OpenAI 常在 finish_reason 之后才发 usage 块
        if state.reasoning_buffer.is_empty() {
            if let Some(text) = extract_chat_message_reasoning(&parsed) {
                if !text.is_empty() {
                    out += &ensure_reasoning_prelude(state);
                    state.reasoning_buffer = text.clone();
                    state.sequence += 1;
                    let event = json!({
                        "type": "response.reasoning_summary_text.delta",
                        "sequence_number": state.sequence,
                        "item_id": state.reasoning_id,
                        "output_index": state.reasoning_output_index,
                        "summary_index": 0,
                        "delta": text,
                    });
                    out += &format_sse_data(&event);
                }
            }
        }
        if state.text_buffer.is_empty() {
            if let Some(text) = extract_chat_message_text(&parsed) {
                if !text.is_empty() {
                    out += &ensure_message_prelude(state);
                    state.text_buffer = text;
                    state.sequence += 1;
                    let event = json!({
                        "type": "response.output_text.delta",
                        "sequence_number": state.sequence,
                        "item_id": state.item_id,
                        "output_index": state.message_output_index,
                        "content_index": 0,
                        "delta": state.text_buffer.clone(),
                    });
                    out += &format_sse_data(&event);
                }
            }
        }
    }

    out
}

fn process_upstream_line(state: &mut SseMapState, line: &str) -> String {
    let line = line.trim();
    if line.is_empty() || line.starts_with("event:") || line == ":" {
        return String::new();
    }

    let payload = if let Some(rest) = line.strip_prefix("data:") {
        rest.trim()
    } else if line.starts_with('{') {
        line
    } else {
        return String::new();
    };

    process_upstream_payload(state, payload)
}

fn drain_remainder(state: &mut SseMapState, parser: &mut SseLineParser) -> String {
    let remainder = parser.buffer.trim().to_string();
    if remainder.is_empty() {
        return String::new();
    }
    parser.buffer.clear();
    if remainder.starts_with('{') {
        return process_upstream_payload(state, &remainder);
    }
    process_upstream_line(state, &remainder)
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

/// 将 OpenAI Chat SSE 转为 Responses SSE（Codex 消费）。
pub fn chat_sse_to_responses_sse(
    input: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    model: String,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    chat_sse_to_responses_sse_with_meta(input, model, ChatToolMeta::default(), 0)
}

pub fn chat_sse_to_responses_sse_with_meta(
    input: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    model: String,
    tool_meta: ChatToolMeta,
    estimated_input_tokens: u64,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    Box::pin(stream! {
        let (response_id, item_id, reasoning_id) = new_ids();
        let mut state = SseMapState {
            response_id,
            item_id,
            reasoning_id,
            model,
            text_buffer: String::new(),
            reasoning_buffer: String::new(),
            sequence: 0,
            completed: false,
            created_sent: false,
            message_prelude_sent: false,
            reasoning_prelude_sent: false,
            tool_calls: HashMap::new(),
            upstream_samples: Vec::new(),
            output_index: 0,
            message_output_index: 0,
            reasoning_output_index: 0,
            tool_meta,
            input_tokens: 0,
            output_tokens: 0,
            estimated_input_tokens,
            usage_estimated: false,
        };
        let mut parser = SseLineParser::new();

        state.created_sent = true;
        state.sequence = 1;
        yield Ok(Bytes::from(created_event(&state)));

        pin_mut!(input);
        while let Some(chunk) = input.next().await {
            let chunk = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    yield Err(std::io::Error::other(err));
                    return;
                }
            };
            for line in parser.push(&chunk) {
                let mapped = process_upstream_line(&mut state, &line);
                if !mapped.is_empty() {
                    yield Ok(Bytes::from(mapped));
                }
            }
        }

        let remainder = drain_remainder(&mut state, &mut parser);
        if !remainder.is_empty() {
            yield Ok(Bytes::from(remainder));
        }

        if !state.completed {
            let tail = finalize_events(&mut state);
            if !tail.is_empty() {
                yield Ok(Bytes::from(tail));
            }
        }

        let content_len = state.text_buffer.len();
        let reasoning_len = state.reasoning_buffer.len();
        let tool_calls = state.tool_calls.len();
        let input_tokens = state.input_tokens;
        let output_tokens = state.output_tokens;
        let usage_estimated = state.usage_estimated;
        if content_len == 0 && tool_calls == 0 {
            warn!(
                reasoning_len,
                input_tokens,
                output_tokens,
                usage_estimated,
                samples = ?state.upstream_samples,
                "codex chat stream: no content/tool_calls (reasoning may have been promoted); check upstream"
            );
        } else {
            info!(
                content_len,
                reasoning_len,
                tool_calls,
                input_tokens,
                output_tokens,
                usage_estimated,
                "codex responses stream completed"
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{stream, StreamExt};

    #[test]
    fn ark_code_model_triggers_native_even_with_wrong_base() {
        assert!(should_try_native_responses(
            "https://api-inference.modelscope.cn/v1",
            "ark-code-latest"
        ));
        let base = native_responses_base_url(
            "https://api-inference.modelscope.cn/v1",
            "ark-code-latest",
        );
        assert_eq!(base, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn volcengine_coding_supports_native_responses() {
        assert!(provider_supports_native_responses(
            "https://ark.cn-beijing.volces.com/api/coding/v3"
        ));
        assert!(!provider_supports_native_responses(
            "https://api-inference.modelscope.cn/v1"
        ));
    }

    #[test]
    fn converts_string_input() {
        let body = Bytes::from(r#"{"model":"gpt-4","input":"hi","stream":false}"#);
        let out = responses_to_chat(&body, "upstream-model").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "upstream-model");
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn converts_responses_input_text_array() {
        let body = Bytes::from(
            r#"{"model":"gpt-4","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello codex"}]}]}"#,
        );
        let out = responses_to_chat(&body, "upstream-model").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["messages"][0]["content"], "hello codex");
        assert_eq!(v["model"], "upstream-model");
    }

    #[test]
    fn includes_instructions_as_system() {
        let body = Bytes::from(r#"{"input":"hi","instructions":"be helpful","stream":false}"#);
        let out = responses_to_chat(&body, "upstream-model").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][0]["content"], "be helpful");
    }

    #[test]
    fn sse_prelude_includes_output_item_added() {
        let (response_id, item_id, reasoning_id) = new_ids();
        let mut state = SseMapState {
            response_id,
            item_id,
            reasoning_id,
            model: "test-model".to_string(),
            text_buffer: String::new(),
            reasoning_buffer: String::new(),
            sequence: 1,
            completed: false,
            created_sent: true,
            message_prelude_sent: false,
            reasoning_prelude_sent: false,
            tool_calls: HashMap::new(),
            upstream_samples: Vec::new(),
            output_index: 0,
            message_output_index: 0,
            reasoning_output_index: 0,
            tool_meta: ChatToolMeta::default(),
            input_tokens: 0,
            output_tokens: 0,
            estimated_input_tokens: 0,
            usage_estimated: false,
        };
        let prelude = created_event(&state) + &ensure_message_prelude(&mut state);
        assert!(prelude.contains("response.created"));
        assert!(prelude.contains("event: response.output_item.added"));
        assert!(prelude.contains("response.content_part.added"));
    }

    #[tokio::test]
    async fn sse_stream_emits_full_codex_sequence() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("response.output_item.added"));
        assert!(all.contains("response.output_text.delta"));
        assert!(all.contains("\"delta\":\"Hi\""));
        assert!(all.contains("item_id"));
        assert!(all.contains("output_index"));
        assert!(all.contains("response.output_text.done"));
        assert!(all.contains("response.completed"));
        assert!(all.contains("[DONE]"));
    }

    #[test]
    fn stream_requests_include_usage() {
        let body = Bytes::from(r#"{"model":"gpt-4","input":"hi","stream":true}"#);
        let out = responses_to_chat(&body, "upstream-model").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stream"], true);
        assert_eq!(v["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn sse_stream_forwards_usage_into_completed() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("\"input_tokens\":12"));
        assert!(all.contains("\"output_tokens\":3"));
        assert!(all.contains("\"total_tokens\":15"));
        assert!(all.contains("response.completed"));
    }

    #[test]
    fn responses_usage_includes_total_tokens() {
        let u = responses_usage(10, 5);
        assert_eq!(u["input_tokens"], 10);
        assert_eq!(u["output_tokens"], 5);
        assert_eq!(u["total_tokens"], 15);
        assert!(u.get("input_tokens_details").is_some());
        assert!(u.get("output_tokens_details").is_some());
    }

    #[tokio::test]
    async fn sse_stream_captures_usage_after_finish_reason() {
        // OpenAI 常见顺序：finish_reason → usage → [DONE]
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7}}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("\"input_tokens\":100"), "usage after finish must be kept: {all}");
        assert!(all.contains("\"output_tokens\":7"), "usage after finish must be kept: {all}");
        assert!(all.contains("\"total_tokens\":107"), "Codex requires total_tokens: {all}");
        // event + data 各带一次 type；payload 里 "type":"response.completed" 应只一次
        assert_eq!(all.matches("\"type\":\"response.completed\"").count(), 1);
    }

    #[tokio::test]
    async fn sse_stream_estimates_usage_when_upstream_omits_it() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello world\"}}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse_with_meta(
            upstream,
            "m".to_string(),
            ChatToolMeta::default(),
            42,
        );
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("\"input_tokens\":42"), "fallback input: {all}");
        assert!(all.contains("\"output_tokens\":"), "fallback output: {all}");
        assert!(!all.contains("\"output_tokens\":0"), "output should be estimated: {all}");
    }

    #[tokio::test]
    async fn sse_stream_handles_split_chunks() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from("data: {\"choices\":[{\"delta\":{\"con")),
            Ok(Bytes::from("tent\":\"Split\"}}]}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("\"delta\":\"Split\""));
        assert!(all.contains("response.completed"));
    }

    #[tokio::test]
    async fn sse_stream_parses_non_sse_json_body() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"Full reply"},"finish_reason":"stop"}]}"#;
        let upstream = stream::iter(vec![Ok(Bytes::from(body))]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("response.output_text.delta"));
        assert!(all.contains("\"delta\":\"Full reply\""));
        assert!(all.contains("response.completed"));
    }

    #[test]
    fn extracts_reasoning_content_delta() {
        let parsed = json!({"choices":[{"delta":{"reasoning_content":"think"}}]});
        assert_eq!(extract_reasoning_delta(&parsed), Some("think".to_string()));
        assert_eq!(extract_content_delta(&parsed), None);
    }

    #[test]
    fn content_preferred_over_reasoning_in_combined_delta() {
        let parsed = json!({"choices":[{"delta":{"content":"hi","reasoning_content":"think"}}]});
        assert_eq!(extract_content_delta(&parsed), Some("hi".to_string()));
        assert_eq!(extract_reasoning_delta(&parsed), Some("think".to_string()));
    }

    #[test]
    fn lifts_additional_tools_into_chat_tools() {
        let body = Bytes::from(
            r#"{
              "model":"gpt-4",
              "stream":false,
              "tool_choice":"auto",
              "input":[
                {
                  "type":"additional_tools",
                  "id":"at_1",
                  "role":"developer",
                  "tools":[
                    {
                      "type":"namespace",
                      "name":"functions",
                      "tools":[
                        {"type":"custom","name":"exec","description":"Run a command"},
                        {"type":"function","name":"get_weather","description":"weather","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}
                      ]
                    }
                  ]
                },
                {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}
              ]
            }"#,
        );
        let (out, meta) = responses_to_chat_with_meta(&body, "upstream").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        let tools = v["tools"].as_array().expect("tools present");
        assert_eq!(tools.len(), 2);
        assert!(meta.custom_names.contains("exec"));
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.pointer("/function/name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"exec"));
        assert!(names.contains(&"get_weather"));
        // additional_tools 不得变成 user 文本消息
        let msgs = v["messages"].as_array().unwrap();
        assert!(msgs.iter().all(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(|s| !s.contains("additional_tools"))
                .unwrap_or(true)
        }));
        assert_eq!(msgs.last().unwrap()["content"], "hi");
    }

    #[test]
    fn omits_tool_choice_when_no_tools() {
        let body = Bytes::from(
            r#"{"model":"gpt-4","tool_choice":"auto","parallel_tool_calls":true,"input":"hi"}"#,
        );
        let (out, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("tools").is_none());
        assert!(v.get("tool_choice").is_none());
        assert!(v.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn non_stream_falls_back_reasoning_content() {
        let body = r#"{"id":"c1","choices":[{"message":{"role":"assistant","content":"","reasoning_content":"only think"},"finish_reason":"stop"}]}"#;
        let out = chat_json_to_responses_body(body, "m").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["output"][0]["type"], "reasoning");
        assert_eq!(v["output"][0]["summary"][0]["text"], "only think");
        assert_eq!(v["output"][1]["type"], "message");
        assert_eq!(v["output"][1]["content"][0]["text"], "only think");
    }

    #[test]
    fn non_stream_keeps_reasoning_and_content_separate() {
        let body = r#"{"id":"c1","choices":[{"message":{"role":"assistant","content":"你好","reasoning_content":"thinking..."},"finish_reason":"stop"}]}"#;
        let out = chat_json_to_responses_body(body, "m").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["output"][0]["type"], "reasoning");
        assert_eq!(v["output"][0]["summary"][0]["text"], "thinking...");
        assert_eq!(v["output"][1]["type"], "message");
        assert_eq!(v["output"][1]["content"][0]["text"], "你好");
    }

    #[test]
    fn non_stream_maps_custom_tool_call() {
        let body = r#"{"id":"c1","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"exec","arguments":"{\"input\":\"ls\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let mut meta = ChatToolMeta::default();
        meta.custom_names.insert("exec".into());
        let out = chat_json_to_responses_body_with_meta(body, "m", &meta).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["output"][0]["type"], "custom_tool_call");
        assert_eq!(v["output"][0]["name"], "exec");
        assert_eq!(v["output"][0]["input"], "ls");
    }

    #[tokio::test]
    async fn sse_stream_maps_reasoning_to_reasoning_summary() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("response.reasoning_summary_text.delta"));
        assert!(all.contains("\"delta\":\"think\""));
        assert!(all.contains("response.output_text.delta"));
        assert!(all.contains("\"delta\":\"hi\""));
        // 有正文时，推理不得被当成唯一/混进正文 item 的唯一来源
        assert!(all.contains("\"type\":\"reasoning\""));
        assert!(all.contains("response.completed"));
    }

    #[tokio::test]
    async fn sse_stream_reasoning_only_promotes_to_output_text() {
        let upstream = stream::iter(vec![
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"only think\"}}]}\n\n",
            )),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
        let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
        let all = chunks
            .iter()
            .map(|b| String::from_utf8_lossy(b))
            .collect::<Vec<_>>()
            .join("");
        assert!(all.contains("response.reasoning_summary_text.delta"));
        assert!(all.contains("response.output_text.delta"));
        assert!(all.contains("only think"));
        assert!(all.contains("response.completed"));
    }
}
