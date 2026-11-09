//! 小su 写入确认与授权模式。
//! 作者：sun / 孙文龙
//!
//! 安全靠「授权模式 + 启发式」而不是让模型动不动说不行。
//! - ask：安全——会改盘的命令都先确认
//! - auto：完整——工作区内常规写入自动同意，危险/系统级仍确认
//! - trusted：首选完整——几乎全自动，仅拦截明显灾难命令
//!
//! 旧版 `auto_write=true` 等价于 `auto`。

use serde::{Deserialize, Serialize};

/// 对话授权模式（前端下拉 / CLI / IPC 共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeAuthMode {
    /// 安全：写前必确认
    #[default]
    Ask,
    /// 完整：常规写入自动同意
    Auto,
    /// 首选完整：仅拦截灾难级命令
    Trusted,
}

impl MeAuthMode {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("auto") | Some("complete") | Some("full") => Self::Auto,
            Some("trusted") | Some("yolo") | Some("bypass") => Self::Trusted,
            Some("ask") | Some("safe") | Some("secure") | None => Self::Ask,
            // 兼容旧布尔：true → auto
            Some("1") | Some("true") | Some("yes") | Some("on") => Self::Auto,
            Some("0") | Some("false") | Some("no") | Some("off") => Self::Ask,
            _ => Self::Ask,
        }
    }

    /// 旧 API：`auto_write` 布尔 → 模式。
    pub fn from_auto_write(auto_write: bool) -> Self {
        if auto_write {
            Self::Auto
        } else {
            Self::Ask
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
            Self::Trusted => "trusted",
        }
    }

    pub fn auto_approves_writes(self) -> bool {
        !matches!(self, Self::Ask)
    }
}

/// 明确只读、无写盘副作用的命令首词（在无危险 flag 时）。
const READ_ONLY: &[&str] = &[
    "ls", "pwd", "cat", "head", "tail", "wc", "stat", "file", "which", "whereis", "echo", "printf",
    "date", "uname", "whoami", "id", "printenv", "grep", "rg", "awk", "cut", "uniq", "diff", "true",
    "false", "dirname", "basename", "realpath", "readlink", "du", "df", "ps", "tree", "nl", "type",
    "cd", "pushd", "popd",
    // 用系统默认应用打开文件/URL（不改文件）
    "open", "xdg-open", "invoke-item",
];

/// 包装器：真实 argv0 在后面。
const WRAPPERS: &[&str] = &["env", "command", "time", "nice", "nohup", "stdbuf", "timeout"];

/// 灾难级：任意授权模式都要拦（trusted 也不放过）。
const CATASTROPHIC_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -fr /",
    "rm -r /",
    "rm -rf /*",
    "mkfs",
    "dd if=",
    ":(){",
    "fork bomb",
    "diskutil erase",
    "format c:",
    "rd /s /q c:\\",
];

/// 会改盘但常被误标只读的 flag / 子命令。
fn has_dangerous_readonly_flags(word: &str, segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    match word {
        "find" => tokens.iter().any(|t| {
            matches!(
                *t,
                "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir" | "-fprint" | "-fprintf"
            ) || t.starts_with("-fprint")
        }),
        "sort" => tokens.iter().any(|t| *t == "-o" || t.starts_with("-o")),
        "tee" => true,
        "env" | "printenv" => false,
        _ => false,
    }
}

fn is_catastrophic(script: &str) -> bool {
    let lower = script.to_ascii_lowercase().replace('\n', " ");
    CATASTROPHIC_PATTERNS
        .iter()
        .any(|p| lower.contains(p))
        || (lower.contains("rm ")
            && (lower.contains(" --no-preserve-root") || lower.contains(" / ") || lower.ends_with(" /")))
}

/// 是否需要用户确认（不考虑授权模式）。
pub fn command_needs_confirm(script: &str) -> bool {
    let stripped = strip_comments(script);
    if is_catastrophic(&stripped) {
        return true;
    }
    if has_unquoted_redirect(&stripped) || has_unquoted_substitution(&stripped) {
        return true;
    }
    split_segments(&stripped)
        .into_iter()
        .any(|segment| !segment_is_readonly(&segment))
}

