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

/// Responses 标准字段中，无状态网关无法透传给 Chat 上游的部分。
/// 这些字段不再静默丢弃：统一记录到 ChatToolMeta 并打日志，真机抓包时可直接
/// 看出「客户端要了哪些能力、网关降级了哪些」，而不是面对一个无法解释的黑洞。
const DEGRADED_RESPONSES_FIELDS: &[&str] = &[
    "previous_response_id",
    "store",
    "reasoning",
    "include",
    "truncation",
    "service_tier",
    "metadata",
    "prompt_cache_key",
    "safety_identifier",
    "background",
    "conversation",
    "text",
    "prompt",
];

/// Chat 回落时保留的工具元数据，用于把 tool_calls 还原成 Codex 期望的 item 类型。
#[derive(Debug, Clone, Default)]
pub struct ChatToolMeta {
    /// 由 Responses `custom` 转成的 Chat function 名（如 exec / apply_patch）
    pub custom_names: HashSet<String>,
    /// namespace 展平后的上游工具名 → 客户端原始工具名，用于回程还原
    pub flattened_names: HashMap<String, String>,
    /// 本次请求是否携带工具声明。有声明时严格按声明判定工具形态，
    /// 避免把官方 function 工具（如 shell_command）误判成 freeform custom。
    pub has_tool_declarations: bool,
    /// 本次请求里被显式降级的 Responses 顶层字段名（可观测，便于真机诊断）。
    pub dropped_request_fields: Vec<String>,
    /// 同名工具出现在多个 namespace：去程无法唯一确定展平名，历史改写必须保守跳过。
    pub ambiguous_tool_names: HashSet<String>,
    /// 上游不支持 FC 时进入命令模式：正文里的约定标记会合成 function_call。
    pub command_mode: bool,
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
    // tools 已展平为 namespace__name，历史里的客户端原名必须同步改写，
    // 否则多轮闭环会出现「声明 functions__exec_command、历史却叫 exec_command」，
    // 上游拒收或模型混用 shell_command.command / exec_command.cmd 参数。
    rewrite_history_tool_names(&mut messages, &meta);
    // 上游（OpenRouter/Kilo 等）会拒空 function.name；Codex 会话/插件偶发带入
    sanitize_chat_messages_tool_calls(&mut messages);
    // 历史回放里缺 function_call_output 时，Chat 上游会因孤儿 tool_calls 直接 400。
    repair_orphan_tool_calls(&mut messages);

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
            // Responses 用 max_output_tokens 控制输出长度；Chat 上游只认 max_tokens。
            // 之前这个字段被白名单静默丢掉，长任务会被上游默认上限截断。
            if !out_obj.contains_key("max_tokens") {
                if let Some(v) = obj
                    .get("max_completion_tokens")
                    .or_else(|| obj.get("max_output_tokens"))
                {
                    out_obj.insert("max_tokens".to_string(), v.clone());
                }
            }

            // 无状态网关依赖客户端重放完整 input；把降级字段显式暴露出来。
            let degraded: Vec<&str> = DEGRADED_RESPONSES_FIELDS
                .iter()
                .copied()
                .filter(|key| obj.contains_key(*key))
                .collect();
            if !degraded.is_empty() {
                meta.dropped_request_fields = degraded.iter().map(|s| s.to_string()).collect();
                warn!(
                    fields = ?degraded,
                    "Codex Responses 字段在当前无状态网关下降级，依赖客户端重放完整 input"
                );
            }

            sanitize_tool_choice(out_obj, &meta);

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
    let mut declared = false;

    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        declared = !tools.is_empty();
        for tool in tools {
            push_response_tool_as_chat(tool, None, &mut chat_tools, &mut seen, meta);
        }
    }
    if let Some(input) = body.get("input") {
        declared |= collect_input_declared_tools(input, &mut chat_tools, &mut seen, meta);
    }
    meta.has_tool_declarations = declared;
    chat_tools
}

/// 返回 input 里是否出现过工具声明载体（additional_tools / tool_search_output）。
fn collect_input_declared_tools(
    value: &Value,
    chat_tools: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    meta: &mut ChatToolMeta,
) -> bool {
    match value {
        Value::Array(items) => {
            let mut found = false;
            for item in items {
                found |= collect_input_declared_tools(item, chat_tools, seen, meta);
            }
            found
        }
        Value::Object(obj) => {
            let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let mut found = false;
            if matches!(typ, "additional_tools" | "tool_search_output") {
                if let Some(tools) = obj.get("tools").and_then(|v| v.as_array()) {
                    for tool in tools {
                        push_response_tool_as_chat(tool, None, chat_tools, seen, meta);
                    }
                    found = true;
                }
            }
            for child in obj.values() {
                found |= collect_input_declared_tools(child, chat_tools, seen, meta);
            }
            found
        }
        _ => false,
    }
}

/// namespace 里的工具展平成上游可识别的平铺名字，回程靠元数据还原。
fn qualified_tool_name(prefix: Option<&str>, name: &str) -> String {
    match prefix {
        Some(p) if !p.is_empty() => format!("{p}__{name}"),
        _ => name.to_string(),
    }
}

