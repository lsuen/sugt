use crate::model::{AppConfig, ProviderConfig};
use anyhow::{Context, Result};
use directories::UserDirs;
use std::{fs, path::PathBuf};

const DIR_NAME: &str = ".sugt";
const CONFIG_FILE: &str = "config.toml";
const LOG_FILE: &str = "sugt.log";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub log_file: PathBuf,
}

pub fn resolve_paths() -> Result<AppPaths> {
    let base = if let Ok(dir) = std::env::var("SUGT_CONFIG_DIR") {
        PathBuf::from(dir)
    } else {
        let user_dirs = UserDirs::new().context("无法解析用户目录")?;
        user_dirs
            .document_dir()
            .unwrap_or_else(|| user_dirs.home_dir())
            .join(DIR_NAME)
    };

    Ok(AppPaths {
        config_dir: base.clone(),
        config_file: base.join(CONFIG_FILE),
        log_file: base.join(LOG_FILE),
    })
}

pub fn ensure_paths() -> Result<AppPaths> {
    let paths = resolve_paths()?;
    fs::create_dir_all(&paths.config_dir)
        .with_context(|| format!("无法创建配置目录 {}", paths.config_dir.display()))?;
    Ok(paths)
}

pub fn load_or_init_config() -> Result<(AppPaths, AppConfig)> {
    let paths = ensure_paths()?;
    if !paths.config_file.exists() {
        let config = default_config_from_env();
        save_config(&paths, &config)?;
        return Ok((paths, config));
    }

    let raw = fs::read_to_string(&paths.config_file)
        .with_context(|| format!("无法读取配置文件 {}", paths.config_file.display()))?;
    let mut config: AppConfig = toml::from_str(&raw).context("配置文件格式错误")?;
    normalize_config(&mut config);
    Ok((paths, config))
}

pub fn save_config(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    fs::create_dir_all(&paths.config_dir)?;
    let serialized = toml::to_string_pretty(config).context("配置序列化失败")?;
    fs::write(&paths.config_file, serialized)
        .with_context(|| format!("无法写入配置文件 {}", paths.config_file.display()))
}

pub fn normalize_config(config: &mut AppConfig) {
    for provider in &mut config.providers {
        provider.base_url = crate::model::normalize_base_url_for_protocol(
            provider.base_url.clone(),
            &provider.protocol,
        );
    }

    if config.active_provider_id.is_none() {
        config.active_provider_id = config
            .providers
            .iter()
            .find(|provider| provider.enabled)
            .map(|provider| provider.id.clone());
    }
}

fn default_config_from_env() -> AppConfig {
    let mut config = AppConfig::default();
    let base_url = std::env::var("SUGT_DEFAULT_BASE_URL").ok();
    let api_key = std::env::var("SUGT_DEFAULT_API_KEY").ok();
    let model_name = std::env::var("SUGT_DEFAULT_MODEL").ok();

    if let (Some(base_url), Some(api_key), Some(model_name)) = (base_url, api_key, model_name) {
        let provider = ProviderConfig::new(
            "默认模型",
            "OpenAI Compatible",
            base_url,
            api_key,
            model_name,
        );
        config.active_provider_id = Some(provider.id.clone());
        config.providers.push(provider);
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url() {
        let mut config = AppConfig::default();
        config.providers.push(ProviderConfig::new(
            "n",
            "p",
            "https://example.com/v1///",
            "k",
            "m",
        ));
        normalize_config(&mut config);
        assert_eq!(config.providers[0].base_url, "https://example.com/v1");
    }
}