/// 在给定授权模式下，这条命令是否应立刻执行（跳过确认 UI）。
pub fn should_auto_run(mode: MeAuthMode, script: &str) -> bool {
    if is_catastrophic(script) {
        return false;
    }
    match mode {
        MeAuthMode::Ask => !command_needs_confirm(script),
        MeAuthMode::Auto => !command_needs_confirm(script) || !is_high_risk_write(script),
        MeAuthMode::Trusted => true,
    }
}

/// auto 模式下仍要确认的高风险写：系统目录、批量删除、chmod/chown 等。
fn is_high_risk_write(script: &str) -> bool {
    let lower = script.to_ascii_lowercase();
    if lower.contains("/system")
        || lower.contains("/library/")
        || lower.contains("/etc/")
        || lower.contains("/usr/")
        || lower.contains("\\windows\\")
        || lower.contains("c:\\windows")
    {
        return true;
    }
    let stripped = strip_comments(script);
    for segment in split_segments(&stripped) {
        let Some(word) = effective_command_word(&segment) else {
            continue;
        };
        let w = word.to_ascii_lowercase();
        if matches!(
            w.as_str(),
            "rm" | "rmdir" | "del" | "rd" | "chmod" | "chown" | "chgrp" | "kill" | "killall"
                | "pkill" | "shutdown" | "reboot" | "launchctl" | "scutil" | "defaults"
        ) {
            return true;
        }
        if w == "git"
            && matches!(
                git_subcommand(&segment).as_deref(),
                Some("push" | "reset" | "clean" | "checkout" | "restore")
            )
        {
            return true;
        }
    }
    false
}

fn segment_is_readonly(segment: &str) -> bool {
    let Some(word) = effective_command_word(segment) else {
        return true;
    };
    let word = word.to_ascii_lowercase();
    if has_dangerous_readonly_flags(&word, segment) {
        return false;
    }
    if word == "git" {
        return matches!(
            git_subcommand(segment).as_deref(),
            Some("status" | "diff" | "log" | "show" | "rev-parse" | "branch" | "remote")
        );
    }
    if word == "sed" {
        return !segment
            .split_whitespace()
            .any(|token| token == "-i" || token.starts_with("-i"));
    }
    if word == "find" {
        return !has_dangerous_readonly_flags("find", segment);
    }
    if word == "sort" {
        return !has_dangerous_readonly_flags("sort", segment);
    }
    // Windows: cmd.exe /c start "" file.html — 仅打开
    if word == "cmd" || word == "cmd.exe" {
        let lower = segment.to_ascii_lowercase();
        return lower.contains(" start ") || lower.contains("\tstart ");
    }
    READ_ONLY.iter().any(|item| *item == word)
}

/// 穿透 env/command/time 等包装器，拿到真正要跑的命令名。
fn effective_command_word(segment: &str) -> Option<String> {
    let mut parts = segment.split_whitespace().peekable();
    // 跳过 VAR=value 前缀
    while let Some(tok) = parts.peek() {
        if tok.contains('=') && !tok.starts_with('=') && !tok.starts_with('-') {
            parts.next();
            continue;
        }
        break;
    }
    let mut word = parts.next()?.to_string();
    let mut base = command_base(&word)?;
    let mut guard = 0;
    while WRAPPERS.iter().any(|w| *w == base) && guard < 4 {
        guard += 1;
        // env -u FOO / env FOO=1 cmd …
        while let Some(tok) = parts.peek() {
            let t = *tok;
            if t.starts_with('-') {
                parts.next();
                // env -u NAME / timeout 10
                if matches!(base.as_str(), "env") && (t == "-u" || t == "-C" || t == "-i") {
                    let _ = parts.next();
                }
                if base == "timeout" && t.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    continue;
                }
                continue;
            }
            if t.contains('=') && !t.starts_with('=') {
                parts.next();
                continue;
            }
            break;
        }
        word = parts.next()?.to_string();
        base = command_base(&word)?;
    }
    Some(base)
}

