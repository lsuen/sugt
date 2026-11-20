//! Chat 历史工具环净化（对齐 cc-switch / OpenAI Chat Completions 约束）。
//!
//! Codex Responses → Chat 时，历史里的 `tool_calls[].function.arguments` 必须是
//! **合法 JSON 对象字符串**。模型偶发截断、双编码或尾随字符时，上游（Nvidia /
//! Cohere / OpenRouter）会直接 400，SUGT 再误报「上游拒绝」。
//!
//! 另外：连续多条只含 tool_calls 的 assistant 消息要合并成一条，否则部分上游拒收。

use serde::Deserialize;
use serde_json::{json, Value};
use tracing::warn;

/// 解析 JSON，允许正文后有尾随字节（Codex / 注入偶发双包）。
pub fn parse_json_value_lenient(body: &[u8]) -> Result<Value, serde_json::Error> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    let mut de = serde_json::Deserializer::from_slice(body);
    Value::deserialize(&mut de)
}

/// 把任意 arguments 表示收成「合法 JSON 对象字符串」。
///
/// - 已是对象 JSON → 原样（或去掉尾随垃圾后的首个值）
/// - 数组 / 标量 → 包进 `{"input": ...}`，避免上游要求 object 时 400
/// - 完全不可解析 → `{"input":"<原文字符串>"}`
pub fn canonicalize_tool_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    if let Some(value) = parse_first_json_value(trimmed) {
        return match value {
            Value::Object(_) => value.to_string(),
            other => json!({ "input": other }).to_string(),
        };
    }
    // 非 JSON：整段当字符串输入，保证仍是对象。
    json!({ "input": trimmed }).to_string()
}

fn parse_first_json_value(text: &str) -> Option<Value> {
    let mut de = serde_json::Deserializer::from_str(text);
    Value::deserialize(&mut de).ok()
}

/// 净化 messages 里所有 tool_calls.arguments，并合并连续 assistant 工具消息。
pub fn sanitize_chat_tool_history(messages: &mut Vec<Value>) {
    let fixed = canonicalize_arguments_in_messages(messages);
    if fixed > 0 {
        warn!(fixed, "canonicalized tool_calls.function.arguments before upstream chat");
    }
    let merged = merge_consecutive_assistant_tool_messages(messages);
    if merged > 0 {
        warn!(merged, "merged consecutive assistant tool_calls messages");
    }
    let demoted = demote_still_invalid_tool_calls(messages);
    if demoted > 0 {
        warn!(demoted, "demoted still-invalid tool_calls to assistant text");
    }
}

fn canonicalize_arguments_in_messages(messages: &mut [Value]) -> u32 {
    let mut fixed = 0u32;
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
            let Some(raw) = function.get("arguments").map(arguments_to_raw_string) else {
                function.insert("arguments".to_string(), json!("{}"));
                fixed += 1;
                continue;
            };
            let canon = canonicalize_tool_arguments(&raw);
            if canon != raw {
                function.insert("arguments".to_string(), Value::String(canon));
                fixed += 1;
            } else if !matches!(function.get("arguments"), Some(Value::String(_))) {
                // 上游只要字符串字段，对象形式也要 stringify。
                function.insert("arguments".to_string(), Value::String(canon));
                fixed += 1;
            }
        }
    }
    fixed
}

fn arguments_to_raw_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// 连续「仅含 tool_calls」的 assistant 消息合并为一条（Chat Completions 惯用形态）。
fn merge_consecutive_assistant_tool_messages(messages: &mut Vec<Value>) -> u32 {
    if messages.len() < 2 {
        return 0;
    }
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    let mut merged = 0u32;
    for msg in messages.drain(..) {
        let can_merge = is_assistant_tool_only(&msg)
            && out
                .last()
                .map(is_assistant_tool_only)
                .unwrap_or(false);
        if can_merge {
            if let (Some(prev), Some(incoming)) = (out.last_mut(), msg.as_object()) {
                let prev_calls = prev
                    .as_object_mut()
                    .and_then(|o| o.get_mut("tool_calls"))
                    .and_then(|v| v.as_array_mut());
                let new_calls = incoming.get("tool_calls").and_then(Value::as_array);
                if let (Some(prev_calls), Some(new_calls)) = (prev_calls, new_calls) {
                    prev_calls.extend(new_calls.iter().cloned());
                    merged += 1;
                    continue;
                }
            }
        }
        out.push(msg);
    }
    *messages = out;
    merged
}

