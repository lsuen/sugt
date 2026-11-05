//! Codex / Responses 转换的边界回归测试：覆盖静默降级、异常分片与非 UTF-8 边界。
//! 作者：sun / 孙文龙
//!
//! 这些用例刻意模拟真实上游与 Codex 客户端的「非理想」输入：字段白名单之外的
//! Responses 参数、只有 index+arguments 的工具分片、跨 chunk 的多字节字符、
//! 上游返回非法 JSON / 空 body。任何一条退化都不应再以「伪装成功」收场。

use bytes::Bytes;
use futures_util::{stream, StreamExt};
use serde_json::Value;
use sugt_lib::openai_responses_adapter::{
    chat_json_to_responses_body_with_meta, chat_sse_to_responses_sse_with_meta,
    responses_to_chat_with_meta, ChatToolMeta,
};

/// 把若干上游 SSE chunk 送进真实转换流，收集面向客户端的完整事件文本。
fn map_sse(chunks: Vec<Bytes>) -> String {
    let input = stream::iter(
        chunks
            .into_iter()
            .map(|bytes| Ok::<Bytes, reqwest::Error>(bytes))
            .collect::<Vec<_>>(),
    );
    let mapped = chat_sse_to_responses_sse_with_meta(
        input,
        "upstream-model".to_string(),
        ChatToolMeta::default(),
        0,
    );
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let mut mapped = mapped;
        let mut out = String::new();
        while let Some(item) = mapped.next().await {
            if let Ok(bytes) = item {
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        out
    })
}

fn sse(payload: &str) -> Bytes {
    Bytes::from(format!("data: {}\n\n", payload))
}

fn event_types(sse_text: &str) -> Vec<String> {
    sse_text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .filter_map(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn output_items(sse_text: &str) -> Vec<Value> {
    sse_text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|payload| serde_json::from_str::<Value>(payload).ok())
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("response.completed"))
        .filter_map(|value| value.pointer("/response/output").cloned())
        .filter_map(|output| output.as_array().cloned())
        .flatten()
        .collect()
}

// ---------------------------------------------------------------- 请求侧降级

#[test]
fn max_output_tokens_is_mapped_to_chat_max_tokens() {
    let body =
        Bytes::from(r#"{"model":"official","input":"hi","max_output_tokens":4096,"stream":false}"#);
    let (out, _meta) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let chat: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        chat["max_tokens"], 4096,
        "Responses 的 max_output_tokens 必须映射成 Chat 的 max_tokens，不能被静默丢弃：{chat:?}"
    );
}

#[test]
fn dropped_responses_fields_are_observable_instead_of_silent() {
    let body = Bytes::from(
        r#"{"model":"official","input":"hi","previous_response_id":"resp_1","store":true,"reasoning":{"effort":"high"},"include":["file_search_call.results"],"stream":false}"#,
    );
    let (_out, meta) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let observed = format!("{meta:?}");
    for field in ["previous_response_id", "store", "reasoning", "include"] {
        assert!(
            observed.contains(field),
            "被降级的 Responses 字段 {field} 必须出现在可观测元数据里：{observed}"
        );
    }
}

#[test]
fn instructions_item_becomes_system_message_not_user_json() {
    let body = Bytes::from(
        r#"{"input":[{"type":"instructions","text":"be terse"},{"type":"message","role":"user","content":"hi"}]}"#,
    );
    let (out, _meta) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let chat: Value = serde_json::from_slice(&out).unwrap();
    let messages = chat["messages"].as_array().expect("messages");
    assert!(
        messages
            .iter()
            .any(|m| m["role"] == "system" && m["content"].as_str() == Some("be terse")),
        "input 里的 instructions item 应转成 system 消息：{messages:?}"
    );
    assert!(
        !messages.iter().any(|m| m["content"]
            .as_str()
            .is_some_and(|s| s.contains("\"type\":\"instructions\""))),
        "不能把整段 item JSON 塞进 user 文本：{messages:?}"
    );
}