/// 把展平后的名字写回 Chat function，并登记「上游名 → 客户端原名」映射。
///
/// 两种情况不能静默覆盖：不同 namespace 下有同名工具（去程反查没有唯一答案），
/// 以及展平名本身碰撞（`a` + `b__c` 与 `a__b` + `c` 都会得到 `a__b__c`）。
/// 前者标记歧义让历史改写保守跳过，后者生成带序号的唯一名，保证两个工具都不丢。
fn register_tool_name(
    chat: &mut Value,
    original: &str,
    prefix: Option<&str>,
    meta: &mut ChatToolMeta,
) -> String {
    let base = qualified_tool_name(prefix, original);
    let mut upstream = base.clone();
    let mut suffix = 2u32;
    while meta
        .flattened_names
        .get(&upstream)
        .is_some_and(|existing| existing != original)
    {
        warn!(
            tool = %original,
            collided = %upstream,
            "展平名碰撞，改用带序号的唯一上游名"
        );
        upstream = format!("{base}__{suffix}");
        suffix += 1;
    }
    if meta
        .flattened_names
        .iter()
        .any(|(registered, existing)| existing == original && registered != &upstream)
    {
        warn!(
            tool = %original,
            "同名工具出现在多个 namespace，历史工具名改写将保守跳过"
        );
        meta.ambiguous_tool_names.insert(original.to_string());
    }
    if upstream != original {
        if let Some(function) = chat.pointer_mut("/function") {
            if let Some(obj) = function.as_object_mut() {
                obj.insert("name".to_string(), json!(upstream.clone()));
            }
        }
        meta.flattened_names
            .insert(upstream.clone(), original.to_string());
    }
    upstream
}

/// 未识别但带 schema 的工具类型降级为普通 function；hosted 工具通常无 schema，跳过。
fn fallback_unknown_tool_as_function(
    tool: &Value,
    prefix: Option<&str>,
    meta: &mut ChatToolMeta,
) -> Option<(String, Value)> {
    let original = non_empty_name(tool, &["name", "/function/name"])?;
    let has_schema = tool.get("parameters").is_some()
        || tool.get("input_schema").is_some()
        || tool.pointer("/function/parameters").is_some();
    if !has_schema {
        return None;
    }
    let mut chat = responses_function_to_chat_tool(tool)?;
    let upstream = register_tool_name(&mut chat, original, prefix, meta);
    Some((upstream, chat))
}

fn push_response_tool_as_chat(
    tool: &Value,
    prefix: Option<&str>,
    chat_tools: &mut Vec<Value>,
    seen: &mut HashSet<String>,
    meta: &mut ChatToolMeta,
) {
    let typ = tool
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    match typ {
        "function" => {
            if let Some(mut chat) = responses_function_to_chat_tool(tool) {
                let original = chat
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = register_tool_name(&mut chat, &original, prefix, meta);
                if !name.is_empty() && seen.insert(name) {
                    chat_tools.push(chat);
                }
            }
        }
        "custom" => {
            if let Some((original, mut chat)) = responses_custom_to_chat_tool(tool) {
                let name = register_tool_name(&mut chat, &original, prefix, meta);
                if seen.insert(name.clone()) {
                    meta.custom_names.insert(name);
                    chat_tools.push(chat);
                }
            }
        }
        "namespace" => {
            let ns = tool
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("namespace");
            let child_prefix = qualified_tool_name(prefix, ns);
            let children = tool
                .get("tools")
                .or_else(|| tool.get("children"))
                .and_then(|v| v.as_array());
            if let Some(children) = children {
                for child in children {
                    // Codex 把 exec/apply_patch 等 custom 挂在 functions namespace 下
                    push_response_tool_as_chat(child, Some(&child_prefix), chat_tools, seen, meta);
                }
            }
        }
        "tool_search" => {
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
        other => {
            if let Some((name, chat)) = fallback_unknown_tool_as_function(tool, prefix, meta) {
                if seen.insert(name.clone()) {
                    warn!(
                        tool_type = other,
                        tool_name = %name,
                        "未识别的 Responses 工具类型已降级为 function"
                    );
                    chat_tools.push(chat);
                }
            } else {
                warn!(
                    tool_type = other,
                    "跳过无 schema 的未识别工具类型（hosted 工具由客户端自管）"
                );
            }
        }
    }
}

/// 解析工具/调用名：兼容 Responses 顶层 `name` 与 Chat 嵌套 `function.name`；空串视为缺失。
fn non_empty_name<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a str> {
    for path in paths {
        let raw = if path.contains('/') {
            value.pointer(path).and_then(|v| v.as_str())
        } else {
            value.get(*path).and_then(|v| v.as_str())
        };
        if let Some(name) = raw.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(name);
        }
    }
    None
}

