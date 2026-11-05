//! 协议互转 round-trip 集成测试。
//! 作者：sun / 孙文龙
//!
//! 覆盖网关两条真实链路的「请求进 → 转换 → 上游响应 → 反向转换」：
//!   1. Responses 请求 → Chat 请求 → Chat 响应 → Responses 响应
//!   2. Anthropic 请求 → OpenAI 请求 → OpenAI 响应 → Anthropic 响应
//! 与文件内单元测试的区别：这里跨模块调用公开转换入口，验证字段在完整链路上不丢失。

use bytes::Bytes;
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};
use sugt_lib::anthropic_adapter::{anthropic_to_openai, openai_json_to_anthropic_json};
use sugt_lib::openai_responses_adapter::{
    chat_json_to_responses_body, chat_json_to_responses_body_with_meta,
    chat_sse_to_responses_sse_with_meta, responses_to_chat_with_meta, ChatToolMeta,
};

fn parse(bytes: &Bytes) -> Value {
    serde_json::from_slice(bytes).expect("转换结果必须是合法 JSON")
}

fn json_bytes(value: &Value) -> Bytes {
    Bytes::from(serde_json::to_vec(value).expect("测试输入必须是可序列化的 JSON"))
}

#[test]
fn responses_to_chat_and_back_keeps_tools_text_and_usage() {
    let request = json!({
        "model": "gpt-4",
        "instructions": "简洁回答",
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "列出当前目录" }]
            }
        ],
        "tools": [
            {
                "type": "function",
                "name": "get_weather",
                "description": "run",
                "parameters": { "type": "object", "properties": {} }
            }
        ],
        "tool_choice": { "type": "function", "function": { "name": "get_weather" } },
        "stream": false
    });

    let (chat_request_bytes, meta) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat_request = parse(&chat_request_bytes);

    assert_eq!(chat_request["model"], "upstream-model");
    assert_eq!(chat_request["stream"], false);
    assert_eq!(chat_request["messages"][0]["role"], "system");
    assert_eq!(chat_request["messages"][0]["content"], "简洁回答");
    assert!(
        chat_request.to_string().contains("列出当前目录"),
        "用户正文在 Chat 请求中丢失：{chat_request}"
    );
    assert_eq!(
        chat_request
            .pointer("/tools/0/function/name")
            .and_then(Value::as_str),
        Some("get_weather")
    );
    assert_eq!(
        chat_request
            .pointer("/tool_choice/function/name")
            .and_then(Value::as_str),
        Some("get_weather"),
        "非空工具名的 tool_choice 不应被净化掉"
    );
    assert!(meta.custom_names.is_empty());

    let chat_response = json!({
        "id": "chatcmpl-1",
        "model": "upstream-model",
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "get_weather", "arguments": "{\"cmd\":\"ls\"}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 7 }
    })
    .to_string();

    let responses = parse(
        &chat_json_to_responses_body_with_meta(&chat_response, "upstream-model", &meta).unwrap(),
    );
    assert_eq!(responses["object"], "response");
    assert_eq!(responses["status"], "completed");
    let output = responses["output"].as_array().expect("output 应为数组");
    let call = output
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap_or_else(|| {
            panic!("Chat tool_calls 未还原为 function_call，实际 responses={responses}")
        });
    assert_eq!(call["name"], "get_weather");
    assert_eq!(call["call_id"], "call_1");
    assert_eq!(call["status"], "completed");
    assert!(call["arguments"].as_str().unwrap().contains("ls"));
    assert_eq!(responses["usage"]["total_tokens"], 19);
}

