//! 上游 LLM 连通性探测（PRD v1.1.11 R1）
//!
//! 旧的探测只发一个 "ping"、HTTP 2xx 就算通过，模型名写错或额度耗尽也会显示「连接成功」。
//! 这里改成真的发一轮对话要求模型回话：只有解析出非空文本才算通过，同时回传耗时与生成速度。
//! URL 构造、鉴权头和实验免费通道路由全部复用网关代理现成的实现，不另发明一套。

use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};

use crate::error_hint;
use crate::gateway::{build_target_url, prepare_upstream_provider};
use crate::model::{ProviderConfig, ProviderProtocol};

/// 探测提示词：要一句最短回复；内容不做校验，只看模型有没有真实生成。
const PROBE_PROMPT: &str = "直接回复我在";
/// 给足输出上限，避免旧的 8 token 把回复直接截成空。
const PROBE_MAX_TOKENS: u32 = 64;
/// 真实生成比 ping 慢，单请求超时放宽到 30 秒。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// 上游错误体截断长度，避免整页 HTML 塞满 toast。
const ERROR_MAX_CHARS: usize = 300;

/// 一次探测的结果，前端按 camelCase 读取（latencyMs / tokensPerSec / completionTokens）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    /// 只有「HTTP 2xx 且解析出非空文本」才为 true
    pub ok: bool,
    /// 模型实际返回的文本
    pub reply: Option<String>,
    /// 完整请求耗时（发起 → 读完响应体）
    pub latency_ms: u64,
    /// 生成速度 = completion_tokens / 耗时秒
    pub tokens_per_sec: f64,
    /// 输出 token 数：优先取上游 usage，缺失时为估算值
    pub completion_tokens: u32,
    /// completion_tokens 是否为「字符数 / 4」的估算
    pub estimated: bool,
    /// 失败原因（已截断）
    pub error: Option<String>,
}

impl ProbeReport {
    fn failure(latency_ms: u64, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            reply: None,
            latency_ms,
            tokens_per_sec: 0.0,
            completion_tokens: 0,
            estimated: false,
            error: Some(truncate_error(&error.into())),
        }
    }
}

/// 探测上游是否能真实对话。永不 panic，所有失败都落在 `ProbeReport::error` 里。
pub async fn probe_llm_connectivity(client: &Client, provider: &ProviderConfig) -> ProbeReport {
    match provider.protocol {
        ProviderProtocol::OpenAi => probe_openai(client, provider).await,
        ProviderProtocol::Anthropic => probe_anthropic(client, provider).await,
    }
}