fn responses_function_to_chat_tool(tool: &Value) -> Option<Value> {
    // Responses: {type,name,...}；部分客户端会塞 Chat 形 {function:{name}}
    let name = non_empty_name(tool, &["name", "/function/name"])?;
    let description = tool
        .get("description")
        .cloned()
        .or_else(|| tool.pointer("/function/description").cloned())
        .unwrap_or_else(|| json!(""));
    let parameters = tool
        .get("parameters")
        .or_else(|| tool.get("input_schema"))
        .or_else(|| tool.pointer("/function/parameters"))
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
    let name = non_empty_name(tool, &["name", "/function/name"])?.to_string();
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
    // 请求里带工具声明时严格按声明判定：没有 custom 声明就不是 freeform 工具。
    // 否则会把官方 function 工具（shell_command）误判成 custom_tool_call，客户端直接 aborted。
    if meta.has_tool_declarations {
        return false;
    }
    // 无声明的历史回放兜底：仅保留确实是 freeform 的名字。
    // shell_command 是 OpenAI 官方 Responses 的 JSON 参数 function 工具，绝不能归入此处。
    matches!(name, "exec" | "apply_patch" | "write_stdin" | "shell")
}

/// 回程把 namespace 展平名还原成客户端认识的原始工具名。
fn restore_tool_name(name: &str, meta: &ChatToolMeta) -> String {
    meta.flattened_names
        .get(name)
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

/// 去程：客户端原名 → 上游展平名。已是展平名则原样保留。
fn upstream_tool_name(name: &str, meta: &ChatToolMeta) -> String {
    if name.is_empty() || meta.flattened_names.is_empty() {
        return name.to_string();
    }
    // 多个 namespace 有同名工具时反查结果取决于 HashMap 遍历顺序；
    // 保持原名是唯一确定的行为，绝不随机挑一个。
    if meta.ambiguous_tool_names.contains(name) {
        return name.to_string();
    }
    if meta.flattened_names.contains_key(name) {
        return name.to_string();
    }
    for (upstream, original) in &meta.flattened_names {
        if original == name {
            return upstream.clone();
        }
    }
    name.to_string()
}

/// 把历史 tool_calls 里的客户端原名改写成与 tools[] 一致的展平名。
fn rewrite_history_tool_names(messages: &mut [Value], meta: &ChatToolMeta) {
    if meta.flattened_names.is_empty() {
        return;
    }
    let mut rewritten = 0u32;
    let mut skipped = 0u32;
    for msg in messages.iter_mut() {
        let Some(calls) = msg
            .as_object_mut()
            .and_then(|obj| obj.get_mut("tool_calls"))
            .and_then(|v| v.as_array_mut())
        else {
            continue;
        };
        for call in calls.iter_mut() {
            let Some(function) = call
                .as_object_mut()
                .and_then(|obj| obj.get_mut("function"))
                .and_then(|v| v.as_object_mut())
            else {
                continue;
            };
            let Some(name) = function.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            if meta.ambiguous_tool_names.contains(&name) {
                skipped += 1;
                continue;
            }
            let upstream = upstream_tool_name(&name, meta);
            if upstream != name {
                function.insert("name".to_string(), json!(upstream));
                rewritten += 1;
            }
        }
    }
    if rewritten > 0 {
        warn!(
            rewritten,
            "rewrote history tool_calls names to flattened upstream form"
        );
    }
    if skipped > 0 {
        warn!(
            skipped,
            "ambiguous namespace tool names kept as-is in history replay"
        );
    }
}

pub use crate::responses_upstream::{
    is_volcengine_coding_model, native_responses_base_url, provider_supports_native_responses,
    rewrite_responses_upstream_model, should_try_native_responses,
};

fn output_value_to_string(output: &Value) -> String {
    match output {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn arguments_value_to_string(arguments: &Value) -> String {
    match arguments {
        Value::String(s) => s.clone(),
        Value::Null => "{}".to_string(),
        other => other.to_string(),
    }
}

/// 从「未识别」的 Responses item 里尽量提取纯文本。
/// 之前直接 `item.to_string()` 会把整段 JSON 结构当成用户输入喂给模型，
/// 既污染上下文又让工具语义丢失；这里退化成文本优先、原结构兜底。
fn plain_response_item_text(item: &Value) -> Option<String> {
    for key in ["text", "content", "output", "result"] {
        let Some(value) = item.get(key) else {
            continue;
        };
        if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
        if let Some(s) = content_value_to_text(value).filter(|s| !s.is_empty()) {
            return Some(s);
        }
    }
    None
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
                let mut msg = json!({ "role": role, "content": content });
                // 部分会话会把历史 tool_calls 挂在 message 上；空 name 需留给 sanitize
                if let (Some(out), Some(calls)) = (
                    msg.as_object_mut(),
                    item.get("tool_calls").filter(|v| v.is_array()),
                ) {
                    out.insert("tool_calls".to_string(), calls.clone());
                }
                Some(msg)
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
                // `name: ""` 时 unwrap_or 不会兜底，上游会直接 400
                let name = non_empty_name(item, &["name", "/function/name"]).unwrap_or("function");
                let arguments = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .or_else(|| item.pointer("/function/arguments").and_then(|a| a.as_str()))
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
                let name = non_empty_name(item, &["name", "/function/name"]).unwrap_or("custom");
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
            // 客户端偶尔把系统级说明放进 input 数组；转成 system 而不是 user 文本。
            "instructions" | "developer_message" => plain_response_item_text(item)
                .map(|text| json!({ "role": "system", "content": text })),
            // 工具声明载体：已在 collect_chat_tools 提起，不能当成 user 文本
            "item_reference" | "reasoning" | "additional_tools" | "tool_search_output" => None,
            other => {
                // MCP / local_shell / tool_search 等非标准工具 item 必须保住 call_id ↔ output 结构，
                // 否则 Chat 上游只看到一段 JSON 文本，工具闭环直接断掉。
                let call_id = item.get("call_id").and_then(|v| v.as_str());
                if let Some(call_id) = call_id {
                    if let Some(output) = item.get("output").or_else(|| item.get("result")) {
                        return Some(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": output_value_to_string(output),
                        }));
                    }
                }
                if let Some(name) = non_empty_name(item, &["name", "/function/name"]) {
                    let call_id = call_id.unwrap_or("call");
                    let arguments = item
                        .get("arguments")
                        .or_else(|| item.get("input"))
                        .map(arguments_value_to_string)
                        .unwrap_or_else(|| "{}".to_string());
                    return Some(json!({
                        "role": "assistant",
                        "content": Value::Null,
                        "tool_calls": [{
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments },
                        }],
                    }));
                }
                let text = plain_response_item_text(item).unwrap_or_else(|| {
                    warn!(
                        item_type = %other,
                        "未识别的 Responses input item 无文本字段，按原结构降级"
                    );
                    item.to_string()
                });
                Some(json!({ "role": "user", "content": text }))
            }
        }
    } else if let Some(obj) = item.as_object() {
        // 无 type 的 Chat 形消息：保留 tool_calls，稍后统一净化空 name
        let role = obj.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = obj
            .get("content")
            .map(normalize_message_content)
            .unwrap_or(json!(""));
        let mut msg = json!({ "role": role, "content": content });
        if let (Some(out), Some(calls)) = (
            msg.as_object_mut(),
            obj.get("tool_calls").filter(|v| v.is_array()),
        ) {
            out.insert("tool_calls".to_string(), calls.clone());
        }
        if let (Some(out), Some(id)) = (
            msg.as_object_mut(),
            obj.get("tool_call_id").and_then(|v| v.as_str()),
        ) {
            out.insert("tool_call_id".to_string(), json!(id));
        }
        Some(msg)
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

fn tool_call_function_name(call: &Value) -> Option<&str> {
    non_empty_name(call, &["/function/name", "name"])
}

/// 去掉 / 修补 messages 里空的 tool_calls.function.name，避免上游 400。
fn sanitize_chat_messages_tool_calls(messages: &mut Vec<Value>) {
    let mut dropped = 0u32;
    let mut patched = 0u32;
    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };
        let Some(Value::Array(calls)) = obj.get_mut("tool_calls") else {
            continue;
        };
        let mut kept = Vec::with_capacity(calls.len());
        for call in calls.drain(..) {
            if tool_call_function_name(&call).is_some() {
                kept.push(call);
                continue;
            }
            // 有 id/arguments 但 name 空：补占位，避免整段历史断裂
            if let Some(call_obj) = call.as_object() {
                let mut fixed = call.clone();
                if let Some(fixed_obj) = fixed.as_object_mut() {
                    let mut function = call_obj
                        .get("function")
                        .and_then(|f| f.as_object())
                        .cloned()
                        .unwrap_or_default();
                    function.insert("name".to_string(), json!("unknown_tool"));
                    if !function.contains_key("arguments") {
                        function.insert("arguments".to_string(), json!("{}"));
                    }
                    fixed_obj.insert("function".to_string(), Value::Object(function));
                    if !fixed_obj.contains_key("type") {
                        fixed_obj.insert("type".to_string(), json!("function"));
                    }
                    if !fixed_obj.contains_key("id") {
                        fixed_obj.insert("id".to_string(), json!("call_unknown"));
                    }
                }
                kept.push(fixed);
                patched += 1;
            } else {
                dropped += 1;
            }
        }
        if kept.is_empty() {
            obj.remove("tool_calls");
        } else {
            obj.insert("tool_calls".to_string(), Value::Array(kept));
        }
    }
    if dropped > 0 || patched > 0 {
        warn!(
            dropped,
            patched, "sanitized empty tool_calls.function.name before upstream chat"
        );
    }
}