#[test]
fn unrecognized_input_item_uses_plain_text_not_serialized_json() {
    let body = Bytes::from(
        r#"{"input":[{"type":"custom","text":"some context","meta":{"source":"local"}}]}"#,
    );
    let (out, _meta) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let chat: Value = serde_json::from_slice(&out).unwrap();
    let content = chat["messages"][0]["content"].as_str().unwrap_or("");
    assert_eq!(
        content, "some context",
        "未识别 item 应提取文本，不能把结构序列化成 user 文本：{content}"
    );
}

// ---------------------------------------------------------- 非流式上游异常兜底

#[test]
fn invalid_upstream_json_becomes_responses_shaped_failure() {
    let bytes = chat_json_to_responses_body_with_meta(
        "this is not json",
        "upstream-model",
        &ChatToolMeta::default(),
    )
    .unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        response["status"], "failed",
        "上游非法 JSON 不能伪装成 completed 空回复：{response:?}"
    );
    assert!(
        response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "失败响应必须带 Responses 形状的 error 对象：{response:?}"
    );
}

#[test]
fn empty_upstream_body_becomes_responses_shaped_failure() {
    let bytes =
        chat_json_to_responses_body_with_meta("", "upstream-model", &ChatToolMeta::default())
            .unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        response["status"], "failed",
        "空 body 不能伪装成功：{response:?}"
    );
    assert!(
        response["error"].is_object(),
        "缺少 error 对象：{response:?}"
    );
}

// -------------------------------------------------------------- 流式异常分片

#[test]
fn long_multibyte_payload_does_not_panic() {
    let text = "中".repeat(300);
    let payload = format!(r#"{{"choices":[{{"delta":{{"content":"{}"}}}}]}}"#, text);
    let out = map_sse(vec![sse(&payload), Bytes::from("data: [DONE]\n\n")]);
    assert!(
        event_types(&out).contains(&"response.completed".to_string()),
        "长中文分片必须完整走完 SSE 生命周期：{}",
        &out[..out.len().min(400)]
    );
}

#[test]
fn multibyte_char_split_across_chunks_is_preserved() {
    let full = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"中文测试\"}}}}]}}\n\n");
    let split = full.find('中').expect("中文字符位置") + 1;
    let out = map_sse(vec![
        Bytes::copy_from_slice(&full.as_bytes()[..split]),
        Bytes::copy_from_slice(&full.as_bytes()[split..]),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    assert!(
        !out.contains('\u{FFFD}'),
        "跨 chunk 的多字节字符不能被替换成 U+FFFD：{}",
        &out[..out.len().min(500)]
    );
    assert!(
        out.contains("中文测试"),
        "正文应完整保留：{}",
        &out[..out.len().min(500)]
    );
}

#[test]
fn tool_delta_without_name_keeps_event_order() {
    let out = map_sse(vec![
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"cmd\":"}}]}}]}"#,
        ),
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"shell_command"}}]}}]}"#,
        ),
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls\"}"}}]}}]}"#,
        ),
        sse(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    let types = event_types(&out);
    let added = types
        .iter()
        .position(|t| t == "response.output_item.added")
        .expect("必须有 output_item.added");
    let first_delta = types
        .iter()
        .position(|t| t == "response.function_call_arguments.delta")
        .expect("必须有 arguments.delta");
    assert!(
        added < first_delta,
        "先发 output_item.added 再发 arguments.delta，不能顺序倒置：{types:?}"
    );
    let items = output_items(&out);
    assert!(
        items
            .iter()
            .any(|i| i["type"] == "function_call" && i["name"] == "shell_command"),
        "最终工具名必须还原：{items:?}"
    );
}