async fn probe_openai(client: &Client, provider: &ProviderConfig) -> ProbeReport {
    // 与网关代理保持一致：实验免费通道先绑定当前路由
    let provider = prepare_upstream_provider(provider);
    let url = build_target_url(&provider.base_url, "v1/chat/completions", "");
    let body = json!({
        "model": provider.model_name,
        "messages": [{ "role": "user", "content": PROBE_PROMPT }],
        "max_tokens": PROBE_MAX_TOKENS,
        "stream": false
    });

    let started = Instant::now();
    let mut req = client.post(&url).json(&body).timeout(PROBE_TIMEOUT);
    if !provider.api_key.trim().is_empty() {
        req = req.bearer_auth(&provider.api_key);
    }
    let response = match req.send().await {
        Ok(response) => response,
        Err(err) => {
            return ProbeReport::failure(
                elapsed_ms(started),
                error_hint::classify_request_error(&err),
            )
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let latency_ms = elapsed_ms(started);

    if !status.is_success() {
        tracing::warn!(provider = %provider.name, %status, body = %text, "probe_openai failed");
        return ProbeReport::failure(latency_ms, error_hint::classify_http_error(status, &text));
    }

    report_from_body(&text, ProviderProtocol::OpenAi, latency_ms)
}

async fn probe_anthropic(client: &Client, provider: &ProviderConfig) -> ProbeReport {
    let url = build_target_url(&provider.base_url, "v1/messages", "");
    let body = json!({
        "model": provider.model_name,
        "max_tokens": PROBE_MAX_TOKENS,
        "messages": [{ "role": "user", "content": PROBE_PROMPT }]
    });

    let started = Instant::now();
    let response = match client
        .post(&url)
        .header("x-api-key", &provider.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            return ProbeReport::failure(
                elapsed_ms(started),
                error_hint::classify_request_error(&err),
            )
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let latency_ms = elapsed_ms(started);

    if status.is_success() {
        return report_from_body(&text, ProviderProtocol::Anthropic, latency_ms);
    }

    tracing::warn!(provider = %provider.name, %status, body = %text, "probe_anthropic failed");

    // 沿用旧探测的判断：404 但 OpenAI 兼容路径可用时，直接提示改协议，少走弯路。
    if status == reqwest::StatusCode::NOT_FOUND && probe_openai(client, provider).await.ok {
        return ProbeReport::failure(
            latency_ms,
            "上游无 Anthropic /v1/messages（404），但 OpenAI chat 可用。请将协议改为 OpenAI Compatible",
        );
    }

    ProbeReport::failure(latency_ms, error_hint::classify_http_error(status, &text))
}

/// 把上游响应体整理成报告：成功判定只认「非空文本」。
/// 抽成纯函数便于单测覆盖空回复、usage 缺失与除零保护。
fn report_from_body(body: &str, protocol: ProviderProtocol, latency_ms: u64) -> ProbeReport {
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(err) => {
            return ProbeReport::failure(latency_ms, format!("响应不是合法 JSON：{err}"))
        }
    };

    let (text, usage_tokens) = extract_reply(&value, protocol);
    let text = text.trim().to_string();
    if text.is_empty() {
        return ProbeReport::failure(latency_ms, "上游返回 2xx，但没有可读的回复内容");
    }

    // usage 缺失或为 0 都按估算处理：0 个 token 却说有回复，只能说明上游没填
    let (completion_tokens, estimated) = match usage_tokens {
        Some(tokens) if tokens > 0 => (tokens, false),
        _ => (estimate_tokens(&text), true),
    };

    ProbeReport {
        ok: true,
        reply: Some(text),
        latency_ms,
        tokens_per_sec: speed_tokens_per_sec(completion_tokens, latency_ms),
        completion_tokens,
        estimated,
        error: None,
    }
}

/// 按协议取出回复文本与 usage 里的输出 token 数。
fn extract_reply(value: &Value, protocol: ProviderProtocol) -> (String, Option<u32>) {
    match protocol {
        ProviderProtocol::OpenAi => {
            let content = value
                .get("choices")
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"));
            let text = content.map(flatten_content).unwrap_or_default();
            let tokens = value
                .get("usage")
                .and_then(|usage| usage.get("completion_tokens"))
                .and_then(|tokens| tokens.as_u64())
                .map(|tokens| tokens as u32);
            (text, tokens)
        }
        ProviderProtocol::Anthropic => {
            let text = value
                .get("content")
                .map(flatten_content)
                .unwrap_or_default();
            let tokens = value
                .get("usage")
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(|tokens| tokens.as_u64())
                .map(|tokens| tokens as u32);
            (text, tokens)
        }
    }
}

/// content 可能是字符串，也可能是 text block 数组（OpenAI 多模态 / Anthropic blocks）。
fn flatten_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// 生成速度（token/s）；耗时为 0 时返回 0，避免除零。
fn speed_tokens_per_sec(completion_tokens: u32, latency_ms: u64) -> f64 {
    if latency_ms == 0 {
        return 0.0;
    }
    completion_tokens as f64 / (latency_ms as f64 / 1000.0)
}

/// 估算输出 token：字符数 / 4 向上取整，至少 1（有回复就说明至少生成了一个 token）。
fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    ((chars + 3) / 4).max(1) as u32
}

fn truncate_error(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= ERROR_MAX_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(ERROR_MAX_CHARS).collect();
    out.push('…');
    out
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_report_uses_usage_tokens_and_guards_division_by_zero() {
        // 上游给出 completion_tokens=10，耗时 2000ms → 5 t/s，且标记为实测
        let body = r#"{"choices":[{"message":{"content":"我在"}}],"usage":{"prompt_tokens":12,"completion_tokens":10}}"#;
        let report = report_from_body(body, ProviderProtocol::OpenAi, 2000);
        assert!(report.ok);
        assert_eq!(report.completion_tokens, 10);
        assert!(!report.estimated);
        assert!((report.tokens_per_sec - 5.0).abs() < 1e-9);

        // usage 缺失 → 按字符数/4 估算；耗时为 0 时不除零
        let body = r#"{"choices":[{"message":{"content":"我在我在我在我在"}}]}"#;
        let report = report_from_body(body, ProviderProtocol::OpenAi, 0);
        assert!(report.ok);
        assert!(report.estimated);
        assert_eq!(report.completion_tokens, 2);
        assert_eq!(report.tokens_per_sec, 0.0);
    }

    #[test]
    fn probe_report_rejects_success_status_with_empty_content() {
        // HTTP 2xx 但模型没回话：必须判失败，不能报「连接成功」
        let body = r#"{"choices":[{"message":{"content":"   "}}]}"#;
        let report = report_from_body(body, ProviderProtocol::OpenAi, 120);
        assert!(!report.ok);
        assert!(report.reply.is_none());
        assert!(report.error.is_some());
        assert_eq!(report.tokens_per_sec, 0.0);

        // Anthropic 的空 content 数组同样判失败
        let body = r#"{"content":[],"usage":{"output_tokens":0}}"#;
        let report = report_from_body(body, ProviderProtocol::Anthropic, 88);
        assert!(!report.ok);
        assert!(report.error.is_some());
    }

    #[test]
    fn probe_report_reads_anthropic_usage() {
        let body = r#"{"content":[{"type":"text","text":"我在"}],"usage":{"input_tokens":9,"output_tokens":4}}"#;
        let report = report_from_body(body, ProviderProtocol::Anthropic, 1000);
        assert!(report.ok);
        assert_eq!(report.reply.as_deref(), Some("我在"));
        assert_eq!(report.completion_tokens, 4);
        assert!(!report.estimated);
        assert!((report.tokens_per_sec - 4.0).abs() < 1e-9);
    }
}
