//! `anthropic_adapter` 模块单元测试（由 anthropic_adapter.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

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
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!({"type": "auto"})),
        json!("auto")
    );
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!({"type": "none"})),
        json!("none")
    );
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!({"type": "any"})),
        json!("required")
    );
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!({"type": "tool", "name": "bash"})),
        json!({"type": "function", "function": {"name": "bash"}})
    );
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!("auto")),
        json!("auto")
    );
    assert_eq!(
        anthropic_tool_choice_to_openai(&json!("any")),
        json!("required")
    );
}

#[test]
fn skips_null_fields_when_converting_request() {
    let body = Bytes::from_static(
        br#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":100,"temperature":null,"top_p":null,"stop_sequences":null,"stream":false}"#,
    );
    let payload = anthropic_to_openai(body, "target").unwrap();
    let value: Value = serde_json::from_slice(&payload).unwrap();
    assert!(
        value.get("temperature").is_none(),
        "temperature 不应透传 null"
    );
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
    assert_eq!(
        converted["image_url"]["url"],
        "data:image/png;base64,aGVsbG8="
    );
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
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/jpeg;base64,AAAA"
    );
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
    let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None, None, None);
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
    let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None, None, None);
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
    let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None, None, None);
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
    let out = openai_sse_to_anthropic_sse(stream, "target-model".to_string(), None, None, None);
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
