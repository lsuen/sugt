//! `openai_responses_adapter` 模块单元测试（由 openai_responses_adapter.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use super::*;
use futures_util::{stream, StreamExt};

#[test]
fn ark_code_model_triggers_native_but_keeps_configured_base() {
    assert!(should_try_native_responses(
        "https://api-inference.modelscope.cn/v1",
        "ark-code-latest"
    ));
    let base =
        native_responses_base_url("https://api-inference.modelscope.cn/v1", "ark-code-latest");
    assert_eq!(base, "https://api-inference.modelscope.cn/v1");
}

#[test]
fn volcengine_plan_and_coding_keep_configured_base() {
    assert!(provider_supports_native_responses(
        "https://ark.cn-beijing.volces.com/api/coding/v3"
    ));
    assert!(provider_supports_native_responses(
        "https://ark.cn-beijing.volces.com/api/plan/v3"
    ));
    assert!(!provider_supports_native_responses(
        "https://api-inference.modelscope.cn/v1"
    ));
    assert_eq!(
        native_responses_base_url(
            "https://ark.cn-beijing.volces.com/api/plan/v3",
            "ark-code-latest"
        ),
        "https://ark.cn-beijing.volces.com/api/plan/v3"
    );
    assert_eq!(
        native_responses_base_url(
            "https://ark.cn-beijing.volces.com/api/coding/v3",
            "ark-code-latest"
        ),
        "https://ark.cn-beijing.volces.com/api/coding/v3"
    );
}

#[test]
fn native_responses_skipped_when_tools_declared() {
    let plain = Bytes::from(r#"{"model":"ark-code-latest","input":"hi","stream":true}"#);
    let with_tools = Bytes::from(
        r#"{"model":"ark-code-latest","input":"hi","stream":true,"tools":[{"type":"function","name":"shell"}]}"#,
    );
    let base = "https://ark.cn-beijing.volces.com/api/plan/v3";
    assert!(!responses_body_declares_tools(&plain));
    assert!(responses_body_declares_tools(&with_tools));
    assert!(should_try_native_responses_for_body(
        base,
        "ark-code-latest",
        &plain
    ));
    assert!(!should_try_native_responses_for_body(
        base,
        "ark-code-latest",
        &with_tools
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
    assert!(
        all.contains("\"input_tokens\":100"),
        "usage after finish must be kept: {all}"
    );
    assert!(
        all.contains("\"output_tokens\":7"),
        "usage after finish must be kept: {all}"
    );
    assert!(
        all.contains("\"total_tokens\":107"),
        "Codex requires total_tokens: {all}"
    );
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
    let mapped =
        chat_sse_to_responses_sse_with_meta(upstream, "m".to_string(), ChatToolMeta::default(), 42);
    let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
    let all = chunks
        .iter()
        .map(|b| String::from_utf8_lossy(b))
        .collect::<Vec<_>>()
        .join("");
    assert!(all.contains("\"input_tokens\":42"), "fallback input: {all}");
    assert!(all.contains("\"output_tokens\":"), "fallback output: {all}");
    assert!(
        !all.contains("\"output_tokens\":0"),
        "output should be estimated: {all}"
    );
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
    // namespace 展平：避免多个 namespace 下的同名工具互相覆盖，也避免上游不认 namespace 类型。
    // 回程必须还原成客户端原始名，否则 Codex 会返回 unsupported call。
    assert!(meta.custom_names.contains("functions__exec"));
    assert_eq!(
        meta.flattened_names
            .get("functions__exec")
            .map(String::as_str),
        Some("exec")
    );
    assert_eq!(restore_tool_name("functions__exec", &meta), "exec");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.pointer("/function/name").and_then(|n| n.as_str()))
        .collect();
    assert!(names.contains(&"functions__exec"));
    assert!(names.contains(&"functions__get_weather"));
    assert_eq!(
        restore_tool_name("functions__get_weather", &meta),
        "get_weather"
    );
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
    let name = assistant
        .pointer("/tool_calls/0/function/name")
        .and_then(|n| n.as_str());
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
    let name = v
        .pointer("/messages/0/tool_calls/0/function/name")
        .and_then(|n| n.as_str())
        .unwrap_or("");
    assert!(
        !name.is_empty(),
        "empty tool name must be patched, got {name:?}"
    );
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
    assert_eq!(
        v.pointer("/tools/0/function/name").and_then(|n| n.as_str()),
        Some("shell")
    );
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

/// 空流（只有 [DONE]）必须 failed，不能伪装 completed。
#[tokio::test]
async fn empty_sse_stream_becomes_failed_not_completed() {
    let upstream = stream::iter(vec![Ok(Bytes::from("data: [DONE]\n\n"))]);
    let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
    let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
    let all = chunks
        .iter()
        .map(|b| String::from_utf8_lossy(b))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        all.contains("response.failed"),
        "空流必须 failed：{all}"
    );
    assert!(
        !all.contains("response.completed"),
        "空流不能伪装 completed：{all}"
    );
}

/// 上游半路 Err：发 failed_events，不要把 Err 抛给客户端当连接断开。
#[tokio::test]
async fn midstream_upstream_error_yields_failed_events() {
    let boom = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(50))
        // 回环探测必须直连：环境 HTTP_PROXY 会把连不上的端口转成 502 响应，
        // 让 expect_err 拿到 Ok，全量套件在办公代理环境下假红。
        .no_proxy()
        .build()
        .unwrap()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("连不上的端口应立即失败");
    let upstream = stream::iter(vec![
        Ok(Bytes::from(
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
        )),
        Err(boom),
    ]);
    let mapped = chat_sse_to_responses_sse(upstream, "m".to_string());
    let mut saw_failed = false;
    let mut saw_io_err = false;
    let mut mapped = mapped;
    while let Some(item) = mapped.next().await {
        match item {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if text.contains("response.failed") {
                    saw_failed = true;
                }
            }
            Err(_) => saw_io_err = true,
        }
    }
    assert!(saw_failed, "半路截断应发 response.failed");
    assert!(!saw_io_err, "不应再把 Err 抛给客户端");
}

