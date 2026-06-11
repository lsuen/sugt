use crate::{config::AppPaths, model::AppConfig};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;

const DUMMY_KEY: &str = "sugt-local-key";

#[derive(Debug, Clone, Serialize)]
pub struct ClientEnvStatus {
    pub client: String,
    pub configured: bool,
    pub variables: BTreeMap<String, String>,
    pub missing: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientsEnvStatus {
    pub listen_url: String,
    pub claude: ClientEnvStatus,
    pub codex: ClientEnvStatus,
}

pub fn status(config: &AppConfig) -> ClientsEnvStatus {
    let listen_url = listen_url(config);
    let claude_expected = claude_vars(&listen_url);
    let codex_expected = codex_vars(&listen_url);

    ClientsEnvStatus {
        listen_url,
        claude: client_status(
            "Claude Code",
            claude_expected,
            "Anthropic 协议，重启终端后生效",
        ),
        codex: client_status("Codex", codex_expected, "OpenAI 协议，重启终端后生效"),
    }
}

pub fn install(config: &AppConfig) -> Result<ClientsEnvStatus> {
    let listen_url = listen_url(config);
    for (name, value) in claude_vars(&listen_url)
        .into_iter()
        .chain(codex_vars(&listen_url))
    {
        set_user_env(&name, &value)?;
    }
    Ok(status(config))
}

pub fn uninstall(config: &AppConfig) -> Result<ClientsEnvStatus> {
    let listen_url = listen_url(config);
    for (name, expected) in claude_vars(&listen_url)
        .into_iter()
        .chain(codex_vars(&listen_url))
    {
        if std::env::var(&name).unwrap_or_default() == expected {
            unset_user_env(&name)?;
        }
    }
    Ok(status(config))
}

pub fn print_launch_script(config: &AppConfig) -> String {
    let listen_url = listen_url(config);
    format!(
        "set ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset ANTHROPIC_API_KEY={}\r\nset CLAUDE_CODE_API_KEY={}\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\nset OPENAI_API_KEY={}\r\n",
        listen_url, DUMMY_KEY, DUMMY_KEY, DUMMY_KEY, listen_url, listen_url, DUMMY_KEY
    )
}

pub fn write_launch_scripts(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    let listen_url = listen_url(config);
    let claude = format!(
        "@echo off\r\nset ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset ANTHROPIC_API_KEY={}\r\nset CLAUDE_CODE_API_KEY={}\r\nclaude %*\r\n",
        listen_url, DUMMY_KEY, DUMMY_KEY, DUMMY_KEY
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
        ("ANTHROPIC_API_KEY".to_string(), DUMMY_KEY.to_string()),
        ("CLAUDE_CODE_API_KEY".to_string(), DUMMY_KEY.to_string()),
    ])
}

fn codex_vars(listen_url: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("OPENAI_BASE_URL".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_BASE".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_KEY".to_string(), DUMMY_KEY.to_string()),
    ])
}

fn client_status(client: &str, expected: BTreeMap<String, String>, note: &str) -> ClientEnvStatus {
    let mut variables = BTreeMap::new();
    let mut missing = Vec::new();

    for (name, expected_value) in expected {
        let actual = std::env::var(&name).unwrap_or_default();
        variables.insert(name.clone(), mask_env_value(&name, &actual));
        if actual != expected_value {
            missing.push(name);
        }
    }

    ClientEnvStatus {
        client: client.to_string(),
        configured: missing.is_empty(),
        variables,
        missing,
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
    if status.success() {
        std::env::remove_var(name);
        Ok(())
    } else {
        Err(anyhow!("删除用户环境变量 {} 失败", name))
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