/// 补齐历史里缺少 function_call_output 的 tool_calls。
/// Chat Completions 要求每个 assistant tool_call 紧跟一条 role=tool 结果，
/// 缺失时上游（Kimi / DeepSeek / OpenRouter 等）会直接 400 或中断整轮。
fn repair_orphan_tool_calls(messages: &mut Vec<Value>) {
    let mut answered: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        if let Some(id) = msg.get("tool_call_id").and_then(Value::as_str) {
            answered.insert(id.to_string());
        }
    }
    let mut repaired: Vec<Value> = Vec::with_capacity(messages.len());
    let mut added = 0u32;
    for msg in messages.drain(..) {
        let calls = msg
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        repaired.push(msg);
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("");
            if id.is_empty() || !answered.insert(id.to_string()) {
                continue;
            }
            repaired.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": "(工具结果缺失，已由 SUGT 补齐配对)",
            }));
            added += 1;
        }
    }
    *messages = repaired;
    if added > 0 {
        warn!(added, "paired orphan tool_calls before upstream chat");
    }
}

fn sanitize_tool_choice(out_obj: &mut serde_json::Map<String, Value>, meta: &ChatToolMeta) {
    let Some(choice) = out_obj.get("tool_choice") else {
        return;
    };
    // 字符串 "auto"/"none"/"required" 原样保留
    if choice.is_string() {
        return;
    }
    let name = choice
        .pointer("/function/name")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if name.is_empty() && choice.get("type").and_then(|t| t.as_str()) == Some("function") {
        warn!("dropping tool_choice with empty function.name");
        out_obj.insert("tool_choice".to_string(), json!("auto"));
        return;
    }
    if name.is_empty() {
        return;
    }
    let upstream = upstream_tool_name(&name, meta);
    if upstream == name {
        return;
    }
    if let Some(function) = out_obj
        .get_mut("tool_choice")
        .and_then(|v| v.as_object_mut())
        .and_then(|obj| obj.get_mut("function"))
        .and_then(|v| v.as_object_mut())
    {
        function.insert("name".to_string(), json!(upstream));
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

/// 兼容层常在 HTTP 200 的正常响应里附带 `"error": null`、空串或空对象。
/// 流式与非流式必须共用这一个判定，否则一边修好另一边仍会把整轮对话判成失败。
fn is_real_error_value(err: &Value) -> bool {
    match err {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        _ => true,
    }
}

/// 判断上游 body 是否为错误体（含 HTTP 200 里夹带 error 对象的兼容层响应）。
pub fn is_openai_compat_error_body(body: &str) -> bool {
    let value: Value = serde_json::from_str(body).unwrap_or(json!({}));
    if let Some(err) = value.get("error") {
        if is_real_error_value(err) {
            return true;
        }
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

/// 上游返回空 body / 非法 JSON 时构造 Responses 形状的失败响应。
/// 之前会伪装成 `status: completed` 的空回复，客户端以为模型「正常地什么都没说」，
/// 工具闭环就此静默断掉；现在明确失败并带上可诊断的 error 对象。
pub(crate) fn responses_upstream_failure(model: &str, message: &str) -> Result<Bytes> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let out = json!({
        "id": "sugt-resp-error",
        "object": "response",
        "created_at": created_at,
        "status": "failed",
        "model": model,
        "output": [],
        "output_text": "",
        "usage": responses_usage(0, 0),
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "store": false,
        "incomplete_details": Value::Null,
        "error": {
            "message": message,
            "type": "upstream_error",
            "code": Value::Null,
            "param": Value::Null,
        },
    });
    Ok(Bytes::from(serde_json::to_vec(&out)?))
}

pub fn chat_json_to_responses_body_with_meta(
    body: &str,
    model: &str,
    meta: &ChatToolMeta,
) -> Result<Bytes> {
    if body.trim().is_empty() {
        warn!("上游返回空响应体，降级为 Responses failed 而非伪装成功");
        return responses_upstream_failure(model, "上游返回空响应体");
    }

    if is_openai_compat_error_body(body) {
        let message = serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        v.get("error")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
            })
            .unwrap_or_else(|| "上游返回错误响应".to_string());
        warn!(%message, "上游 Chat 错误体转为 Responses failed，避免客户端按成功解析");
        return responses_upstream_failure(model, &message);
    }

    let chat: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(err) => {
            warn!(error = %err, "上游返回非法 JSON，降级为 Responses failed 而非伪装成功");
            return responses_upstream_failure(model, &format!("上游返回非法 JSON：{err}"));
        }
    };

    let choices = chat.get("choices").and_then(Value::as_array);
    if choices.map(|c| c.is_empty()).unwrap_or(true) {
        warn!("上游返回空 choices，降级为 Responses failed 而非伪装 completed");
        return responses_upstream_failure(model, "上游返回空 choices / 无有效内容");
    }

    let message = choices
        .and_then(|c| c.first())
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

    // 命令模式：正文里的约定标记合成 function_call，并给用户可读提示。
    let mut command_items: Vec<Value> = Vec::new();
    if meta.command_mode {
        let shell = crate::command_mode::preferred_shell_tool_name(
            &meta.flattened_names,
            &meta.custom_names,
        );
        let (visible, items) =
            crate::command_mode::materialize_command_mode_output(&text, id, &shell);
        text = visible;
        command_items = items;
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
            let upstream_name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("function");
            // 判定用展平后的名字（meta 里存的就是发给上游的名字），输出还原成客户端原名。
            let name = restore_tool_name(upstream_name, meta);
            let arguments = call
                .pointer("/function/arguments")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "{}".to_string());
            if is_custom_tool_name(upstream_name, meta) {
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
    // 原生 tool_calls 优先；没有时才用命令模式合成，避免重复。
    if message
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true)
    {
        output.extend(command_items);
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
    let output_text = output
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Responses 客户端会反序列化完整响应对象，缺失的顶层字段会让整条响应解析失败。
    let out = json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "status": "completed",
        "model": model,
        "output": output,
        "output_text": output_text,
        "usage": usage,
        "parallel_tool_calls": false,
        "tool_choice": "auto",
        "store": false,
        "incomplete_details": Value::Null,
        "error": Value::Null,
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
    /// 工具名还没到达时先攒住的参数分片。真实上游常把 arguments 分片放在
    /// name 之前，若直接发 delta 会让客户端先收到参数、后收到工具声明。
    pending_deltas: Vec<String>,
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

fn in_progress_event(state: &SseMapState) -> String {
    let event = json!({
        "type": "response.in_progress",
        "sequence_number": state.sequence,
        "response": {
            "id": state.response_id,
            "object": "response",
            "status": "in_progress",
            "model": state.model,
            "output": [],
        }
    });
    format_sse_data(&event)
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

/// 流结束收尾：空流 → failed；有内容/推理/工具 → completed（含命令模式合成）。
fn end_stream_events(state: &mut SseMapState) -> String {
    if state.completed {
        return String::new();
    }
    if state.text_buffer.is_empty()
        && state.tool_calls.is_empty()
        && state.reasoning_buffer.is_empty()
    {
        warn!("上游流式响应为空（无正文/工具/推理），降级为 Responses failed");
        return failed_events(state, "上游返回空流式响应（无正文/工具/推理）");
    }
    finalize_events(state)
}

/// 命令模式流式收尾：清洗正文、补用户提示，并把约定命令写入 tool_calls。
fn apply_command_mode_to_stream_state(state: &mut SseMapState, out: &mut String) {
    let shell = crate::command_mode::preferred_shell_tool_name(
        &state.tool_meta.flattened_names,
        &state.tool_meta.custom_names,
    );
    let raw = state.text_buffer.clone();
    let (visible, items) =
        crate::command_mode::materialize_command_mode_output(&raw, &state.response_id, &shell);
    if items.is_empty() && visible == raw {
        return;
    }

    // done 事件以清洗后的正文为准；若尚未发过 message prelude，这里补发。
    if visible != raw {
        if !state.message_prelude_sent && !visible.is_empty() {
            *out += &ensure_message_prelude(state);
            state.sequence += 1;
            *out += &format_sse_data(&json!({
                "type": "response.output_text.delta",
                "sequence_number": state.sequence,
                "item_id": state.item_id,
                "output_index": state.message_output_index,
                "content_index": 0,
                "delta": visible.clone(),
            }));
        } else if state.message_prelude_sent
            && !raw.contains(crate::command_mode::USER_NOTICE)
            && visible.contains(crate::command_mode::USER_NOTICE)
        {
            // 分片里已有正文：追加一句提示，避免用户不知道为何走了命令模式。
            let notice_delta = format!("\n{}", crate::command_mode::USER_NOTICE);
            state.sequence += 1;
            *out += &format_sse_data(&json!({
                "type": "response.output_text.delta",
                "sequence_number": state.sequence,
                "item_id": state.item_id,
                "output_index": state.message_output_index,
                "content_index": 0,
                "delta": notice_delta,
            }));
        }
        state.text_buffer = visible;
    }

    for (idx, item) in items.iter().enumerate() {
        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or("cmd")
            .to_string();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(shell.as_str())
            .to_string();
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("{}")
            .to_string();
        let output_index = state.output_index;
        state.output_index += 1;

        state.sequence += 1;
        *out += &format_sse_data(&json!({
            "type": "response.output_item.added",
            "sequence_number": state.sequence,
            "output_index": output_index,
            "item": tool_item_shell(&call_id, &name, false, &state.tool_meta),
        }));
        *out += &arguments_delta_event(&mut state.sequence, &call_id, output_index, &arguments);

        // 用高位 key，避免与上游 tool_calls.index 撞车。
        state.tool_calls.insert(
            10_000 + idx,
            ToolCallAccum {
                id: call_id,
                name,
                arguments,
                item_added: true,
                output_index,
                is_custom: false,
                pending_deltas: Vec::new(),
            },
        );
    }
}

fn finalize_events(state: &mut SseMapState) -> String {
    if state.completed {
        return String::new();
    }
    state.completed = true;
    apply_usage_fallback(state);
    let mut out = String::new();
    let mut completed_output: Vec<Value> = Vec::new();

    // 先修「上游始终没给工具名」的分片，保证后面的 done / completed 拿到可解析的名字。
    repair_anonymous_tool_calls(state, &mut out);

    // 命令模式：把正文里的约定标记合成 tool_calls，再进入正常 done 路径。
    if state.tool_meta.command_mode && state.tool_calls.is_empty() {
        apply_command_mode_to_stream_state(state, &mut out);
    }

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
        let client_name = restore_tool_name(&tool.name, &state.tool_meta);
        let item = if tool.is_custom {
            json!({
                "type": "custom_tool_call",
                "id": format!("ctc_{}", tool.id),
                "call_id": tool.id,
                "name": client_name,
                "input": custom_tool_input_from_chat_arguments(&tool.arguments),
                "status": "completed",
            })
        } else {
            json!({
                "type": "function_call",
                "id": format!("fc_{}", tool.id),
                "call_id": tool.id,
                "name": client_name,
                "arguments": tool.arguments,
                "status": "completed",
            })
        };
        // 客户端靠 done 事件确认参数流结束；只发 delta 会让工具调用一直挂在 in_progress。
        let arguments_done = if tool.is_custom {
            json!({
                "type": "response.custom_tool_call_input.done",
                "sequence_number": state.sequence,
                "item_id": format!("ctc_{}", tool.id),
                "output_index": tool.output_index,
                "input": custom_tool_input_from_chat_arguments(&tool.arguments),
            })
        } else {
            json!({
                "type": "response.function_call_arguments.done",
                "sequence_number": state.sequence,
                "item_id": format!("fc_{}", tool.id),
                "output_index": tool.output_index,
                "arguments": tool.arguments,
            })
        };
        state.sequence += 1;
        out += &format_sse_data(&arguments_done);
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
            "error": {
                "message": message,
                "type": "upstream_error",
                "code": Value::Null,
                "param": Value::Null,
            },
        }
    });
    format_sse_data(&failed) + "data: [DONE]\n\n"
}

