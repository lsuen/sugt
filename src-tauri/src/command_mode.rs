//! 命令模式：上游不支持 function calling 时的降级路径。
//!
//! Codex 工具环依赖 Responses 的 `function_call` item。部分免费/轻量模型会
//! 直接拒 `tools`/`tool_choice`。此时不要把对话停死：去掉工具声明、注入约定，
//! 把模型用文本写出的命令再合成回 `function_call`，让客户端继续执行。
//!
//! 约定格式（优先结构化标记，bash 围栏作兜底）：
//! ```text
//! [[SUGT_CMD]]{"name":"shell_command","arguments":{"command":"ls"}}[[/SUGT_CMD]]
//! ```
//!
//! 作者：sun / 孙文龙

use bytes::Bytes;
use serde_json::{json, Value};
use tracing::{info, warn};

/// 命令模式开/闭标记（单独一行或嵌在正文里均可）。
pub const CMD_OPEN: &str = "[[SUGT_CMD]]";
pub const CMD_CLOSE: &str = "[[/SUGT_CMD]]";

/// 用户可见的中英提示：放在 output_text 前缀，不走 failed，以免打断工具环。
pub const USER_NOTICE: &str =
    "当前模型不支持函数调用，已切换命令模式 / Function calling unsupported; switched to command mode.";

/// 上游 4xx / 错误体是否在抱怨「不支持 tools / function calling」。
pub fn tools_unsupported_hint(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mentions_tools = lower.contains("tool_choice")
        || lower.contains("function calling")
        || lower.contains("function_calling")
        || lower.contains("function call")
        || lower.contains("tools parameter")
        || lower.contains("tool use")
        || lower.contains("tool_use")
        || (lower.contains("\"tools\"") && lower.contains("support"))
        || (lower.contains("tools")
            && (lower.contains("not support")
                || lower.contains("unsupported")
                || lower.contains("does not support")
                || lower.contains("don't support")
                || lower.contains("do not support")
                || lower.contains("not available")
                || lower.contains("is not supported")
                || lower.contains("aren't supported")
                || lower.contains("are not supported")));
    let mentions_cn = text.contains("不支持")
        && (text.contains("工具")
            || text.contains("函数调用")
            || text.contains("function")
            || text.contains("tools"));
    mentions_tools || mentions_cn
}

/// 解析出的一条命令（稍后合成 Responses `function_call`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,
    /// 已是 JSON 对象字符串，便于直接塞进 `arguments`。
    pub arguments: String,
}

/// 从正文提取命令：先结构化标记，没有再扫 ```bash 围栏。
pub fn extract_commands(text: &str) -> Vec<ParsedCommand> {
    let marked = parse_sugt_commands(text);
    if !marked.is_empty() {
        return marked;
    }
    parse_bash_fences(text)
}

/// 解析 `[[SUGT_CMD]]{...}[[/SUGT_CMD]]`。
pub fn parse_sugt_commands(text: &str) -> Vec<ParsedCommand> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(CMD_OPEN) {
        let after_open = &rest[start + CMD_OPEN.len()..];
        let Some(end) = after_open.find(CMD_CLOSE) else {
            break;
        };
        let raw = after_open[..end].trim();
        if let Some(cmd) = parse_command_json(raw, "shell_command") {
            out.push(cmd);
        } else {
            warn!(payload = %raw, "命令模式标记内 JSON 无法解析，已跳过");
        }
        rest = &after_open[end + CMD_CLOSE.len()..];
    }
    out
}

/// 兜底：```bash / ```sh 围栏 → shell 工具。
pub fn parse_bash_fences(text: &str) -> Vec<ParsedCommand> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(fence_at) = rest.find("```") {
        let after = &rest[fence_at + 3..];
        let (lang_line, body_start) = match after.find('\n') {
            Some(i) => (&after[..i], i + 1),
            None => break,
        };
        let lang = lang_line.trim().to_ascii_lowercase();
        if !matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh") {
            // 跳过非 shell 围栏，从下一个 ``` 继续
            if let Some(close) = after.find("```") {
                rest = &after[close + 3..];
                continue;
            }
            break;
        }
        let body = &after[body_start..];
        let Some(close) = body.find("```") else {
            break;
        };
        let script = body[..close].trim();
        if !script.is_empty() {
            out.push(ParsedCommand {
                name: "shell_command".to_string(),
                arguments: json!({ "command": script }).to_string(),
            });
        }
        rest = &body[close + 3..];
    }
    out
}

fn parse_command_json(raw: &str, default_name: &str) -> Option<ParsedCommand> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_name)
        .to_string();
    let arguments = match value.get("arguments") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => {
            // 允许直接写 {"command":"..."} 省略外层 name/arguments
            if value.get("command").is_some() || value.get("cmd").is_some() {
                value.to_string()
            } else {
                "{}".to_string()
            }
        }
    };
    Some(ParsedCommand { name, arguments })
}

