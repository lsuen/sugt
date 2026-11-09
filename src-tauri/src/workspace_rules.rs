//! 全局工作区规则：软约束注入 system / instructions，webhook 通知走本地代发。

use bytes::Bytes;
use serde_json::{json, Value};
use tracing::info;

use crate::model::{DocHabit, WorkspaceRules};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestShape {
    OpenAiChat,
    AnthropicMessages,
    OpenAiResponses,
}

/// 文档习惯的内置注入文案。`{docs}` 会换成已启用的文档列表。
///
/// 注意：不要写成「未建好文档就禁止做任何事」。Codex 工具环里那种硬门禁会把
/// 「打开浏览器 / 写指定文件」劫持成先建 AGENTS.md，然后模型只宣布下一步就假收工。
pub fn default_inject_template() -> &'static str {
    "[SUGT 文档习惯]\n\
适用范围：仅当本次会改代码或项目结构时维护下列文档；打开网页、查资料、读写用户点名的文件等任务不要被文档门禁打断。\n\
开干：若会改代码，先读下列文档；不存在可边做边补，不要只宣布「先建文档」就结束本轮。\n\
收工：若本次改动了代码/结构，总结前用写入工具把变动写回下列文档。\n\
{docs}"
}

pub fn effective_inject_template(rules: &WorkspaceRules) -> String {
    let custom = rules.inject_template.trim();
    if custom.is_empty() {
        default_inject_template().to_string()
    } else {
        custom.to_string()
    }
}

/// 拼出将注入模型上下文的文本；无可注入内容时返回 None。
pub fn build_inject_text(rules: &WorkspaceRules, _listen_url: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let rules_text = rules.rules_text.trim();
    if rules.enabled && !rules_text.is_empty() {
        parts.push(format!("[SUGT 全局规则]\n{rules_text}"));
    }

    let habits = enabled_habits(rules);
    if !habits.is_empty() {
        let docs = habits
            .iter()
            .map(|habit| {
                let path = habit.path.trim();
                let note = habit.note.trim();
                if note.is_empty() {
                    format!("- `{path}`")
                } else {
                    format!("- `{path}`\n  作用与说明：{note}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let template = effective_inject_template(rules);
        let block = if template.contains("{docs}") {
            template.replace("{docs}", &docs)
        } else {
            format!("{template}\n{docs}")
        };
        parts.push(block);
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn enabled_habits(rules: &WorkspaceRules) -> Vec<&DocHabit> {
    if !rules.docs_enabled {
        return Vec::new();
    }
    rules
        .doc_habits
        .iter()
        .filter(|h| h.enabled && !h.path.trim().is_empty())
        .collect()
}

/// 钉在最近一条 user 消息上的短提醒（仅文档习惯）。
fn request_nudge(rules: &WorkspaceRules) -> Option<String> {
    let habits = enabled_habits(rules);
    if habits.is_empty() {
        return None;
    }
    let paths = habits
        .iter()
        .map(|h| {
            let path = format!("`{}`", h.path.trim());
            let note = h.note.trim();
            if note.is_empty() {
                path
            } else {
                format!("{path}（{note}）")
            }
        })
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "[SUGT] 文档 {paths}：仅改代码时维护；勿因文档门禁中断用户当前任务，更不要只宣布计划就收工。"
    ))
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
    let nudge = request_nudge(rules);
    let mut nudged = false;
    match shape {
        RequestShape::OpenAiChat => {
            nudged = inject_openai_chat(&mut value, &text, nudge.as_deref());
        }
        RequestShape::AnthropicMessages => {
            nudged = inject_anthropic(&mut value, &text, nudge.as_deref());
        }
        RequestShape::OpenAiResponses => {
            nudged = inject_responses(&mut value, &text, nudge.as_deref());
        }
    }
    info!(
        shape = ?shape,
        inject_chars = text.len(),
        nudged,
        "workspace rules injected into upstream request"
    );
    Bytes::from(serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec()))
}

/// 返回是否实际把 nudge 钉到了 user 消息上。
fn inject_openai_chat(value: &mut Value, text: &str, nudge: Option<&str>) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return false;
    };
    if let Some(first) = messages.first_mut() {
        if first.get("role").and_then(|r| r.as_str()) == Some("system") {
            append_content_field(first, text);
        } else {
            messages.insert(0, json!({ "role": "system", "content": text }));
        }
    } else {
        messages.push(json!({ "role": "system", "content": text }));
    }
    if let Some(nudge) = nudge {
        nudge_last_user_message(messages, nudge)
    } else {
        false
    }
}

/// 返回是否实际把 nudge 钉到了 user 消息上。
fn inject_anthropic(value: &mut Value, text: &str, nudge: Option<&str>) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    match obj.get_mut("system") {
        Some(system) if system.is_string() => {
            let old = system.as_str().unwrap_or("").to_string();
            *system = Value::String(append_text(&old, text));
        }
        Some(system) if system.is_array() => {
            if let Some(blocks) = system.as_array_mut() {
                // 追加到 system 末尾，避免被超长 Claude system 淹没
                blocks.push(json!({ "type": "text", "text": text }));
            }
        }
        Some(_) | None => {
            obj.insert("system".to_string(), Value::String(text.to_string()));
        }
    }
    if let Some(nudge) = nudge {
        if let Some(messages) = obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
            return nudge_last_user_message(messages, nudge);
        }
    }
    false
}

