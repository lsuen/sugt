use crate::config::AppPaths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const HISTORY_FILE: &str = "quick-chat.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub id: String,
    pub role: String,
    pub content: String,
    /// Unix epoch milliseconds
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_per_sec: Option<f64>,
    #[serde(default)]
    pub tokens_estimated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatHistory {
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub messages: Vec<ChatTurn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatReply {
    pub content: String,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tokens_per_sec: f64,
    pub tokens_estimated: bool,
    pub provider_id: String,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryMsg {
    pub role: String,
    pub content: String,
}

fn history_path(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join(HISTORY_FILE)
}

pub fn load_history(paths: &AppPaths) -> ChatHistory {
    let path = history_path(paths);
    let Ok(raw) = fs::read_to_string(&path) else {
        return ChatHistory::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_history(paths: &AppPaths, history: &ChatHistory) -> Result<()> {
    fs::create_dir_all(&paths.config_dir)?;
    let path = history_path(paths);
    let raw = serde_json::to_string_pretty(history).context("会话序列化失败")?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &raw).with_context(|| format!("无法写入 {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("无法保存会话 {}", path.display()))
}

/// 物理清空：删除会话文件。下次打开读不到任何历史。
pub fn clear_history(paths: &AppPaths) -> Result<()> {
    let path = history_path(paths);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("无法删除会话 {}", path.display()))?;
    }
    let tmp = path.with_extension("tmp");
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

pub fn extract_reply_text(body: &Value) -> Option<String> {
    for pointer in [
        "/choices/0/message/content",
        "/choices/0/text",
        "/content/0/text",
        // 部分推理模型把正文放在 reasoning（content 为空）
        "/choices/0/message/reasoning",
        "/choices/0/message/reasoning_content",
        "/choices/0/delta/reasoning",
    ] {
        if let Some(content) = body.pointer(pointer).and_then(value_as_nonempty_text) {
            return Some(content);
        }
    }
    None
}

fn value_as_nonempty_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(text.to_string());
    }
    let parts = value.as_array()?;
    let text: String = parts
        .iter()
        .filter_map(|part| {
            part.as_str()
                .map(str::to_string)
                .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_string))
        })
        .collect();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub fn extract_usage(body: &Value) -> Option<(u64, u64)> {
    let usage = body.get("usage")?;
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
    if input == 0 && output == 0 {
        None
    } else {
        Some((input, output))
    }
}

/// 无 usage 时用字数粗估输出 token（CJK 约 1 字≈1 token，拉丁约 4 字符≈1）。
pub fn estimate_output_tokens(text: &str) -> u64 {
    let mut units = 0.0_f64;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch.is_ascii_punctuation() || ch == ' ' {
            units += 0.25;
        } else {
            units += 1.0;
        }
    }
    units.ceil().max(1.0) as u64
}

/// 粗估 Chat Completions 请求的入站 token：消息正文 + tools schema（不计 JSON 外壳噪音）。
/// Codex 每轮都会带全量 tools + 历史，因此「短句」也会到数万，这是上下文体积而非统计错误。
pub fn estimate_chat_input_tokens(chat_json: &str) -> u64 {
    let Ok(value) = serde_json::from_str::<Value>(chat_json) else {
        return estimate_output_tokens(chat_json);
    };
    let mut total = 0u64;
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for msg in messages {
            total = total.saturating_add(estimate_message_tokens(msg));
        }
    }
    if let Some(tools) = value.get("tools") {
        total = total.saturating_add(estimate_output_tokens(&tools.to_string()));
    }
    if let Some(system) = value.get("instructions").and_then(Value::as_str) {
        total = total.saturating_add(estimate_output_tokens(system));
    }
    total.max(1)
}

fn estimate_message_tokens(msg: &Value) -> u64 {
    let mut n = 4; // role / 结构开销
    match msg.get("content") {
        Some(Value::String(s)) => n += estimate_output_tokens(s),
        Some(Value::Array(parts)) => {
            for part in parts {
                if let Some(t) = part.get("text").and_then(Value::as_str) {
                    n += estimate_output_tokens(t);
                } else if let Some(t) = part.as_str() {
                    n += estimate_output_tokens(t);
                } else {
                    n += estimate_output_tokens(&part.to_string());
                }
            }
        }
        Some(other) => n += estimate_output_tokens(&other.to_string()),
        None => {}
    }
    if let Some(tools) = msg.get("tool_calls") {
        n += estimate_output_tokens(&tools.to_string());
    }
    if let Some(args) = msg.get("function_call") {
        n += estimate_output_tokens(&args.to_string());
    }
    n
}

pub fn tokens_per_sec(output_tokens: u64, latency_ms: u64) -> f64 {
    if output_tokens == 0 || latency_ms == 0 {
        return 0.0;
    }
    (output_tokens as f64) / (latency_ms as f64 / 1000.0)
}

pub fn build_upstream_messages(history: &[HistoryMsg], latest: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in history {
        let role = msg.role.trim();
        if role != "user" && role != "assistant" {
            continue;
        }
        let content = msg.content.trim();
        if content.is_empty() {
            continue;
        }
        out.push(serde_json::json!({ "role": role, "content": content }));
    }
    out.push(serde_json::json!({ "role": "user", "content": latest }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tokens_per_sec_basic() {
        assert!((tokens_per_sec(100, 1000) - 100.0).abs() < 1e-6);
        assert_eq!(tokens_per_sec(0, 1000), 0.0);
        assert_eq!(tokens_per_sec(50, 0), 0.0);
    }

    #[test]
    fn extract_usage_openai_and_anthropic() {
        let oai = json!({"usage": {"prompt_tokens": 3, "completion_tokens": 7}});
        assert_eq!(extract_usage(&oai), Some((3, 7)));
        let ant = json!({"usage": {"input_tokens": 2, "output_tokens": 9}});
        assert_eq!(extract_usage(&ant), Some((2, 9)));
    }

    #[test]
    fn estimate_and_history_build() {
        assert!(estimate_output_tokens("你好世界") >= 4);
        let chat = r#"{"messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"x","parameters":{}}}]}"#;
        let n = estimate_chat_input_tokens(chat);
        assert!(n > 1);
        assert!(n < estimate_output_tokens(chat));
        let hist = [
            HistoryMsg {
                role: "user".into(),
                content: "hi".into(),
            },
            HistoryMsg {
                role: "system".into(),
                content: "err".into(),
            },
            HistoryMsg {
                role: "assistant".into(),
                content: "yo".into(),
            },
        ];
        let msgs = build_upstream_messages(&hist, "next");
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["content"], "next");
    }

    #[test]
    fn clear_removes_file() {
        let dir = std::env::temp_dir().join(format!("sugt-qc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let paths = AppPaths {
            config_dir: dir.clone(),
            config_file: dir.join("config.toml"),
            log_file: dir.join("sugt.log"),
        };
        let hist = ChatHistory {
            provider_id: "p1".into(),
            messages: vec![ChatTurn {
                id: "1".into(),
                role: "user".into(),
                content: "hi".into(),
                created_at: 1,
                latency_ms: None,
                input_tokens: None,
                output_tokens: None,
                tokens_per_sec: None,
                tokens_estimated: false,
                provider_id: None,
                provider_name: None,
                model_name: None,
            }],
        };
        save_history(&paths, &hist).unwrap();
        assert!(history_path(&paths).exists());
        clear_history(&paths).unwrap();
        assert!(!history_path(&paths).exists());
        assert!(load_history(&paths).messages.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