/// 正文已发后，工具 output_index 必须连续（message=0 → tool=1）。
#[tokio::test]
async fn tool_output_index_is_contiguous_after_message() {
    let upstream = stream::iter(vec![
        Ok(Bytes::from(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        )),
        Ok(Bytes::from(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"shell_command\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
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
    // 找 function_call 的 output_item.added
    let mut message_idx = None;
    let mut tool_idx = None;
    for line in all.lines() {
        let Some(payload) = line.strip_prefix("data: ") else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("response.output_item.added") {
            continue;
        }
        let idx = v.get("output_index").and_then(Value::as_u64);
        let typ = v.pointer("/item/type").and_then(Value::as_str);
        match typ {
            Some("message") => message_idx = idx,
            Some("function_call") => tool_idx = idx,
            _ => {}
        }
    }
    assert_eq!(message_idx, Some(0), "message 应为 0：{all}");
    assert_eq!(tool_idx, Some(1), "tool 应紧跟为 1，不能跳号：{all}");
}

/// 命令模式：正文标记合成 function_call，并带用户可读提示。
#[test]
fn command_mode_synthesizes_function_call_from_markers() {
    let upstream = r#"{"id":"chat1","choices":[{"message":{"role":"assistant","content":"执行一下\n[[SUGT_CMD]]{\"name\":\"shell_command\",\"arguments\":{\"command\":\"pwd\"}}[[/SUGT_CMD]]"}}]}"#;
    let mut meta = ChatToolMeta::default();
    meta.command_mode = true;
    let bytes = chat_json_to_responses_body_with_meta(upstream, "m", &meta).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["status"], "completed");
    let output = v["output"].as_array().unwrap();
    assert!(
        output.iter().any(|i| {
            i["type"] == "function_call"
                && i["name"] == "shell_command"
                && i["arguments"].as_str().unwrap_or("").contains("pwd")
        }),
        "应合成 function_call：{output:?}"
    );
    let text = v["output_text"].as_str().unwrap_or("");
    assert!(
        text.contains(crate::command_mode::USER_NOTICE),
        "应提示已切换命令模式：{text}"
    );
    assert!(!text.contains("[[SUGT_CMD]]"), "可见正文应去掉标记：{text}");
}

/// Codex 请求未指定任何 max_output_tokens / max_completion_tokens 时，
/// Chat Completions 转发体应有安全默认，避免上游用小默认截断 tool_call。
#[test]
fn fills_default_max_tokens_when_omitted() {
    let body = Bytes::from(
        r#"{"model":"codex","stream":true,"input":[{"type":"input_text","text":"run a long shell command"}],"tools":[{"type":"function","name":"shell_command","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}}]}"#,
    );
    let (chat_body, _) = responses_to_chat_with_meta(&body, "upstream-model").unwrap();
    let v: Value = serde_json::from_slice(&chat_body).unwrap();
    // 必须带上我们的安全默认值
    assert_eq!(v["max_tokens"], super::DEFAULT_CODEX_MAX_TOKENS,
        "Codex 未指定 max 时应填充安全默认值 {}，实际 {:?}",
        super::DEFAULT_CODEX_MAX_TOKENS, v["max_tokens"]);
}

/// Codex 显式传了 max_output_tokens 时，应原样映射到 Chat max_tokens。
#[test]
fn respects_explicit_max_output_tokens() {
    let body = Bytes::from(
        r#"{"model":"codex","stream":true,"max_output_tokens":12000,"input":[{"type":"input_text","text":"hi"}],"tools":[{"type":"function","name":"shell_command","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}}]}"#,
    );
    let (chat_body, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&chat_body).unwrap();
    assert_eq!(v["max_tokens"], 12000,
        "应透传 Codex 指定的 max_output_tokens");
}

/// Codex 显式传了更小的 max_completion_tokens 时，应优先尊重它。
#[test]
fn respects_smaller_max_completion_tokens_over_default() {
    let body = Bytes::from(
        r#"{"model":"codex","stream":true,"max_completion_tokens":500,"input":[{"type":"input_text","text":"hi"}],"tools":[{"type":"function","name":"shell_command","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}}]}"#,
    );
    let (chat_body, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&chat_body).unwrap();
    assert_eq!(v["max_tokens"], 500,
        "应尊重 Codex 显式指定的小值，不要用默认值覆盖");
}

/// Codex 显式传 null ("让上游决定") 时，视为未指定，填安全默认。
#[test]
fn treats_null_max_output_tokens_as_unspecified() {
    let body = Bytes::from(
        r#"{"model":"codex","stream":true,"max_output_tokens":null,"input":[{"type":"input_text","text":"hi"}],"tools":[{"type":"function","name":"shell_command","parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}}]}"#,
    );
    let (chat_body, _) = responses_to_chat_with_meta(&body, "upstream").unwrap();
    let v: Value = serde_json::from_slice(&chat_body).unwrap();
    assert_eq!(v["max_tokens"], super::DEFAULT_CODEX_MAX_TOKENS,
        "null 应视为未指定，填充安全默认而非透传 null");
}