/// 返回是否实际把 nudge 钉到了 user 消息上。
fn inject_responses(value: &mut Value, text: &str, nudge: Option<&str>) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    match obj.get("instructions").and_then(|i| i.as_str()) {
        Some(old) if !old.is_empty() => {
            obj.insert(
                "instructions".to_string(),
                Value::String(append_text(old, text)),
            );
        }
        _ => {
            obj.insert("instructions".to_string(), Value::String(text.to_string()));
        }
    }
    if let Some(nudge) = nudge {
        // Codex 多轮会把原始 user + function_call(_output) 整包重放。若每轮都把
        // 文档 nudge 钉回原始 user，模型会被反复拽去「先建 AGENTS.md」，用户任务
        // （开浏览器等）永远排不到。工具环进行中只保留 instructions，不再改 user。
        if !responses_input_in_tool_loop(obj) {
            return nudge_responses_input(obj, nudge);
        }
    }
    false
}

/// Responses input 是否已进入工具环（含 call / output / reasoning 等）。
fn responses_input_in_tool_loop(obj: &serde_json::Map<String, Value>) -> bool {
    let Some(Value::Array(items)) = obj.get("input") else {
        return false;
    };
    items.iter().any(responses_item_is_tool_loop)
}

fn responses_item_is_tool_loop(item: &Value) -> bool {
    let typ = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    matches!(
        typ,
        "function_call"
            | "function_call_output"
            | "custom_tool_call"
            | "custom_tool_call_output"
            | "local_shell_call"
            | "local_shell_call_output"
            | "tool_search_call"
            | "tool_search_output"
            | "mcp_call"
            | "mcp_call_output"
            | "reasoning"
            | "item_reference"
    ) || (item.get("call_id").is_some()
        && (item.get("output").is_some() || item.get("arguments").is_some()))
}

/// Responses 的 input 可能是 string 或 message 数组；尽量钉到末条 user。
fn nudge_responses_input(obj: &mut serde_json::Map<String, Value>, nudge: &str) -> bool {
    match obj.get_mut("input") {
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                *s = nudge.to_string();
            } else {
                *s = format!("{nudge}\n\n{s}");
            }
            true
        }
        Some(Value::Array(items)) => {
            for item in items.iter_mut().rev() {
                let role = item
                    .get("role")
                    .and_then(|r| r.as_str())
                    .or_else(|| item.get("type").and_then(|t| t.as_str()));
                if role == Some("user") || role == Some("message") {
                    if is_tool_result_only(item) || responses_item_is_tool_loop(item) {
                        continue;
                    }
                    prepend_content_field(item, nudge);
                    return true;
                }
            }
            items.push(json!({ "role": "user", "content": nudge }));
            true
        }
        _ => false,
    }
}

/// 给最近一条「可读的」user 文本加提醒；跳过纯 tool_result 回合。
fn nudge_last_user_message(messages: &mut [Value], nudge: &str) -> bool {
    for message in messages.iter_mut().rev() {
        if message.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        if is_tool_result_only(message) {
            continue;
        }
        prepend_content_field(message, nudge);
        return true;
    }
    false
}

fn is_tool_result_only(message: &Value) -> bool {
    match message.get("content") {
        Some(Value::Array(blocks)) => {
            !blocks.is_empty()
                && blocks.iter().all(|b| {
                    matches!(b.get("type").and_then(|t| t.as_str()), Some("tool_result"))
                })
        }
        _ => false,
    }
}