/// 去掉可见正文里的命令标记与已被消费的 bash 围栏，避免客户端当纯文本回显。
pub fn strip_command_markup(text: &str) -> String {
    let mut rest = text;
    let mut cleaned = String::with_capacity(text.len());
    while let Some(start) = rest.find(CMD_OPEN) {
        cleaned.push_str(&rest[..start]);
        let after_open = &rest[start + CMD_OPEN.len()..];
        match after_open.find(CMD_CLOSE) {
            Some(end) => {
                rest = &after_open[end + CMD_CLOSE.len()..];
            }
            None => {
                // 残缺标记：丢掉开标记本身，保留后续，避免把半截协议漏给用户
                rest = after_open;
                break;
            }
        }
    }
    cleaned.push_str(rest);
    strip_shell_fences(&cleaned)
}

fn strip_shell_fences(text: &str) -> String {
    let mut rest = text;
    let mut cleaned = String::with_capacity(text.len());
    while let Some(fence_at) = rest.find("```") {
        cleaned.push_str(&rest[..fence_at]);
        let after = &rest[fence_at + 3..];
        let (lang_line, body_start) = match after.find('\n') {
            Some(i) => (&after[..i], i + 1),
            None => {
                cleaned.push_str("```");
                cleaned.push_str(after);
                return cleaned;
            }
        };
        let lang = lang_line.trim().to_ascii_lowercase();
        let body = &after[body_start..];
        if let Some(close) = body.find("```") {
            if matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh") {
                // 消费掉 shell 围栏
                rest = &body[close + 3..];
                continue;
            }
            // 非 shell：原样保留
            cleaned.push_str("```");
            cleaned.push_str(lang_line);
            cleaned.push('\n');
            cleaned.push_str(&body[..=close + 2]);
            rest = &body[close + 3..];
        } else {
            cleaned.push_str("```");
            cleaned.push_str(after);
            return cleaned;
        }
    }
    cleaned.push_str(rest);
    cleaned
}

/// 系统提示：告诉模型用约定格式输出命令。
pub fn command_mode_system_prompt(shell_tool_name: &str) -> String {
    format!(
        "你当前运行在 SUGT 命令模式：上游模型不支持原生 function calling / tools。\n\
         需要执行 shell 时，请单独输出一行结构化标记（不要用其它工具协议）：\n\
         {CMD_OPEN}{{\"name\":\"{shell}\",\"arguments\":{{\"command\":\"你的命令\"}}}}{CMD_CLOSE}\n\
         也可退而求其次用 ```bash 代码围栏。不要假装已经执行成功；等客户端回传结果后再继续。\n\
         You are in SUGT command mode (no native function calling). To run shell, emit one line:\n\
         {CMD_OPEN}{{\"name\":\"{shell}\",\"arguments\":{{\"command\":\"...\"}}}}{CMD_CLOSE}\n\
         Fallback: a ```bash fenced block. Do not claim success before the client returns output.",
        shell = shell_tool_name
    )
}

/// Chat 请求体是否带了 tools（决定要不要走命令模式重试）。
pub fn chat_body_has_tools(body: &Bytes) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("tools").and_then(Value::as_array).cloned())
        .map(|t| !t.is_empty())
        .unwrap_or(false)
}

/// 去掉 tools/tool_choice，并注入命令模式 system 提示。
pub fn prepare_chat_body_for_command_mode(
    body: &Bytes,
    shell_tool_name: &str,
) -> anyhow::Result<Bytes> {
    let mut value: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body)?
    };
    let Some(obj) = value.as_object_mut() else {
        return Ok(body.clone());
    };
    obj.remove("tools");
    obj.remove("tool_choice");

    let prompt = command_mode_system_prompt(shell_tool_name);
    match obj.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(messages) => {
            // 已有 system：拼到第一条；否则插到最前
            if let Some(first) = messages.first_mut() {
                if first.get("role").and_then(Value::as_str) == Some("system") {
                    let existing = first
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let merged = if existing.is_empty() {
                        prompt
                    } else {
                        format!("{existing}\n\n{prompt}")
                    };
                    if let Some(map) = first.as_object_mut() {
                        map.insert("content".to_string(), json!(merged));
                    }
                } else {
                    messages.insert(0, json!({ "role": "system", "content": prompt }));
                }
            } else {
                messages.push(json!({ "role": "system", "content": prompt }));
            }
        }
        None => {
            obj.insert(
                "messages".to_string(),
                json!([{ "role": "system", "content": prompt }]),
            );
        }
    }

    info!(
        shell_tool = %shell_tool_name,
        "已将 Chat 请求降级为命令模式（去掉 tools，注入约定提示）"
    );
    Ok(Bytes::from(serde_json::to_vec(&value)?))
}

/// 从工具元数据挑主 shell 名：优先客户端声明的 shell_command / exec_command。
pub fn preferred_shell_tool_name(
    flattened_names: &std::collections::HashMap<String, String>,
    custom_names: &std::collections::HashSet<String>,
) -> String {
    const CANDIDATES: &[&str] = &[
        "shell_command",
        "exec_command",
        "shell",
        "exec",
        "local_shell",
    ];
    // 先看客户端原名（flattened values / custom）
    for cand in CANDIDATES {
        if flattened_names.values().any(|v| v == cand) || custom_names.contains(*cand) {
            return (*cand).to_string();
        }
        if flattened_names.contains_key(*cand) {
            return (*cand).to_string();
        }
    }
    // 再模糊匹配
    for original in flattened_names.values() {
        let lower = original.to_ascii_lowercase();
        if lower.contains("shell") || lower.contains("exec") {
            return original.clone();
        }
    }
    for name in custom_names {
        let lower = name.to_ascii_lowercase();
        if lower.contains("shell") || lower.contains("exec") {
            return name.clone();
        }
    }
    "shell_command".to_string()
}