#[test]
fn anonymous_tool_call_gets_placeholder_and_full_events() {
    let out = map_sse(vec![
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_x","type":"function","function":{"arguments":"{}"}}]}}]}"#,
        ),
        sse(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    let types = event_types(&out);
    assert!(
        types.contains(&"response.output_item.added".to_string()),
        "即使上游从不给 name，也必须补发 output_item.added：{types:?}"
    );
    assert!(
        types.contains(&"response.function_call_arguments.done".to_string()),
        "参数结束事件不能缺：{types:?}"
    );
    let items = output_items(&out);
    let call = items
        .iter()
        .find(|i| i["type"] == "function_call")
        .expect("工具调用不能丢");
    assert!(
        call["name"].as_str().is_some_and(|s| !s.is_empty()),
        "不能输出空工具名：{call:?}"
    );
}

#[test]
fn out_of_order_parallel_tool_calls_keep_their_own_arguments() {
    let out = map_sse(vec![
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"tool_b","arguments":"{\"b\":1}"}}]}}]}"#,
        ),
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"tool_a","arguments":"{\"a\":2}"}}]}}]}"#,
        ),
        sse(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    let items = output_items(&out);
    let names: Vec<&str> = items.iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names.len(), 2, "两个并行工具都要在：{items:?}");
    assert!(names.contains(&"tool_a") && names.contains(&"tool_b"));
    assert!(
        items
            .iter()
            .any(|i| i["name"] == "tool_a" && i["arguments"] == "{\"a\":2}")
            && items
                .iter()
                .any(|i| i["name"] == "tool_b" && i["arguments"] == "{\"b\":1}"),
        "参数不能串味：{items:?}"
    );
}

#[test]
fn invalid_arguments_json_does_not_panic_or_drop_tool() {
    let out = map_sse(vec![
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_bad","type":"function","function":{"name":"shell_command","arguments":"not-json"}}]}}]}"#,
        ),
        sse(r#"{"choices":[{"finish_reason":"tool_calls"}]}"#),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    assert!(
        event_types(&out).contains(&"response.completed".to_string()),
        "非法 arguments 不能中断整条流"
    );
    assert!(
        output_items(&out)
            .iter()
            .any(|i| i["type"] == "function_call"),
        "工具调用不能因为参数非法而丢失"
    );
}

#[test]
fn single_invalid_json_chunk_is_skipped_without_killing_stream() {
    let out = map_sse(vec![
        sse("this-is-not-json"),
        sse(r#"{"choices":[{"delta":{"content":"still alive"}}]}"#),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    assert!(
        out.contains("still alive"),
        "单个坏分片不能吞掉后续正常分片：{}",
        &out[..out.len().min(500)]
    );
    assert!(event_types(&out).contains(&"response.completed".to_string()));
}

// ------------------------------------------------------------ 保护性回归夹具

#[test]
fn healthy_tool_call_still_emits_standard_sequence() {
    let out = map_sse(vec![
        sse(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_ok","type":"function","function":{"name":"shell_command","arguments":"{\"command\":\"ls\"}"}}]}}]}"#,
        ),
        sse(
            r#"{"choices":[{"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#,
        ),
        Bytes::from("data: [DONE]\n\n"),
    ]);
    let types = event_types(&out);
    for required in [
        "response.created",
        "response.in_progress",
        "response.output_item.added",
        "response.function_call_arguments.delta",
        "response.function_call_arguments.done",
        "response.output_item.done",
        "response.completed",
    ] {
        assert!(
            types.contains(&required.to_string()),
            "缺少 {required}：{types:?}"
        );
    }
    assert_eq!(
        output_items(&out)[0]["name"],
        "shell_command",
        "标准路径不能被本轮改动破坏"
    );
}

#[test]
fn normal_responses_request_is_unchanged() {
    let body = Bytes::from(
        r#"{"model":"official","instructions":"be helpful","input":"hi","temperature":0.2,"stream":false}"#,
    );
    let (out, _meta) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let chat: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(chat["messages"][0]["role"], "system");
    assert_eq!(chat["messages"][1]["content"], "hi");
    assert_eq!(chat["temperature"], 0.2);
    assert_eq!(chat["model"], "upstream-model");
}

#[test]
fn error_shaped_upstream_body_is_still_passthrough() {
    let upstream = r#"{"error":{"message":"rate limited","type":"rate_limit_error"}}"#;
    let bytes =
        chat_json_to_responses_body_with_meta(upstream, "upstream-model", &ChatToolMeta::default())
            .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["error"]["message"], "rate limited");
}