fn command_base(word: &str) -> Option<String> {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = base.trim_matches(|c| c == '"' || c == '\'');
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

pub fn command_word(segment: &str) -> Option<String> {
    effective_command_word(segment)
}

pub fn git_subcommand(segment: &str) -> Option<String> {
    let mut parts = segment.split_whitespace();
    // 跳到 git
    while let Some(tok) = parts.next() {
        let base = command_base(tok)?.to_ascii_lowercase();
        if base == "git" {
            break;
        }
        if WRAPPERS.contains(&base.as_str()) || (tok.contains('=') && !tok.starts_with('-')) {
            continue;
        }
    }
    for tok in parts {
        if tok.starts_with('-') {
            continue;
        }
        return Some(tok.to_ascii_lowercase());
    }
    None
}

pub fn strip_comments(script: &str) -> String {
    let mut out = String::with_capacity(script.len());
    let mut quote: Option<char> = None;
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            out.push(c);
            continue;
        }
        if c == '#' {
            while let Some(&n) = chars.peek() {
                if n == '\n' {
                    break;
                }
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn split_segments(script: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < chars.len() {
                i += 1;
                cur.push(chars[i]);
            }
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            cur.push(c);
            i += 1;
            continue;
        }
        // && || ; | 分段（管道右侧也可能写文件，如 tee）
        if c == ';' {
            push_seg(&mut segments, &mut cur);
            i += 1;
            continue;
        }
        if c == '&' || c == '|' {
            let doubled = i + 1 < chars.len() && chars[i + 1] == c;
            push_seg(&mut segments, &mut cur);
            i += if doubled { 2 } else { 1 };
            continue;
        }
        cur.push(c);
        i += 1;
    }
    push_seg(&mut segments, &mut cur);
    segments
}

fn push_seg(segments: &mut Vec<String>, cur: &mut String) {
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    cur.clear();
}

pub fn has_unquoted_redirect(script: &str) -> bool {
    let mut quote: Option<char> = None;
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < chars.len() {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '>' {
            let next = chars.get(i + 1).copied().unwrap_or(' ');
            // >= 与 >& 不是重定向写文件
            if next == '=' || next == '&' {
                i += 1;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

pub fn has_unquoted_substitution(script: &str) -> bool {
    let mut quote: Option<char> = None;
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < chars.len() {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '`' {
            return true;
        }
        if c == '$' && i + 1 < chars.len() && (chars[i + 1] == '(' || chars[i + 1] == '{') {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mode_parse_aliases() {
        assert_eq!(MeAuthMode::parse(Some("ask")), MeAuthMode::Ask);
        assert_eq!(MeAuthMode::parse(Some("auto")), MeAuthMode::Auto);
        assert_eq!(MeAuthMode::parse(Some("trusted")), MeAuthMode::Trusted);
        assert_eq!(MeAuthMode::parse(Some("true")), MeAuthMode::Auto);
        assert_eq!(MeAuthMode::from_auto_write(true), MeAuthMode::Auto);
    }

    #[test]
    fn wrappers_and_find_delete_need_confirm() {
        assert!(command_needs_confirm("find . -delete"));
        assert!(command_needs_confirm("find . -exec rm {} +"));
        assert!(command_needs_confirm("env rm -rf tmp"));
        assert!(command_needs_confirm("command rm x"));
        assert!(command_needs_confirm("sort -o out.txt in.txt"));
        assert!(!command_needs_confirm("find . -name '*.rs'"));
        assert!(!command_needs_confirm("sort in.txt"));
        assert!(!command_needs_confirm("env printenv PATH"));
    }

    #[test]
    fn trusted_auto_runs_normal_writes_but_not_catastrophe() {
        assert!(should_auto_run(MeAuthMode::Trusted, "mkdir notes"));
        assert!(should_auto_run(MeAuthMode::Trusted, "rm -rf tmp"));
        assert!(!should_auto_run(MeAuthMode::Trusted, "rm -rf /"));
        assert!(!should_auto_run(MeAuthMode::Ask, "mkdir notes"));
        assert!(should_auto_run(MeAuthMode::Auto, "mkdir notes"));
        assert!(!should_auto_run(MeAuthMode::Auto, "rm -rf tmp"));
    }
}