/// 把解析到的命令合成 Responses `function_call` output item。
pub fn synthesize_function_call_items(
    commands: &[ParsedCommand],
    id_prefix: &str,
    shell_fallback: &str,
) -> Vec<Value> {
    commands
        .iter()
        .enumerate()
        .map(|(idx, cmd)| {
            let name = if cmd.name.is_empty() {
                shell_fallback
            } else {
                cmd.name.as_str()
            };
            let call_id = format!("{id_prefix}_cmd_{idx}");
            json!({
                "type": "function_call",
                "id": format!("fc_{call_id}"),
                "call_id": call_id,
                "name": name,
                "arguments": cmd.arguments,
                "status": "completed",
            })
        })
        .collect()
}

/// 命令模式收尾：提取命令 + 清洗正文 + 加用户提示。
/// 返回 `(可见正文, function_call items)`。
pub fn materialize_command_mode_output(
    text: &str,
    id_prefix: &str,
    shell_tool_name: &str,
) -> (String, Vec<Value>) {
    let commands = extract_commands(text);
    // 标记里的 name 若仍是占位 shell_command，统一改成客户端主工具名
    let normalized: Vec<ParsedCommand> = commands
        .into_iter()
        .map(|mut c| {
            if c.name == "shell_command" && shell_tool_name != "shell_command" {
                c.name = shell_tool_name.to_string();
            }
            c
        })
        .collect();
    let items = synthesize_function_call_items(&normalized, id_prefix, shell_tool_name);
    let mut visible = strip_command_markup(text);
    visible = visible.trim().to_string();
    if !visible.starts_with(USER_NOTICE) {
        if visible.is_empty() {
            visible = USER_NOTICE.to_string();
        } else {
            visible = format!("{USER_NOTICE}\n{visible}");
        }
    }
    (visible, items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn detects_tools_unsupported_phrases() {
        assert!(tools_unsupported_hint(
            "This model does not support tools parameter"
        ));
        assert!(tools_unsupported_hint(
            r#"{"error":{"message":"tool_choice is not supported"}}"#
        ));
        assert!(tools_unsupported_hint(
            "function calling is unsupported on this endpoint"
        ));
        assert!(tools_unsupported_hint("当前模型不支持函数调用"));
        assert!(!tools_unsupported_hint("rate limit exceeded"));
        assert!(!tools_unsupported_hint(r#"{"error":null}"#));
    }

    #[test]
    fn parses_structured_command_marker() {
        let text = r#"先看目录
[[SUGT_CMD]]{"name":"shell_command","arguments":{"command":"ls -la"}}[[/SUGT_CMD]]
"#;
        let cmds = parse_sugt_commands(text);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "shell_command");
        assert!(cmds[0].arguments.contains("ls -la"));
    }

    #[test]
    fn bash_fence_is_fallback_only() {
        let with_marker = r#"[[SUGT_CMD]]{"name":"shell_command","arguments":{"command":"pwd"}}[[/SUGT_CMD]]
```bash
echo ignored
```"#;
        let cmds = extract_commands(with_marker);
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].arguments.contains("pwd"));

        let fence_only = "```bash\nls\n```";
        let cmds = extract_commands(fence_only);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "shell_command");
        assert!(cmds[0].arguments.contains("ls"));
    }

    #[test]
    fn strips_markup_and_notices_user() {
        let (visible, items) = materialize_command_mode_output(
            r#"准备执行
[[SUGT_CMD]]{"name":"shell_command","arguments":{"command":"echo hi"}}[[/SUGT_CMD]]"#,
            "resp",
            "shell_command",
        );
        assert!(visible.starts_with(USER_NOTICE));
        assert!(visible.contains("准备执行"));
        assert!(!visible.contains(CMD_OPEN));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "function_call");
        assert_eq!(items[0]["name"], "shell_command");
    }

    #[test]
    fn prepare_body_removes_tools_and_injects_system() {
        let body = Bytes::from(
            r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function"}],"tool_choice":"auto"}"#,
        );
        let out = prepare_chat_body_for_command_mode(&body, "shell_command").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert!(v.get("tools").is_none());
        assert!(v.get("tool_choice").is_none());
        assert_eq!(v["messages"][0]["role"], "system");
        assert!(v["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains(CMD_OPEN));
    }

    #[test]
    fn prefers_declared_shell_tool_name() {
        let mut flat = HashMap::new();
        flat.insert(
            "functions__exec_command".to_string(),
            "exec_command".to_string(),
        );
        let custom = HashSet::new();
        assert_eq!(
            preferred_shell_tool_name(&flat, &custom),
            "exec_command"
        );
    }
}
