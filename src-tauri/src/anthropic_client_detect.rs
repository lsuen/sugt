//! Anthropic 系客户端识别
//!
//! 网关需要区分请求来源：
//! - 强制走 Anthropic 协议（原生端点）的 TUI / GUI 客户端（如 Claude Code、Claude Desktop）
//! - 普通 OpenAI 客户端（如 codex CLI 走 /v1/messages 的场景）
//!
//! 识别到 Claude 系客户端后，优先尝试服务商的原生 Anthropic 端点，
//! 原生端点不可用时再降级走 OpenAI→Anthropic 转换规则（fallback）。

use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;

/// Anthropic 系客户端的类型，用于后续的匹配与展示
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicClientKind {
    /// Claude Code CLI（TUI，claude-cli / claude-code）
    ClaudeCode,
    /// Claude Desktop（GUI）
    ClaudeDesktop,
    /// 其它明确要求 Anthropic 协议的客户端（cline / roo-cline / continue / windsurf 等）
    OtherAnthropic,
}

impl AnthropicClientKind {
    /// 用于展示的短名称
    pub fn label(self) -> &'static str {
        match self {
            AnthropicClientKind::ClaudeCode => "claude-code",
            AnthropicClientKind::ClaudeDesktop => "claude-desktop",
            AnthropicClientKind::OtherAnthropic => "anthropic-client",
        }
    }
}

/// 从请求头识别是否为 Anthropic 系客户端。
///
/// 匹配策略：
/// 1. User-Agent 精确特征（claude-cli / claude-code / claude-desktop）
/// 2. User-Agent 模糊特征（含 claude / anthropic / cline / roo-cline / continue / windsurf）
///
/// 返回 `None` 表示未识别到（按普通 OpenAI 客户端处理）。
pub fn detect_anthropic_client(headers: &HeaderMap) -> Option<AnthropicClientKind> {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim();

    if user_agent.is_empty() {
        return None;
    }

    let lower = user_agent.to_lowercase();
    // Claude Desktop 等客户端 UA 常写作 `ClaudeDesktop/...`（无连字符），
    // 去掉连字符后统一匹配，避免精确特征漏掉
    let compact = lower.replace('-', "");

    // 精确特征：Claude Code CLI / Claude Desktop
    if lower.contains("claude-cli")
        || lower.contains("claude-code")
        || compact.contains("claudecli")
        || compact.contains("claudecode")
    {
        return Some(AnthropicClientKind::ClaudeCode);
    }
    if lower.contains("claude-desktop") || compact.contains("claudedesktop") {
        return Some(AnthropicClientKind::ClaudeDesktop);
    }

    // 模糊特征：明确以 Anthropic 协议接入的 TUI / GUI
    const ANTHROPIC_KEYWORDS: [&str; 6] = [
        "claude", "anthropic", "cline", "roo-cline", "windsurf", "continue",
    ];
    if ANTHROPIC_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        return Some(AnthropicClientKind::OtherAnthropic);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_ua(user_agent: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, user_agent.parse().unwrap());
        headers
    }

    #[test]
    fn detects_claude_code_cli() {
        let headers = headers_with_ua("claude-cli/2.0.0 (Claude Code)");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::ClaudeCode)
        );
    }

    #[test]
    fn detects_claude_code_ua() {
        let headers = headers_with_ua("claude-code/1.2.3 node/v20.0.0");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::ClaudeCode)
        );
    }

    #[test]
    fn detects_claude_desktop() {
        let headers = headers_with_ua("ClaudeDesktop/1.0.10 macOS");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::ClaudeDesktop)
        );
    }

    #[test]
    fn detects_other_anthropic_clients() {
        let headers = headers_with_ua("Cline/3.0.0 (VSCode extension)");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::OtherAnthropic)
        );
        let headers = headers_with_ua("roo-cline/4.1.0");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::OtherAnthropic)
        );
    }

    #[test]
    fn does_not_detect_plain_curl_or_codex() {
        assert_eq!(
            detect_anthropic_client(&headers_with_ua("curl/8.4.0")),
            None
        );
        assert_eq!(
            detect_anthropic_client(&headers_with_ua("codex/0.12.0")),
            None
        );
    }

    #[test]
    fn empty_ua_returns_none() {
        assert_eq!(detect_anthropic_client(&HeaderMap::new()), None);
    }

    #[test]
    fn detects_case_insensitively() {
        let headers = headers_with_ua("CLAUDE-CODE/2.0.0");
        assert_eq!(
            detect_anthropic_client(&headers),
            Some(AnthropicClientKind::ClaudeCode)
        );
    }
}
