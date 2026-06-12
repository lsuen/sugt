use crate::{config::AppPaths, model::AppConfig};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;

const DUMMY_KEY: &str = "sugt-local-key";

/// Claude Code 与 AUTH_TOKEN 冲突的变量（旧版 SUGT 会误设，需清理）
const CLAUDE_CONFLICTING_VARS: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"];

#[derive(Debug, Clone, Serialize)]
pub struct ClientEnvStatus {
    pub client: String,
    pub configured: bool,
    pub variables: BTreeMap<String, String>,
    pub missing: Vec<String>,
    pub issues: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientsEnvStatus {
    pub listen_url: String,
    pub claude: ClientEnvStatus,
    pub codex: ClientEnvStatus,
    pub has_issues: bool,
}

pub fn status(config: &AppConfig) -> ClientsEnvStatus {
    let listen_url = listen_url(config);
    let claude_expected = claude_vars(&listen_url);
    let codex_expected = codex_vars(&listen_url);

    let claude = client_status(
        "Claude Code",
        claude_expected,
        claude_tracked_vars(),
        detect_claude_issues,
        "仅使用 ANTHROPIC_AUTH_TOKEN 接管，请新开终端后生效",
    );
    let codex = client_status(
        "Codex",
        codex_expected,
        codex_tracked_vars(),
        detect_no_issues,
        "OpenAI 协议，请新开终端后生效",
    );

    ClientsEnvStatus {
        listen_url,
        has_issues: !claude.issues.is_empty() || !codex.issues.is_empty(),
        claude,
        codex,
    }
}

pub fn install(config: &AppConfig) -> Result<ClientsEnvStatus> {
    clear_claude_conflicts()?;
    let listen_url = listen_url(config);
    for (name, value) in claude_vars(&listen_url)
        .into_iter()
        .chain(codex_vars(&listen_url))
    {
        set_user_env(&name, &value)?;
    }
    Ok(status(config))
}

pub fn repair(config: &AppConfig) -> Result<ClientsEnvStatus> {
    install(config)
}

pub fn uninstall(config: &AppConfig) -> Result<ClientsEnvStatus> {
    let listen_url = listen_url(config);
    let mut names: Vec<String> = claude_vars(&listen_url)
        .into_keys()
        .chain(codex_vars(&listen_url).into_keys())
        .chain(CLAUDE_CONFLICTING_VARS.iter().map(|s| s.to_string()))
        .collect();
    names.sort_unstable();
    names.dedup();

    for name in names {
        let expected = claude_vars(&listen_url)
            .get(&name)
            .cloned()
            .or_else(|| codex_vars(&listen_url).get(&name).cloned())
            .unwrap_or_default();
        let actual = std::env::var(&name).unwrap_or_default();
        if expected.is_empty() || actual == expected || actual == DUMMY_KEY {
            let _ = unset_user_env(&name);
        }
    }
    Ok(status(config))
}

pub fn print_launch_script(config: &AppConfig) -> String {
    let listen_url = listen_url(config);
    format!(
        "set ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\nset OPENAI_API_KEY={}\r\n",
        listen_url, DUMMY_KEY, listen_url, listen_url, DUMMY_KEY
    )
}

pub fn write_launch_scripts(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    let listen_url = listen_url(config);
    let claude = format!(
        "@echo off\r\nset ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset ANTHROPIC_API_KEY=\r\nset CLAUDE_CODE_API_KEY=\r\nclaude %*\r\n",
        listen_url, DUMMY_KEY
    );
    let codex = format!(
        "@echo off\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\nset OPENAI_API_KEY={}\r\ncodex %*\r\n",
        listen_url, listen_url, DUMMY_KEY
    );
    std::fs::write(paths.config_dir.join("claude-sugt.cmd"), claude)?;
    std::fs::write(paths.config_dir.join("codex-sugt.cmd"), codex)?;
    Ok(())
}

fn listen_url(config: &AppConfig) -> String {
    format!("http://{}:{}", config.host, config.port)
}

fn claude_vars(listen_url: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("ANTHROPIC_BASE_URL".to_string(), listen_url.to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), DUMMY_KEY.to_string()),
    ])
}

fn codex_vars(listen_url: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("OPENAI_BASE_URL".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_BASE".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_KEY".to_string(), DUMMY_KEY.to_string()),
    ])
}