/// 上游返回 4xx 时，构造一段 Responses 形状的失败 SSE 直接交给客户端。
/// 之前把错误体当作成功流转发，客户端只会收到一条空的 `response.completed`，
/// 看起来像「模型正常地什么都没说」，错误完全不可诊断。
pub fn upstream_failure_sse(model: &str, message: &str) -> String {
    let failed = json!({
        "type": "response.failed",
        "sequence_number": 1,
        "response": {
            "id": "sugt-resp-error",
            "object": "response",
            "status": "failed",
            "model": model,
            "error": {
                "message": message,
                "type": "upstream_error",
                "code": Value::Null,
                "param": Value::Null,
            },
        },
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
        // 与非流式共用判定：`error: null` / "" / {} 是兼容层的正常分片，
        // 不能触发 response.failed 并打断后续 SSE 事件序列。
        if !is_real_error_value(err) {
            return None;
        }
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

/// in_progress 阶段的工具 item 外壳，added 事件与 finalize 补发共用同一形状。
fn tool_item_shell(id: &str, name: &str, is_custom: bool, meta: &ChatToolMeta) -> Value {
    let client_name = restore_tool_name(name, meta);
    if is_custom {
        json!({
            "type": "custom_tool_call",
            "id": format!("ctc_{}", id),
            "call_id": id,
            "name": client_name,
            "input": "",
            "status": "in_progress",
        })
    } else {
        json!({
            "type": "function_call",
            "id": format!("fc_{}", id),
            "call_id": id,
            "name": client_name,
            "arguments": "",
            "status": "in_progress",
        })
    }
}

fn arguments_delta_event(sequence: &mut u32, id: &str, output_index: u32, delta: &str) -> String {
    *sequence += 1;
    format_sse_data(&json!({
        "type": "response.function_call_arguments.delta",
        "sequence_number": *sequence,
        "item_id": format!("fc_{}", id),
        "output_index": output_index,
        "delta": delta,
    }))
}

/// 上游始终没给工具名时，在收尾阶段补一个可解析的名字和完整的 added / delta 事件。
/// 空名字会让客户端直接解析失败，比「工具调用丢失」更难排查。
fn repair_anonymous_tool_calls(state: &mut SseMapState, out: &mut String) {
    let keys: Vec<usize> = state
        .tool_calls
        .iter()
        .filter(|(_, tool)| tool.name.is_empty())
        .map(|(key, _)| *key)
        .collect();
    if keys.is_empty() {
        return;
    }
    // 兜底名必须用「发给上游的名字」（custom_names / flattened keys），
    // 不能混入 flattened values（客户端原名），否则单工具 namespace 会变成 2 个候选而失效。
    let mut declared: Vec<String> = state.tool_meta.custom_names.iter().cloned().collect();
    declared.extend(state.tool_meta.flattened_names.keys().cloned());
    declared.sort();
    declared.dedup();
    let fallback = if declared.len() == 1 {
        declared.remove(0)
    } else {
        "unknown_tool".to_string()
    };

    for key in keys {
        let Some(tool) = state.tool_calls.get_mut(&key) else {
            continue;
        };
        tool.name = fallback.clone();
        tool.is_custom = state.tool_meta.custom_names.contains(&tool.name);
        tool.item_added = true;
        let pending = std::mem::take(&mut tool.pending_deltas);
        let (id, output_index, name, is_custom) = (
            tool.id.clone(),
            tool.output_index,
            tool.name.clone(),
            tool.is_custom,
        );
        warn!(
            call_id = %id,
            fallback = %name,
            "上游流式工具调用始终未提供 name，使用兜底工具名并补齐事件"
        );
        state.sequence += 1;
        *out += &format_sse_data(&json!({
            "type": "response.output_item.added",
            "sequence_number": state.sequence,
            "output_index": output_index,
            "item": tool_item_shell(&id, &name, is_custom, &state.tool_meta),
        }));
        if !is_custom {
            for delta in pending {
                *out += &arguments_delta_event(&mut state.sequence, &id, output_index, &delta);
            }
        }
    }
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
            // 与 message/reasoning prelude 一样连续占用 output_index，
            // 正文已发后不要再 +1，否则会跳号（0=message → 误分到 2）。
            let output_index = state.output_index;
            state.output_index += 1;
            ToolCallAccum {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
                item_added: false,
                output_index,
                is_custom: false,
                pending_deltas: Vec::new(),
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
        let args_fragment = call
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        if !args_fragment.is_empty() {
            entry.arguments.push_str(&args_fragment);
        }

        if entry.id.is_empty() {
            entry.id = format!("call_{}_{}", state.response_id, index);
        }

        let newly_added = !entry.item_added && !entry.name.is_empty();
        if newly_added {
            entry.item_added = true;
            state.sequence += 1;
            let item = tool_item_shell(&entry.id, &entry.name, entry.is_custom, &state.tool_meta);
            out += &format_sse_data(&json!({
                "type": "response.output_item.added",
                "sequence_number": state.sequence,
                "output_index": entry.output_index,
                "item": item,
            }));
            // name 迟到时，把之前攒住的参数分片按原顺序补发，事件顺序才不会倒置。
            if !entry.is_custom {
                let pending = std::mem::take(&mut entry.pending_deltas);
                let (id, output_index) = (entry.id.clone(), entry.output_index);
                for fragment in pending {
                    out +=
                        &arguments_delta_event(&mut state.sequence, &id, output_index, &fragment);
                }
            }
        }

        // custom 工具的 Chat 参数是 JSON 分片，不能逐片 parse；完整 input 在 item.done 给出。
        if !args_fragment.is_empty() && !entry.is_custom {
            if entry.item_added {
                let (id, output_index) = (entry.id.clone(), entry.output_index);
                out +=
                    &arguments_delta_event(&mut state.sequence, &id, output_index, &args_fragment);
            } else {
                // 工具名还没来：先攒住，绝不抢在 output_item.added 之前发 delta。
                entry.pending_deltas.push(args_fragment);
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
    // 必须按字符边界截断：直接 &payload[..500] 会切在多字节汉字中间并 panic，
    // 长中文回复会直接打断整条 SSE 流。
    let sample = if payload.chars().count() > 500 {
        let prefix: String = payload.chars().take(500).collect();
        format!("{}...", prefix)
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
        return end_stream_events(state);
    }

    if payload.trim().is_empty() {
        return String::new();
    }

    record_upstream_sample(state, payload);

    // 单个坏分片不该毒化整条流：显式告警后跳过，后续正常分片继续处理。
    let parsed: Value = match serde_json::from_str(payload) {
        Ok(value) => value,
        Err(err) => {
            let preview: String = payload.chars().take(120).collect();
            warn!(error = %err, payload = %preview, "跳过无法解析的上游 SSE 分片");
            return String::new();
        }
    };
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
    if parser.buffer.is_empty() {
        return String::new();
    }
    let raw = std::mem::take(&mut parser.buffer);
    let decoded = String::from_utf8_lossy(&raw);
    let remainder = decoded.trim();
    if remainder.is_empty() {
        return String::new();
    }
    if remainder.starts_with('{') {
        return process_upstream_payload(state, remainder);
    }
    process_upstream_line(state, remainder)
}

/// 按字节攒行、按整行解码：上游 TCP chunk 完全可能把一个汉字切成两半，
/// 逐 chunk `from_utf8_lossy` 会永久写入 U+FFFD 并损坏正文。
struct SseLineParser {
    buffer: Vec<u8>,
}

impl SseLineParser {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn push(&mut self, chunk: &Bytes) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        loop {
            let Some(pos) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            let mut raw: Vec<u8> = self.buffer.drain(..=pos).collect();
            if raw.last() == Some(&b'\n') {
                raw.pop();
            }
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            if !raw.is_empty() {
                lines.push(String::from_utf8_lossy(&raw).into_owned());
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
        state.sequence = 2;
        yield Ok(Bytes::from(in_progress_event(&state)));
        state.sequence = 3;

        pin_mut!(input);
        while let Some(chunk) = input.next().await {
            let chunk = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    // 半路截断：不要 yield Err 让客户端只看到连接断开；
                    // 未完成时发一段 Responses failed，工具环才能感知失败。
                    if !state.completed {
                        let msg = err.to_string();
                        yield Ok(Bytes::from(failed_events(&mut state, &msg)));
                    }
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
            let tail = end_stream_events(&mut state);
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
mod tests;