#[test]
fn anthropic_to_openai_and_back_keeps_tool_calls_and_stop_reason() {
    let request = json!({
        "model": "claude-3",
        "system": "你是助手",
        "max_tokens": 128,
        "temperature": null,
        "messages": [
            { "role": "user", "content": [{ "type": "text", "text": "运行 ls" }] },
            {
                "role": "assistant",
                "content": [
                    { "type": "tool_use", "id": "t1", "name": "get_weather", "input": { "cmd": "ls" } }
                ]
            },
            {
                "role": "user",
                "content": [
                    { "type": "tool_result", "tool_use_id": "t1", "content": "file.txt" }
                ]
            }
        ],
        "tools": [
            {
                "name": "get_weather",
                "description": "run",
                "input_schema": { "type": "object", "properties": {} }
            }
        ],
        "tool_choice": { "type": "tool", "name": "get_weather" },
        "stream": false
    });

    let openai_request =
        parse(&anthropic_to_openai(json_bytes(&request), "upstream-model").unwrap());

    assert_eq!(openai_request["model"], "upstream-model");
    assert_eq!(openai_request["messages"][0]["role"], "system");
    assert_eq!(openai_request["messages"][0]["content"], "你是助手");
    assert_eq!(openai_request["max_tokens"], 128);
    assert!(
        openai_request.get("temperature").is_none(),
        "显式 null 参数不应透传给严格要求类型的上游"
    );
    assert_eq!(
        openai_request
            .pointer("/tools/0/function/name")
            .and_then(Value::as_str),
        Some("get_weather")
    );
    assert_eq!(
        openai_request
            .pointer("/tool_choice/function/name")
            .and_then(Value::as_str),
        Some("get_weather")
    );
    assert!(
        openai_request.to_string().contains("t1"),
        "历史 tool_call_id 丢失"
    );
    assert!(
        openai_request.to_string().contains("file.txt"),
        "工具结果丢失"
    );

    let openai_response = json!({
        "id": "chatcmpl-2",
        "model": "upstream-model",
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": "已执行",
                "tool_calls": [{
                    "id": "t2",
                    "type": "function",
                    "function": { "name": "get_weather", "arguments": "{\"cmd\":\"pwd\"}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 20, "completion_tokens": 9 }
    })
    .to_string();

    let anthropic_response = openai_json_to_anthropic_json(openai_response.as_bytes()).unwrap();
    assert_eq!(anthropic_response["type"], "message");
    assert_eq!(anthropic_response["role"], "assistant");
    assert_eq!(anthropic_response["stop_reason"], "tool_use");
    assert_eq!(anthropic_response["usage"]["input_tokens"], 20);
    assert_eq!(anthropic_response["usage"]["output_tokens"], 9);
    let content = anthropic_response["content"]
        .as_array()
        .expect("content 应为数组");
    assert!(content
        .iter()
        .any(|block| block["type"] == "text" && block["text"] == "已执行"));
    let tool_use = content
        .iter()
        .find(|block| block["type"] == "tool_use")
        .expect("OpenAI tool_calls 应还原为 Anthropic tool_use");
    assert_eq!(tool_use["id"], "t2");
    assert_eq!(tool_use["name"], "get_weather");
    assert_eq!(tool_use["input"]["cmd"], "pwd");
}

#[test]
fn upstream_error_body_is_passthrough_not_reshaped() {
    let error_body = r#"{"error":{"message":"rate limited","type":"rate_limit_error"}}"#;
    let out = chat_json_to_responses_body_with_meta(
        error_body,
        "upstream-model",
        &ChatToolMeta::default(),
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(out.to_vec()).unwrap(),
        error_body,
        "上游错误体应原样返回，不能被伪装成成功的 Responses 对象"
    );
}

#[test]
fn empty_tool_name_never_survives_the_full_responses_path() {
    // 跨模块回归：空 name 的 Responses 历史项经 Chat 请求后不得留空，
    // 否则严格上游直接返回 400。
    let request = json!({
        "model": "gpt-4",
        "input": [
            {
                "type": "function_call",
                "call_id": "c1",
                "name": "",
                "arguments": "{}"
            },
            { "type": "function_call_output", "call_id": "c1", "output": "ok" },
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "继续" }]
            }
        ]
    });

    let (chat_request_bytes, _) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat_request = parse(&chat_request_bytes);
    let messages = chat_request["messages"].as_array().unwrap();
    for message in messages {
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let name = call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                assert!(!name.is_empty(), "空工具名泄漏到上游请求：{chat_request}");
            }
        }
    }
}