fn claude_tracked_vars() -> Vec<String> {
    let mut vars = vec![
        "ANTHROPIC_BASE_URL".to_string(),
        "ANTHROPIC_AUTH_TOKEN".to_string(),
    ];
    vars.extend(
        CLAUDE_CONFLICTING_VARS
            .iter()
            .map(|name| (*name).to_string()),
    );
    vars
}

fn codex_tracked_vars() -> Vec<String> {
    vec![
        "OPENAI_BASE_URL".to_string(),
        "OPENAI_API_BASE".to_string(),
        "OPENAI_API_KEY".to_string(),
    ]
}

fn detect_no_issues() -> Vec<String> {
    Vec::new()
}

fn detect_claude_issues() -> Vec<String> {
    let mut issues = Vec::new();
    let auth_token = std::env::var("ANTHROPIC_AUTH_TOKEN").unwrap_or_default();
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let claude_code_key = std::env::var("CLAUDE_CODE_API_KEY").unwrap_or_default();

    if !auth_token.is_empty() && !api_key.is_empty() {
        issues.push(
            "同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，Claude Code 会报警".to_string(),
        );
    }
    if !auth_token.is_empty() && !claude_code_key.is_empty() {
        issues.push(
            "同时设置了 ANTHROPIC_AUTH_TOKEN 与 CLAUDE_CODE_API_KEY，建议仅保留 AUTH_TOKEN"
                .to_string(),
        );
    }
    if !api_key.is_empty() && auth_token.is_empty() {
        issues.push(
            "检测到 ANTHROPIC_API_KEY 但未设置 ANTHROPIC_AUTH_TOKEN，SUGT 接管应使用 AUTH_TOKEN"
                .to_string(),
        );
    }
    issues
}

fn clear_claude_conflicts() -> Result<()> {
    for name in CLAUDE_CONFLICTING_VARS {
        let _ = unset_user_env(name);
    }
    Ok(())
}

fn client_status(
    client: &str,
    expected: BTreeMap<String, String>,
    tracked: Vec<String>,
    issue_detector: fn() -> Vec<String>,
    note: &str,
) -> ClientEnvStatus {
    let mut variables = BTreeMap::new();
    let mut missing = Vec::new();

    for name in tracked {
        let actual = std::env::var(&name).unwrap_or_default();
        variables.insert(name.clone(), mask_env_value(&name, &actual));
    }

    for (name, expected_value) in &expected {
        let actual = std::env::var(name).unwrap_or_default();
        if actual != *expected_value {
            missing.push(name.clone());
        }
    }

    let issues = issue_detector();
    let configured = missing.is_empty() && issues.is_empty();

    ClientEnvStatus {
        client: client.to_string(),
        configured,
        variables,
        missing,
        issues,
        note: note.to_string(),
    }
}

fn mask_env_value(name: &str, value: &str) -> String {
    if value.is_empty() {
        return "未设置".to_string();
    }
    if name.contains("KEY") || name.contains("TOKEN") {
        return crate::model::mask_secret(value);
    }
    value.to_string()
}

#[cfg(windows)]
fn set_user_env(name: &str, value: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let status = std::process::Command::new("setx")
        .arg(name)
        .arg(value)
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if status.success() {
        std::env::set_var(name, value);
        Ok(())
    } else {
        Err(anyhow!("setx 写入 {} 失败", name))
    }
}

#[cfg(windows)]
fn unset_user_env(name: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let status = std::process::Command::new("reg")
        .args(["delete", r"HKCU\Environment", "/v", name, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    std::env::remove_var(name);
    if status.success() {
        Ok(())
    } else {
        // 变量不存在时 reg delete 也会失败，清理场景视为成功
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_user_env(_name: &str, _value: &str) -> Result<()> {
    Err(anyhow!(
        "当前平台暂不支持自动写入用户环境变量，请使用 sugt-cli env print 输出临时启动变量"
    ))
}

#[cfg(not(windows))]
fn unset_user_env(_name: &str) -> Result<()> {
    Err(anyhow!("当前平台暂不支持自动删除用户环境变量"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_vars_only_use_auth_token() {
        let vars = claude_vars("http://127.0.0.1:8787");
        assert!(vars.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(!vars.contains_key("ANTHROPIC_API_KEY"));
        assert!(!vars.contains_key("CLAUDE_CODE_API_KEY"));
    }
}