fn is_assistant_tool_only(msg: &Value) -> bool {
    let Some(obj) = msg.as_object() else {
        return false;
    };
    if obj.get("role").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    let has_calls = obj
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if !has_calls {
        return false;
    }
    match obj.get("content") {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(parts)) => parts.is_empty(),
        _ => false,
    }
}

/// 规范化后仍非法的 call：降级成 assistant 文本，避免整轮 400。
fn demote_still_invalid_tool_calls(messages: &mut Vec<Value>) -> u32 {
    let mut demoted = 0u32;
    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };
        if obj.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(Value::Array(calls)) = obj.get_mut("tool_calls") else {
            continue;
        };
        let mut kept = Vec::new();
        let mut notes = Vec::new();
        for call in calls.drain(..) {
            let args = call
                .pointer("/function/arguments")
                .map(arguments_to_raw_string)
                .unwrap_or_default();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            if arguments_is_object_json(&args) {
                kept.push(call);
            } else {
                notes.push(format!(
                    "[invalid tool_call dropped: {name} arguments={args}]"
                ));
                demoted += 1;
            }
        }
        if kept.is_empty() {
            obj.remove("tool_calls");
        } else {
            obj.insert("tool_calls".to_string(), Value::Array(kept));
        }
        if !notes.is_empty() {
            let extra = notes.join("\n");
            match obj.get_mut("content") {
                Some(Value::String(s)) if !s.is_empty() => {
                    s.push('\n');
                    s.push_str(&extra);
                }
                Some(Value::Null) | None => {
                    obj.insert("content".to_string(), Value::String(extra));
                }
                Some(Value::String(s)) => *s = extra,
                _ => {
                    obj.insert("content".to_string(), Value::String(extra));
                }
            }
        }
    }
    demoted
}

fn arguments_is_object_json(raw: &str) -> bool {
    matches!(parse_first_json_value(raw.trim()), Some(Value::Object(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_strips_trailing_garbage() {
        let raw = r#"{"command":"ls"}trailing"#;
        let out = canonicalize_tool_arguments(raw);
        assert_eq!(out, r#"{"command":"ls"}"#);
        assert!(arguments_is_object_json(&out));
    }

    #[test]
    fn canonicalize_wraps_non_object() {
        assert_eq!(
            canonicalize_tool_arguments(r#""just-a-string""#),
            r#"{"input":"just-a-string"}"#
        );
        assert_eq!(
            canonicalize_tool_arguments("not-json-at-all"),
            r#"{"input":"not-json-at-all"}"#
        );
    }

    #[test]
    fn lenient_parse_ignores_trailing() {
        let body = br#"{"model":"x","input":"hi"}{"extra":1}"#;
        let v = parse_json_value_lenient(body).unwrap();
        assert_eq!(v["model"], "x");
    }

    #[test]
    fn merges_consecutive_assistant_tool_messages() {
        let mut messages = vec![
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c1",
                    "type": "function",
                    "function": {"name": "exec", "arguments": "{\"cmd\":\"a\"}"}
                }]
            }),
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c2",
                    "type": "function",
                    "function": {"name": "exec", "arguments": "{\"cmd\":\"b\"}extra"}
                }]
            }),
            json!({"role": "tool", "tool_call_id": "c1", "content": "ok"}),
        ];
        sanitize_chat_tool_history(&mut messages);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(
            messages[0]["tool_calls"][1]["function"]["arguments"],
            r#"{"cmd":"b"}"#
        );
    }
}