#[test]
fn codex_freeform_tool_name_restores_custom_tool_call_without_meta() {
    // 请求侧没有 custom 工具元数据时，响应侧按 Codex 常见 freeform 工具名兜底。
    let chat_response = json!({
        "id": "chatcmpl-3",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_9",
                    "type": "function",
                    "function": { "name": "shell", "arguments": "{\"cmd\":\"pwd\"}" }
                }]
            }
        }]
    })
    .to_string();

    let responses = parse(
        &chat_json_to_responses_body_with_meta(
            &chat_response,
            "upstream-model",
            &ChatToolMeta::default(),
        )
        .unwrap(),
    );
    let output = responses["output"].as_array().unwrap();
    let call = output
        .iter()
        .find(|item| item["type"] == "custom_tool_call")
        .unwrap_or_else(|| {
            panic!("freeform 工具名应还原为 custom_tool_call，实际 output={output:?}")
        });
    assert_eq!(call["name"], "shell");
    assert_eq!(call["call_id"], "call_9");
    assert!(output.iter().all(|item| item["type"] != "function_call"));
}

// ===== Codex 真实工具链路回归：以下用例先以失败态记录问题，再逐项修复 =====

fn user_message(text: &str) -> Value {
    json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    })
}

fn chat_tool_response(call_id: &str, name: &str, arguments: &str) -> String {
    json!({
        "id": "chatcmpl-tool",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": Value::Null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }]
            }
        }],
        "usage": { "prompt_tokens": 8, "completion_tokens": 4 }
    })
    .to_string()
}

#[test]
fn codex_shell_command_stays_a_function_call() {
    // OpenAI Codex prompting guide 明确 shell_command 是 function 工具；
    // 被判成 freeform custom_tool_call 时客户端解析失败并返回 aborted。
    let request = json!({
        "model": "gpt-5-codex",
        "input": [user_message("列出当前目录")],
        "tools": [{
            "type": "function",
            "name": "shell_command",
            "description": "Runs a shell command and returns its output.",
            "parameters": {
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }
        }],
        "stream": false
    });
    let (chat_bytes, meta) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat = parse(&chat_bytes);
    assert_eq!(
        chat.pointer("/tools/0/function/name")
            .and_then(Value::as_str),
        Some("shell_command")
    );

    let responses = parse(
        &chat_json_to_responses_body_with_meta(
            &chat_tool_response("call_shell", "shell_command", "{\"command\":\"ls\"}"),
            "upstream-model",
            &meta,
        )
        .unwrap(),
    );
    let output = responses["output"].as_array().expect("output 应为数组");
    let call = output
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap_or_else(|| panic!("shell_command 应还原为 function_call，实际 output={output:?}"));
    assert_eq!(call["name"], "shell_command");
    assert_eq!(call["arguments"], "{\"command\":\"ls\"}");
    assert!(output.iter().all(|item| item["type"] != "custom_tool_call"));
}

#[test]
fn namespace_tools_are_flattened_and_restored() {
    // Codex 把 MCP / 内置工具挂在 namespace 下；Chat 上游只认平铺 function。
    let request = json!({
        "input": [user_message("执行命令")],
        "tools": [{
            "type": "namespace",
            "name": "functions",
            "tools": [{
                "type": "function",
                "name": "exec_command",
                "description": "run a command",
                "parameters": {
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } }
                }
            }]
        }],
        "stream": false
    });
    let (chat_bytes, meta) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat = parse(&chat_bytes);
    let flat = chat
        .pointer("/tools/0/function/name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("namespace 工具未展平：{chat}"));
    assert_eq!(flat, "functions__exec_command");

    let responses = parse(
        &chat_json_to_responses_body_with_meta(
            &chat_tool_response("call_ns", flat, "{\"cmd\":\"pwd\"}"),
            "upstream-model",
            &meta,
        )
        .unwrap(),
    );
    let output = responses["output"].as_array().expect("output 应为数组");
    let call = output
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap_or_else(|| panic!("展平工具回程未生成 function_call：{output:?}"));
    assert_eq!(
        call["name"], "exec_command",
        "回程必须还原客户端认识的原始工具名，而不是把展平名直接丢给 Codex"
    );
}

