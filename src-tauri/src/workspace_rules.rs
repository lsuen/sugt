//! 全局工作区规则：软约束注入 system / instructions，硬能力（钉钉）走本地代发。

use bytes::Bytes;
use serde_json::{json, Value};

use crate::model::WorkspaceRules;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestShape {
    OpenAiChat,
    AnthropicMessages,
    OpenAiResponses,
}

/// 拼出将注入模型上下文的文本；无可注入内容时返回 None。
pub fn build_inject_text(rules: &WorkspaceRules, listen_url: &str) -> Option<String> {
    if !rules.enabled {
        return None;
    }

    let mut parts: Vec<String> = Vec::new();
    let rules_text = rules.rules_text.trim();
    if !rules_text.is_empty() {
        parts.push(format!("[SUGT 全局规则]\n{rules_text}"));
    }

    if rules.map_doc_enabled {
        let path = rules.map_doc_path.trim();
        let path = if path.is_empty() {
            "docs/MAP.md"
        } else {
            path
        };
        parts.push(format!(
            "[文档习惯]\n开干前阅读并遵循 `{path}`；目录/模块结构有变时，任务结束前更新该文档。"
        ));
    }

    if rules.dingtalk_enabled && !rules.dingtalk_webhook.trim().is_empty() {
        let base = listen_url.trim().trim_end_matches('/');
        parts.push(format!(
            "[收尾通知]\n任务有实质进展或完成时，向本地 SUGT 网关发送钉钉通知（由网关代发，勿把 webhook 写入对话）：\n\
             POST {base}/v1/_sugt/notify\n\
             Header: Authorization: Bearer <与连接本网关相同的 API Key>\n\
             Body JSON: {{\"title\":\"简短标题\",\"text\":\"Markdown 正文\"}}"
        ));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub fn inject_into_request_body(
    body: Bytes,
    rules: &WorkspaceRules,
    listen_url: &str,
    shape: RequestShape,
) -> Bytes {
    let Some(text) = build_inject_text(rules, listen_url) else {
        return body;
    };
    if body.is_empty() {
        return body;
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    match shape {
        RequestShape::OpenAiChat => inject_openai_chat(&mut value, &text),
        RequestShape::AnthropicMessages => inject_anthropic(&mut value, &text),
        RequestShape::OpenAiResponses => inject_responses(&mut value, &text),
    }
    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

fn inject_openai_chat(value: &mut Value, text: &str) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            prepend_content_field(first, text);
            return;
        }
    }
    messages.insert(0, json!({ "role": "system", "content": text }));
}

fn inject_anthropic(value: &mut Value, text: &str) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    match obj.get_mut("system") {
        Some(system) if system.is_string() => {
            let old = system.as_str().unwrap_or("").to_string();
            *system = Value::String(merge_text(&old, text));
        }
        Some(system) if system.is_array() => {
            if let Some(blocks) = system.as_array_mut() {
                blocks.insert(0, json!({ "type": "text", "text": text }));
            }
        }
        Some(_) | None => {
            obj.insert("system".to_string(), Value::String(text.to_string()));
        }
    }
}

fn inject_responses(value: &mut Value, text: &str) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    match obj.get("instructions").and_then(|i| i.as_str()) {
        Some(old) if !old.is_empty() => {
            obj.insert(
                "instructions".to_string(),
                Value::String(merge_text(old, text)),
            );
        }
        _ => {
            obj.insert("instructions".to_string(), Value::String(text.to_string()));
        }
    }
}

fn prepend_content_field(message: &mut Value, text: &str) {
    let Some(obj) = message.as_object_mut() else {
        return;
    };
    match obj.get_mut("content") {
        Some(content) if content.is_string() => {
            let old = content.as_str().unwrap_or("").to_string();
            *content = Value::String(merge_text(&old, text));
        }
        Some(content) if content.is_array() => {
            if let Some(parts) = content.as_array_mut() {
                parts.insert(0, json!({ "type": "text", "text": text }));
            }
        }
        _ => {
            obj.insert("content".to_string(), Value::String(text.to_string()));
        }
    }
}

fn merge_text(existing: &str, prepend: &str) -> String {
    if existing.trim().is_empty() {
        prepend.to_string()
    } else {
        format!("{prepend}\n\n{existing}")
    }
}

/// 钉钉自定义机器人 markdown 消息体
pub fn dingtalk_markdown_payload(title: &str, text: &str) -> Value {
    let title = if title.trim().is_empty() {
        "SUGT"
    } else {
        title.trim()
    };
    let text = if text.trim().is_empty() {
        title
    } else {
        text.trim()
    };
    json!({
        "msgtype": "markdown",
        "markdown": {
            "title": title,
            "text": text,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorkspaceRules;

    fn rules_on() -> WorkspaceRules {
        WorkspaceRules {
            enabled: true,
            rules_text: "完成后简短汇报".into(),
            map_doc_enabled: true,
            map_doc_path: "docs/MAP.md".into(),
            dingtalk_enabled: true,
            dingtalk_webhook: "https://oapi.dingtalk.com/robot/send?access_token=x".into(),
        }
    }

    #[test]
    fn builds_inject_text_with_all_sections() {
        let text = build_inject_text(&rules_on(), "http://127.0.0.1:8787").unwrap();
        assert!(text.contains("[SUGT 全局规则]"));
        assert!(text.contains("完成后简短汇报"));
        assert!(text.contains("docs/MAP.md"));
        assert!(text.contains("/v1/_sugt/notify"));
        assert!(!text.contains("access_token=x"));
    }

    #[test]
    fn disabled_or_empty_yields_none() {
        let mut r = rules_on();
        r.enabled = false;
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());

        let mut r = WorkspaceRules::default();
        r.enabled = true;
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());
    }

    #[test]
    fn injects_openai_system_message() {
        let body = Bytes::from_static(br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#);
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::OpenAiChat,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["messages"][0]["role"], "system");
        assert!(v["messages"][0]["content"].as_str().unwrap().contains("全局规则"));
        assert_eq!(v["messages"][1]["role"], "user");
    }

    #[test]
    fn prepends_existing_openai_system() {
        let body = Bytes::from_static(
            br#"{"messages":[{"role":"system","content":"project"},{"role":"user","content":"hi"}]}"#,
        );
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::OpenAiChat,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        let content = v["messages"][0]["content"].as_str().unwrap();
        assert!(content.starts_with("[SUGT 全局规则]"));
        assert!(content.contains("project"));
        assert_eq!(v["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn injects_anthropic_system_string() {
        let body = Bytes::from_static(
            br#"{"system":"sys","messages":[{"role":"user","content":"hi"}],"max_tokens":100}"#,
        );
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::AnthropicMessages,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        let system = v["system"].as_str().unwrap();
        assert!(system.starts_with("[SUGT 全局规则]"));
        assert!(system.contains("sys"));
    }

    #[test]
    fn injects_responses_instructions() {
        let body = Bytes::from_static(br#"{"instructions":"base","input":"hi"}"#);
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::OpenAiResponses,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        let instr = v["instructions"].as_str().unwrap();
        assert!(instr.starts_with("[SUGT 全局规则]"));
        assert!(instr.contains("base"));
    }
}
