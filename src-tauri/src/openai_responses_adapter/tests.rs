//! `openai_responses_adapter` 模块单元测试（由 openai_responses_adapter.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

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
fn empty_function_call_name_gets_fallback() {
    // 回归：name:"" 时 unwrap_or 不会触发，上游报 tool_calls[0].function.name empty
    let body = Bytes::from(
        r#"{
          "model":"gpt-4",
          "input":[
            {"type":"function_call","call_id":"c1","name":"","arguments":"{}"},
            {"type":"function_call_output","call_id":"c1","output":"ok"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"你好"}]}
          ]
        }"#,
    );
    let (out, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let msgs = v["messages"].as_array().unwrap();
    let assistant = msgs
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .expect("assistant tool call message");
    let name = assistant.pointer("/tool_calls/0/function/name").and_then(|n| n.as_str());
    assert_eq!(name, Some("function"));
    assert!(name.unwrap().len() > 0);
}

#[test]
fn sanitizes_empty_tool_calls_on_chat_shaped_history() {
    let body = Bytes::from(
        r#"{
          "model":"gpt-4",
          "input":[
            {"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"c1","content":"ok"},
            {"role":"user","content":"你好"}
          ]
        }"#,
    );
    let (out, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let name = v.pointer("/messages/0/tool_calls/0/function/name")
        .and_then(|n| n.as_str())
        .unwrap_or("");
    assert!(!name.is_empty(), "empty tool name must be patched, got {name:?}");
    assert_eq!(name, "unknown_tool");
}

#[test]
fn drops_tool_choice_with_empty_function_name() {
    let body = Bytes::from(
        r#"{
          "model":"gpt-4",
          "tool_choice":{"type":"function","function":{"name":""}},
          "tools":[{"type":"function","name":"shell","parameters":{"type":"object","properties":{}}}],
          "input":"hi"
        }"#,
    );
    let (out, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["tool_choice"], json!("auto"));
}

#[test]
fn accepts_chat_nested_function_tool_schema() {
    let body = Bytes::from(
        r#"{
          "model":"gpt-4",
          "tools":[{"type":"function","function":{"name":"shell","description":"run","parameters":{"type":"object","properties":{}}}}],
          "input":"hi"
        }"#,
    );
    let (out, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.pointer("/tools/0/function/name").and_then(|n| n.as_str()), Some("shell"));
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
