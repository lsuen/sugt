use crate::model::AppConfig;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// SUGT 在 Codex config.toml 里托管的 provider id（可安全增删，不碰用户其它 provider）。
pub const SUGT_PROVIDER_ID: &str = "sugt";

/// 写入/合并 Codex 官方配置（新版 CLI 以 config.toml + model_providers 为准）。
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

    // 新版 Codex：model_provider + wire_api=responses（与 cc-switch 同类做法）
    table.insert(
        "model_provider".to_string(),
        toml::Value::String(SUGT_PROVIDER_ID.to_string()),
    );
    table.insert("model".to_string(), toml::Value::String(public_model));

    // 兼容旧字段：部分旧 CLI 仍读 openai_base_url
    table.insert(
        "openai_base_url".to_string(),
        toml::Value::String(openai_base.clone()),
    );

    // 避免 ChatGPT 登录与 API Key 冲突导致 401
    table.insert(
        "preferred_auth_method".to_string(),
        toml::Value::String("apikey".to_string()),
    );
    table.insert(
        "cli_auth_credentials_store".to_string(),
        toml::Value::String("file".to_string()),
    );

    let mut provider = toml::Table::new();
    provider.insert(
        "name".to_string(),
        toml::Value::String("SUGT".to_string()),
    );
    provider.insert(
        "base_url".to_string(),
        toml::Value::String(openai_base),
    );
    provider.insert(
        "wire_api".to_string(),
        toml::Value::String("responses".to_string()),
    );

    let mut providers = match table.get("model_providers").and_then(|v| v.as_table()) {
        Some(existing) => existing.clone(),
        None => toml::Table::new(),
    };
    providers.insert(SUGT_PROVIDER_ID.to_string(), toml::Value::Table(provider));
    table.insert(
        "model_providers".to_string(),
        toml::Value::Table(providers),
    );

    let serialized = toml::to_string_pretty(&table).context("Codex config 序列化失败")?;
    std::fs::write(&config_path, serialized)
        .with_context(|| format!("无法写入 {}", config_path.display()))?;

    write_codex_auth(&codex_dir, &crate::clients::client_api_key_for_config(config))?;

    Ok(config_path)
}

/// 取消 Codex 接管：撤掉 SUGT provider / 指向本机网关的覆盖，不动其它用户配置。
pub fn clear_sugt_codex_overrides(config: &AppConfig) -> Result<()> {
    let config_path = codex_home_dir()?.join("config.toml");
    if !config_path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&config_path)?;
    let mut table: toml::Table = toml::from_str(&raw).unwrap_or_default();
    let expected = format!("{}/v1", config.listen_url());
    let mut changed = false;

    if table
        .get("model_provider")
        .and_then(|v| v.as_str())
        .is_some_and(|s| s == SUGT_PROVIDER_ID)
    {
        table.remove("model_provider");
        changed = true;
    }

    if let Some(providers) = table.get_mut("model_providers").and_then(|v| v.as_table_mut()) {
        if providers.remove(SUGT_PROVIDER_ID).is_some() {
            changed = true;
        }
        if providers.is_empty() {
            table.remove("model_providers");
        }
    }

    let current = table
        .get("openai_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if current == expected || current.contains("127.0.0.1") || current.contains("localhost") {
        table.remove("openai_base_url");
        changed = true;
    }

    if changed {
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
    // 优先读新版 model_providers.sugt.base_url
    if let Some(base) = table
        .get("model_providers")
        .and_then(|v| v.as_table())
        .and_then(|p| p.get(SUGT_PROVIDER_ID))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
    {
        return Ok(Some(base.to_string()));
    }
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
    pub model_provider: Option<String>,
    pub wire_api: Option<String>,
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
    let table = codex_dir.as_ref().and_then(|d| {
        let path = d.join("config.toml");
        if !path.exists() {
            return None;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        toml::from_str::<toml::Table>(&raw).ok()
    });

    let openai_base = read_openai_base_url().ok().flatten();
    let model = table
        .as_ref()
        .and_then(|t| t.get("model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let model_provider = table
        .as_ref()
        .and_then(|t| t.get("model_provider"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let wire_api = table
        .as_ref()
        .and_then(|t| t.get("model_providers"))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get(SUGT_PROVIDER_ID))
        .and_then(|v| v.as_table())
        .and_then(|p| p.get("wire_api"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let auth_configured = codex_dir
        .as_ref()
        .map(|d| d.join("auth.json").exists())
        .unwrap_or(false);

    let base_ok = openai_base.as_deref() == Some(expected_base.as_str());
    let provider_ok = model_provider.as_deref() == Some(SUGT_PROVIDER_ID);
    let wire_ok = wire_api.as_deref() == Some("responses") || wire_api.is_none();
    // 新配置：provider=sugt 且 base 指向网关；旧配置：仅 openai_base_url 匹配也算
    let matches_gateway = base_ok && (provider_ok || model_provider.is_none()) && wire_ok;

    CodexConfigStatus {
        config_path,
        openai_base_url: openai_base,
        model,
        model_provider,
        wire_api,
        auth_configured,
        matches_gateway,
    }
}

/// 写入 auth.json，供新版 Codex 读取 API Key（本地网关 dummy key 即可）。
fn write_codex_auth(codex_dir: &Path, api_key: &str) -> Result<()> {
    let auth_path = codex_dir.join("auth.json");
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
    use crate::model::{AppConfig, ProviderConfig, ProviderProtocol};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_codex_home() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "sugt-codex-test-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config(port: u16) -> AppConfig {
        let mut provider = ProviderConfig::new(
            "test",
            "openai",
            "https://example.com/v1",
            "k",
            "demo-model",
        );
        provider.id = "p1".into();
        provider.protocol = ProviderProtocol::OpenAi;
        AppConfig {
            host: "127.0.0.1".into(),
            port,
            providers: vec![provider],
            active_provider_id: Some("p1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn codex_home_default_under_dot_codex() {
        let dir = codex_home_dir().unwrap();
        assert!(dir.to_string_lossy().ends_with(".codex"));
    }

    #[test]
    fn apply_writes_model_providers_responses() {
        let home = temp_codex_home();
        std::env::set_var("CODEX_HOME", &home);
        let config = sample_config(18787);
        let path = apply_codex_config(&config).unwrap();
        assert!(path.exists());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("model_provider"));
        assert!(raw.contains("sugt"));
        assert!(raw.contains("wire_api"));
        assert!(raw.contains("responses"));
        assert!(raw.contains("127.0.0.1:18787/v1"));
        let status = codex_config_status(&config);
        assert!(status.matches_gateway);
        assert_eq!(status.model_provider.as_deref(), Some("sugt"));
        assert_eq!(status.wire_api.as_deref(), Some("responses"));
        clear_sugt_codex_overrides(&config).unwrap();
        let raw2 = std::fs::read_to_string(&path).unwrap();
        assert!(!raw2.contains("[model_providers.sugt]") && !raw2.contains("model_providers.sugt"));
        // toml pretty may nest differently; ensure sugt provider gone
        let table: toml::Table = toml::from_str(&raw2).unwrap();
        assert!(table.get("model_provider").is_none());
        let providers = table
            .get("model_providers")
            .and_then(|v| v.as_table())
            .cloned()
            .unwrap_or_default();
        assert!(!providers.contains_key("sugt"));
        std::env::remove_var("CODEX_HOME");
        let _ = std::fs::remove_dir_all(&home);
    }
}
