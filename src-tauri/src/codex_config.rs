use crate::model::AppConfig;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 写入/合并 Codex 官方配置（新版 CLI 以 config.toml 为准，环境变量常被忽略）。
pub fn apply_codex_config(config: &AppConfig) -> Result<PathBuf> {
    let codex_dir = codex_home_dir()?;
    std::fs::create_dir_all(&codex_dir)
        .with_context(|| format!("无法创建 Codex 目录 {}", codex_dir.display()))?;
    let config_path = codex_dir.join("config.toml");
    let openai_base = format!("{}/v1", config.listen_url());
    let public_model = config
        .providers
        .iter()
        .find(|p| config.active_provider_id.as_ref() == Some(&p.id))
        .map(|p| p.public_model_id())
        .unwrap_or_else(|| "gpt-4".to_string());

    let mut table: toml::Table = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        toml::from_str(&raw).unwrap_or_default()
    } else {
        toml::Table::new()
    };

    table.insert(
        "openai_base_url".to_string(),
        toml::Value::String(openai_base),
    );
    table.insert("model".to_string(), toml::Value::String(public_model));

    // 避免 ChatGPT 登录与 API Key 环境变量冲突导致 401
    table.insert(
        "preferred_auth_method".to_string(),
        toml::Value::String("apikey".to_string()),
    );
    table.insert(
        "cli_auth_credentials_store".to_string(),
        toml::Value::String("file".to_string()),
    );

    let serialized = toml::to_string_pretty(&table).context("Codex config 序列化失败")?;
    std::fs::write(&config_path, serialized)
        .with_context(|| format!("无法写入 {}", config_path.display()))?;

    write_codex_auth(&codex_dir, &crate::clients::client_api_key_for_config(config))?;

    Ok(config_path)
}

/// 取消 Codex 接管时：仅清除指向本机网关的 openai_base_url，不动其他用户配置。
pub fn clear_sugt_codex_overrides(config: &AppConfig) -> Result<()> {
    let config_path = codex_home_dir()?.join("config.toml");
    if !config_path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&config_path)?;
    let mut table: toml::Table = toml::from_str(&raw).unwrap_or_default();
    let expected = format!("{}/v1", config.listen_url());
    let current = table
        .get("openai_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if current == expected || current.contains("127.0.0.1") || current.contains("localhost") {
        table.remove("openai_base_url");
        let serialized = toml::to_string_pretty(&table).context("Codex config 序列化失败")?;
        std::fs::write(&config_path, serialized)
            .with_context(|| format!("无法写入 {}", config_path.display()))?;
    }
    Ok(())
}

pub fn codex_home_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = directories::UserDirs::new()
        .context("无法解析用户目录")?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".codex"))
}

pub fn read_openai_base_url() -> Result<Option<String>> {
    let config_path = codex_home_dir()?.join("config.toml");
    if !config_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&config_path)?;
    let table: toml::Table = toml::from_str(&raw).unwrap_or_default();
    Ok(table
        .get("openai_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexConfigStatus {
    pub config_path: String,
    pub openai_base_url: Option<String>,
    pub model: Option<String>,
    pub auth_configured: bool,
    pub matches_gateway: bool,
}

pub fn codex_config_status(config: &AppConfig) -> CodexConfigStatus {
    let codex_dir = codex_home_dir().ok();
    let config_path = codex_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| {
            codex_dir
                .as_ref()
                .map(|d| d.join("config.toml").display().to_string())
                .unwrap_or_else(|| "~/.codex/config.toml".to_string())
        });

    let expected_base = format!("{}/v1", config.listen_url());
    let openai_base = read_openai_base_url().ok().flatten();
    let model = codex_dir
        .as_ref()
        .and_then(|d| {
            let path = d.join("config.toml");
            if !path.exists() {
                return None;
            }
            let raw = std::fs::read_to_string(&path).ok()?;
            let table: toml::Table = toml::from_str(&raw).unwrap_or_default();
            table
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let auth_configured = codex_dir
        .as_ref()
        .map(|d| d.join("auth.json").exists())
        .unwrap_or(false);
    let matches_gateway = openai_base.as_deref() == Some(expected_base.as_str());

    CodexConfigStatus {
        config_path,
        openai_base_url: openai_base,
        model,
        auth_configured,
        matches_gateway,
    }
}

/// 写入 auth.json，供新版 Codex 读取 API Key（本地网关 dummy key 即可）。
fn write_codex_auth(codex_dir: &Path, api_key: &str) -> Result<()> {
    let auth_path = codex_dir.join("auth.json");
    // 仅保留 API Key 模式，清除可能残留的 ChatGPT 登录 token
    let value = serde_json::json!({
        "auth_mode": "apikey",
        "OPENAI_API_KEY": api_key,
    });
    std::fs::write(&auth_path, serde_json::to_string_pretty(&value)?)
        .with_context(|| format!("无法写入 {}", auth_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_home_default_under_dot_codex() {
        let dir = codex_home_dir().unwrap();
        assert!(dir.to_string_lossy().ends_with(".codex"));
    }
}