fn append_content_field(message: &mut Value, text: &str) {
    let Some(obj) = message.as_object_mut() else {
        return;
    };
    match obj.get_mut("content") {
        Some(content) if content.is_string() => {
            let old = content.as_str().unwrap_or("").to_string();
            *content = Value::String(append_text(&old, text));
        }
        Some(content) if content.is_array() => {
            if let Some(parts) = content.as_array_mut() {
                parts.push(json!({ "type": "text", "text": text }));
            }
        }
        _ => {
            obj.insert("content".to_string(), Value::String(text.to_string()));
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
            *content = Value::String(if old.trim().is_empty() {
                text.to_string()
            } else {
                format!("{text}\n\n{old}")
            });
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

fn append_text(existing: &str, suffix: &str) -> String {
    if existing.trim().is_empty() {
        suffix.to_string()
    } else {
        format!("{existing}\n\n{suffix}")
    }
}

/// 钉钉/兼容自定义机器人的 markdown 消息体
pub fn webhook_markdown_payload(title: &str, text: &str) -> Value {
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
    use crate::model::{DocHabit, WorkspaceRules};

    fn rules_on() -> WorkspaceRules {
        WorkspaceRules {
            enabled: true,
            rules_text: "完成后简短汇报".into(),
            doc_habits: vec![DocHabit {
                enabled: true,
                path: "docs/MAP.md".into(),
                note: String::new(),
            }],
            docs_enabled: true,
            notify_enabled: true,
            notify_webhook: "https://oapi.dingtalk.com/robot/send?access_token=x".into(),
            inject_template: String::new(),
            map_doc_enabled: false,
            map_doc_path: String::new(),
        }
    }

    #[test]
    fn builds_inject_text_with_all_sections() {
        let text = build_inject_text(&rules_on(), "http://127.0.0.1:8787").unwrap();
        assert!(text.contains("[SUGT 全局规则]"));
        assert!(text.contains("完成后简短汇报"));
        assert!(text.contains("docs/MAP.md"));
        assert!(text.contains("文档习惯"));
        assert!(text.contains("不要被文档门禁打断"));
        assert!(text.contains("收工"));
        assert!(text.contains("不要只宣布"));
        assert!(!text.contains("/v1/_sugt/notify"));
        assert!(!text.contains("access_token=x"));
    }

    #[test]
    fn custom_inject_template_replaces_docs_placeholder() {
        let mut r = rules_on();
        r.inject_template = "请维护：\n{docs}".into();
        let text = build_inject_text(&r, "http://127.0.0.1:8787").unwrap();
        assert!(text.contains("请维护："));
        assert!(text.contains("docs/MAP.md"));
        assert!(!text.contains("{docs}"));
        assert!(!text.contains("收工门禁"));
    }

    #[test]
    fn builds_multiple_doc_habits_with_custom_note() {
        let mut r = rules_on();
        r.doc_habits = vec![
            DocHabit {
                enabled: true,
                path: "AGENTS.md".into(),
                note: String::new(),
            },
            DocHabit {
                enabled: true,
                path: "docs/MAP.md".into(),
                note: "保持模块索引最新".into(),
            },
            DocHabit {
                enabled: false,
                path: "README.md".into(),
                note: String::new(),
            },
        ];
        let text = build_inject_text(&r, "http://127.0.0.1:8787").unwrap();
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("docs/MAP.md"));
        assert!(text.contains("作用与说明：保持模块索引最新"));
        assert!(!text.contains("README.md"));
    }

    #[test]
    fn disabled_or_empty_yields_none() {
        let mut r = rules_on();
        r.enabled = false;
        r.docs_enabled = false;
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());

        let mut r = WorkspaceRules::default();
        r.enabled = false;
        r.docs_enabled = false;
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());

        r.rules_text.clear();
        r.enabled = true;
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());
    }

    #[test]
    fn rules_and_docs_switches_are_independent() {
        let mut r = rules_on();
        r.enabled = false;
        let text = build_inject_text(&r, "http://127.0.0.1:8787").unwrap();
        assert!(!text.contains("全局规则"));
        assert!(text.contains("docs/MAP.md"));

        r.enabled = true;
        r.docs_enabled = false;
        let text = build_inject_text(&r, "http://127.0.0.1:8787").unwrap();
        assert!(text.contains("全局规则"));
        assert!(!text.contains("docs/MAP.md"));
    }

    #[test]
    fn migrates_legacy_map_doc_fields() {
        let mut r = WorkspaceRules {
            enabled: true,
            rules_text: String::new(),
            doc_habits: Vec::new(),
            docs_enabled: false,
            notify_enabled: false,
            notify_webhook: String::new(),
            inject_template: String::new(),
            map_doc_enabled: true,
            map_doc_path: "docs/MAP.md".into(),
        };
        r.normalize();
        assert!(r
            .doc_habits
            .iter()
            .any(|h| h.path == "docs/MAP.md" && h.enabled));
        assert!(!r.map_doc_enabled);
        assert!(r.map_doc_path.is_empty());
    }

    #[test]
    fn seeds_default_habits_when_empty() {
        let mut r = WorkspaceRules::default();
        r.doc_habits.clear();
        r.normalize();
        assert_eq!(r.doc_habits[0].path, "AGENTS.md");
        assert!(r.doc_habits[0].enabled);
        assert_eq!(r.doc_habits[1].path, "docs/MAP.md");
        assert!(r.doc_habits[1].enabled);
        assert!(r.docs_enabled);
    }

    #[test]
    fn default_seeds_rules_text_without_turning_rules_on() {
        let r = WorkspaceRules::default();
        assert!(!r.enabled);
        assert!(!r.notify_enabled);
        assert!(r.rules_text.contains("用用户本次消息的语言回复"));
        assert!(r.rules_text.contains("不要提交、推送、删除"));
        let text = build_inject_text(&r, "http://127.0.0.1:8787").unwrap();
        assert!(!text.contains("全局规则"));
        assert!(text.contains("AGENTS.md"));
        assert!(text.contains("docs/MAP.md"));

        let mut cleared = r;
        cleared.rules_text.clear();
        cleared.normalize();
        assert!(cleared.rules_text.is_empty());
    }

    #[test]
    fn prepends_agents_md_when_missing() {
        let mut r = WorkspaceRules::default();
        r.doc_habits = vec![DocHabit {
            enabled: true,
            path: "docs/MAP.md".into(),
            note: "中文创建".into(),
        }];
        r.normalize();
        assert_eq!(r.doc_habits[0].path, "AGENTS.md");
        assert!(!r.doc_habits[0].enabled);
        assert_eq!(r.doc_habits[1].path, "docs/MAP.md");
        assert!(r.doc_habits[1].enabled);
        assert_eq!(r.doc_habits[1].note, "中文创建");
    }

    #[test]
    fn injects_openai_system_and_nudges_user() {
        let body = Bytes::from_static(
            br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#,
        );
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::OpenAiChat,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["messages"][0]["role"], "system");
        assert!(v["messages"][0]["content"].as_str().unwrap().contains("全局规则"));
        let user = v["messages"][1]["content"].as_str().unwrap();
        assert!(user.contains("[SUGT]"));
        assert!(user.contains("勿因文档门禁中断"));
        assert!(user.contains("docs/MAP.md"));
        assert!(!user.contains("/v1/_sugt/notify"));
        assert!(user.contains("hi"));
    }

    #[test]
    fn appends_existing_openai_system() {
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
        assert!(content.starts_with("project"));
        assert!(content.contains("[SUGT 全局规则]"));
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
        assert!(system.starts_with("sys"));
        assert!(system.contains("[SUGT 全局规则]"));
        let user = v["messages"][0]["content"].as_str().unwrap();
        assert!(user.contains("勿因文档门禁中断"));
        assert!(user.contains("docs/MAP.md"));
        assert!(!user.contains("/v1/_sugt/notify"));
    }

    #[test]
    fn notify_alone_does_not_inject() {
        let mut r = WorkspaceRules::default();
        r.enabled = true;
        r.rules_text.clear();
        r.notify_enabled = true;
        r.notify_webhook = "https://example.com/hook".into();
        r.doc_habits.clear();
        assert!(build_inject_text(&r, "http://127.0.0.1:8787").is_none());
    }

    #[test]
    fn skips_nudge_on_tool_result_only_turn() {
        let body = Bytes::from_static(
            br#"{"system":"sys","messages":[{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}],"max_tokens":100}"#,
        );
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::AnthropicMessages,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        let content = &v["messages"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content.as_array().unwrap().len(), 1);
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
        assert!(instr.starts_with("base"));
        assert!(instr.contains("[SUGT 全局规则]"));
        assert!(instr.contains("收工"));
        let input = v["input"].as_str().unwrap();
        assert!(input.contains("勿因文档门禁中断"));
        assert!(input.contains("hi"));
    }

    #[test]
    fn skips_responses_nudge_during_tool_loop() {
        let body = Bytes::from(
            r#"{"instructions":"base","input":[{"role":"user","content":[{"type":"input_text","text":"open baidu"}]},{"type":"function_call","call_id":"c1","name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"},{"type":"function_call_output","call_id":"c1","output":"ok"}]}"#
                .as_bytes()
                .to_vec(),
        );
        let out = inject_into_request_body(
            body,
            &rules_on(),
            "http://127.0.0.1:8787",
            RequestShape::OpenAiResponses,
        );
        let v: Value = serde_json::from_slice(&out).unwrap();
        let instr = v["instructions"].as_str().unwrap();
        assert!(instr.contains("文档习惯"));
        let input = v["input"].as_array().unwrap();
        let user = input[0]["content"][0]["text"].as_str().unwrap();
        assert_eq!(user, "open baidu");
        assert!(!user.contains("[SUGT]"));
    }
}