#[test]
fn mcp_style_tool_output_stays_structured() {
    // MCP 动态工具以非标准 item 类型出现；转成 user 纯文本会直接断掉工具闭环。
    let request = json!({
        "input": [
            user_message("读取文件"),
            {
                "type": "mcp_call",
                "call_id": "call_mcp_1",
                "name": "read_file",
                "arguments": "{\"path\":\"a.txt\"}"
            },
            {
                "type": "mcp_call_output",
                "call_id": "call_mcp_1",
                "output": "文件内容"
            }
        ],
        "stream": false
    });
    let (chat_bytes, _) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat = parse(&chat_bytes);
    let messages = chat["messages"].as_array().expect("messages 应为数组");
    let tool_result = messages
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == "call_mcp_1")
        .unwrap_or_else(|| panic!("工具结果被降级成纯文本：{messages:?}"));
    assert_eq!(tool_result["content"], "文件内容");
    assert!(messages.iter().any(|m| m["role"] == "assistant"));
}

#[test]
fn orphan_tool_calls_get_a_matching_tool_result() {
    // 历史回放里缺 function_call_output 时，Chat 上游会因孤儿 tool_calls 报 400。
    let request = json!({
        "input": [
            user_message("继续"),
            {
                "type": "function_call",
                "call_id": "call_orphan",
                "name": "get_weather",
                "arguments": "{}"
            }
        ],
        "stream": false
    });
    let (chat_bytes, _) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat = parse(&chat_bytes);
    let messages = chat["messages"].as_array().expect("messages 应为数组");
    assert!(
        messages
            .iter()
            .any(|m| m["role"] == "tool" && m["tool_call_id"] == "call_orphan"),
        "孤儿 tool_call 未补齐配对结果：{messages:?}"
    );
}

#[test]
fn unknown_tool_types_are_degraded_not_dropped() {
    let request = json!({
        "input": [user_message("搜索")],
        "tools": [{
            "type": "mcp",
            "name": "mcp__server__search",
            "description": "search docs",
            "parameters": {
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }
        }],
        "stream": false
    });
    let (chat_bytes, _) =
        responses_to_chat_with_meta(&json_bytes(&request), "upstream-model").unwrap();
    let chat = parse(&chat_bytes);
    assert_eq!(
        chat.pointer("/tools/0/function/name")
            .and_then(Value::as_str),
        Some("mcp__server__search"),
        "带 schema 的未识别工具应降级为 function，而不是静默丢弃：{chat}"
    );
}

#[test]
fn non_stream_response_has_codex_required_fields() {
    let chat_response = json!({
        "id": "chatcmpl-x",
        "choices": [{
            "finish_reason": "stop",
            "message": { "role": "assistant", "content": "ok" }
        }]
    })
    .to_string();
    let responses = parse(&chat_json_to_responses_body(&chat_response, "m").unwrap());
    assert!(responses.get("created_at").is_some(), "缺 created_at");
    assert_eq!(responses["error"], Value::Null, "error 应为 null");
    assert_eq!(
        responses["incomplete_details"],
        Value::Null,
        "incomplete_details 应为 null"
    );
    assert_eq!(responses["output_text"], "ok");
    assert_eq!(responses["store"], false);
}

#[tokio::test]
async fn sse_emits_in_progress_and_tool_argument_done() {
    let first = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_s","type":"function","function":{"name":"shell_command","arguments":"{\"command\":\"ls\"}"}}]}}]}"#;
    let upstream = stream::iter(vec![
        Ok(Bytes::from(format!("{first}\n\n"))),
        Ok(Bytes::from(
            r#"data: {"choices":[{"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":2}}"#.to_string()
                + "\n\n",
        )),
        Ok(Bytes::from("data: [DONE]\n\n")),
    ]);
    let mapped =
        chat_sse_to_responses_sse_with_meta(upstream, "m".to_string(), ChatToolMeta::default(), 0);
    let chunks: Vec<Bytes> = mapped.map(|r| r.expect("io")).collect().await;
    let all = chunks
        .iter()
        .map(|b| String::from_utf8_lossy(b))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        all.contains("event: response.in_progress"),
        "缺 in_progress：{all}"
    );
    assert!(
        all.contains("event: response.function_call_arguments.done"),
        "缺 arguments.done：{all}"
    );
    assert!(
        all.contains("\"type\":\"function_call\""),
        "工具项丢失：{all}"
    );
    assert!(all.contains("response.completed"), "缺终态事件：{all}");
}
