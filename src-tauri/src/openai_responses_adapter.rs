use anyhow::Result;
use async_stream::stream;
use bytes::Bytes;
use futures_util::{pin_mut, Stream, StreamExt};
use reqwest::Response;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use tracing::{debug, warn};

/// ? OpenAI Responses API ????? Chat Completions ???
pub fn responses_to_chat(body: &Bytes, upstream_model: &str) -> Result<Bytes> {
    let value: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body)?
    };

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
            // Codex Responses ? tools ?? custom/shell??? Chat ?????????????
            if obj.get("tools").is_some() {
                warn!(
                    tool_count = obj.get("tools").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0),
                    "stripping Codex tools for chat/completions upstream compatibility"
                );
                out_obj.remove("tools");
                out_obj.remove("tool_choice");
            }
        }
    }

    debug!(
        upstream_model = upstream_model,
        message_count = messages.len(),
        stream,
        "responses_to_chat converted request"
    );

    Ok(Bytes::from(serde_json::to_vec(&out)?))
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
            "item_reference" | "reasoning" => None,
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
    if is_openai_compat_error_body(body) {
        return Ok(Bytes::from(body.to_string()));
    }

    let chat: Value = serde_json::from_str(body).unwrap_or(json!({}));
    let text = chat
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let id = chat
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("sugt-resp");
    let out = json!({
        "id": id,
        "object": "response",
        "status": "completed",
        "model": model,
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text }],
        }],
        "usage": chat.get("usage").cloned().unwrap_or(json!({})),
    });
    Ok(Bytes::from(serde_json::to_vec(&out)?))
}

struct SseMapState {
    response_id: String,
    item_id: String,
    model: String,
    text_buffer: String,
    sequence: u32,
    completed: bool,
    created_sent: bool,
    message_prelude_sent: bool,
    tool_calls: HashMap<usize, ToolCallAccum>,
    upstream_samples: Vec<String>,
    output_index: u32,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
    item_added: bool,
    output_index: u32,
}

fn new_ids() -> (String, String) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    (
        format!("sugt-resp-{millis}"),
        format!("msg_{millis}"),
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
    let item_added = json!({
        "type": "response.output_item.added",
        "sequence_number": state.sequence,
        "output_index": 0,
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
        "output_index": 0,
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

fn finalize_events(state: &mut SseMapState) -> String {
    if state.completed {
        return String::new();
    }
    state.completed = true;
    let mut out = String::new();
    let mut completed_output: Vec<Value> = Vec::new();

    if state.message_prelude_sent || !state.text_buffer.is_empty() {
        let text_done = json!({
            "type": "response.output_text.done",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": 0,
            "content_index": 0,
            "text": state.text_buffer,
        });
        state.sequence += 1;
        out += &format_sse_data(&text_done);

        let part_done = json!({
            "type": "response.content_part.done",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": 0,
            "content_index": 0,
            "part": { "type": "output_text", "text": state.text_buffer },
        });
        state.sequence += 1;
        out += &format_sse_data(&part_done);

        let item_done = json!({
            "type": "response.output_item.done",
            "sequence_number": state.sequence,
            "output_index": 0,
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

    for tool in state.tool_calls.values() {
        let item_done = json!({
            "type": "response.output_item.done",
            "sequence_number": state.sequence,
            "output_index": tool.output_index,
            "item": {
                "type": "function_call",
                "id": tool.id,
                "call_id": tool.id,
                "name": tool.name,
                "arguments": tool.arguments,
                "status": "completed",
            }
        });
        state.sequence += 1;
        out += &format_sse_data(&item_done);
        completed_output.push(json!({
            "type": "function_call",
            "id": tool.id,
            "call_id": tool.id,
            "name": tool.name,
            "arguments": tool.arguments,
            "status": "completed",
        }));
    }

    let completed = json!({
        "type": "response.completed",
        "sequence_number": state.sequence,
        "response": {
            "id": state.response_id,
            "object": "response",
            "status": "completed",
            "model": state.model,
            "output": completed_output,
        }
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
    extract_chat_delta(parsed)
}

fn extract_chat_delta(parsed: &Value) -> Option<String> {
    let choice = parsed.get("choices").and_then(|c| c.get(0));
    if choice.is_none() {
        return None;
    }
    let choice = choice?;

    if let Some(delta_obj) = choice.get("delta") {
        if let Some(text) = delta_obj.get("content").and_then(content_value_to_text) {
            return Some(text);
        }
        if let Some(s) = delta_obj.get("text").and_then(|c| c.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(s) = delta_obj.get("reasoning_content").and_then(|c| c.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(s) = delta_obj.get("reasoning").and_then(|c| c.as_str()) {
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
            let item_added = json!({
                "type": "response.output_item.added",
                "sequence_number": state.sequence,
                "output_index": entry.output_index,
                "item": {
                    "type": "function_call",
                    "id": entry.id,
                    "call_id": entry.id,
                    "name": entry.name,
                    "arguments": "",
                    "status": "in_progress",
                }
            });
            out += &format_sse_data(&item_added);
        }

        if let Some(args) = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
        {
            if !args.is_empty() {
                state.sequence += 1;
                let event = json!({
                    "type": "response.function_call_arguments.delta",
                    "sequence_number": state.sequence,
                    "item_id": entry.id,
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

    let mut out = String::new();

    if let Some(choice) = parsed.get("choices").and_then(|c| c.get(0)) {
        out += &process_tool_call_delta(state, choice);
    }

    if let Some(delta) = extract_chat_delta(&parsed) {
        out += &ensure_message_prelude(state);
        state.text_buffer += &delta;
        state.sequence += 1;
        let event = json!({
            "type": "response.output_text.delta",
            "sequence_number": state.sequence,
            "item_id": state.item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": delta,
        });
        out += &format_sse_data(&event);
    }

    if upstream_finished(&parsed) {
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
                        "output_index": 0,
                        "content_index": 0,
                        "delta": state.text_buffer.clone(),
                    });
                    out += &format_sse_data(&event);
                }
            }
        }
        out += &finalize_events(state);
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

/// ? OpenAI Chat SSE ?? Responses SSE?Codex ??????????
pub fn chat_sse_to_responses_sse(
    input: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    model: String,
) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
    Box::pin(stream! {
        let (response_id, item_id) = new_ids();
        let mut state = SseMapState {
            response_id,
            item_id,
            model,
            text_buffer: String::new(),
            sequence: 0,
            completed: false,
            created_sent: false,
            message_prelude_sent: false,
            tool_calls: HashMap::new(),
            upstream_samples: Vec::new(),
            output_index: 0,
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

        if state.text_buffer.is_empty() && state.tool_calls.is_empty() {
            warn!(
                samples = ?state.upstream_samples,
                "codex chat stream empty: upstream returned no text/tool_calls; check provider base_url and use 0.2.13+ for volcengine native passthrough"
            );
        } else {
            debug!(
                text_len = state.text_buffer.len(),
                tool_calls = state.tool_calls.len(),
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
        let (response_id, item_id) = new_ids();
        let mut state = SseMapState {
            response_id,
            item_id,
            model: "test-model".to_string(),
            text_buffer: String::new(),
            sequence: 1,
            completed: false,
            created_sent: true,
            message_prelude_sent: false,
            tool_calls: HashMap::new(),
            upstream_samples: Vec::new(),
            output_index: 0,
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
        assert_eq!(extract_chat_delta(&parsed), Some("think".to_string()));
    }
}
