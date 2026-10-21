use crate::model::{AppConfig, ProviderConfig};
use anyhow::{Context, Result};
use directories::UserDirs;
use std::{fs, path::{Path, PathBuf}};

const DIR_NAME: &str = ".sugt";
const CONFIG_FILE: &str = "config.toml";
const LOG_FILE: &str = "sugt.log";
/// macOS 本地配置目录名（放 Application Support，避开「桌面与文稿」iCloud 同步）。
const MACOS_APP_SUPPORT_NAME: &str = "sugt";

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
        default_config_dir(&user_dirs)?
    };

    Ok(AppPaths {
        config_dir: base.clone(),
        config_file: base.join(CONFIG_FILE),
        log_file: base.join(LOG_FILE),
    })
}

fn documents_sugt(user_dirs: &UserDirs) -> PathBuf {
    user_dirs
        .document_dir()
        .unwrap_or_else(|| user_dirs.home_dir())
        .join(DIR_NAME)
}

/// 解析默认配置目录。
///
/// macOS：优先 `~/Library/Application Support/sugt`，避免打开目录时落到 iCloud 文稿。
/// 若旧版数据仍在 `Documents/.sugt` 且文稿已进 iCloud，则迁到 Application Support，
/// 并在原位置留符号链接，兼容历史绝对路径。
fn default_config_dir(user_dirs: &UserDirs) -> Result<PathBuf> {
    let legacy = documents_sugt(user_dirs);

    #[cfg(target_os = "macos")]
    {
        let preferred = user_dirs
            .home_dir()
            .join("Library/Application Support")
            .join(MACOS_APP_SUPPORT_NAME);
        if preferred.exists() {
            return Ok(preferred);
        }
        if legacy.exists() && documents_synced_via_icloud(user_dirs) {
            migrate_legacy_config_dir(&legacy, &preferred)?;
            return Ok(preferred);
        }
        if legacy.exists() {
            return Ok(legacy);
        }
        return Ok(preferred);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = MACOS_APP_SUPPORT_NAME;
        Ok(legacy)
    }
}

/// 文稿是否被「桌面与文稿」同步进 iCloud（与 Mobile Documents 下的 Documents 为同一目录）。
#[cfg(target_os = "macos")]
fn documents_synced_via_icloud(user_dirs: &UserDirs) -> bool {
    let Some(docs) = user_dirs.document_dir() else {
        return false;
    };
    let mobile_docs = user_dirs
        .home_dir()
        .join("Library/Mobile Documents/com~apple~CloudDocs/Documents");
    mobile_docs.is_dir()
        && fs::canonicalize(&docs)
            .ok()
            .zip(fs::canonicalize(&mobile_docs).ok())
            .is_some_and(|(a, b)| a == b)
}

#[cfg(target_os = "macos")]
fn migrate_legacy_config_dir(legacy: &Path, preferred: &Path) -> Result<()> {
    if let Some(parent) = preferred.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建 {}", parent.display()))?;
    }
    match fs::rename(legacy, preferred) {
        Ok(()) => {}
        Err(err) => {
            // 跨卷或目录非空时 rename 可能失败，退回拷贝。
            copy_dir_recursive(legacy, preferred).with_context(|| {
                format!(
                    "无法从 {} 迁到 {}（rename 失败：{err}）",
                    legacy.display(),
                    preferred.display()
                )
            })?;
            let _ = fs::remove_dir_all(legacy);
        }
    }
    // 原 Documents/.sugt 留链接，旧对话里的绝对路径仍能落到同一套文件。
    if !legacy.exists() {
        let _ = std::os::unix::fs::symlink(preferred, legacy);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn ensure_paths() -> Result<AppPaths> {
    let paths = resolve_paths()?;
    fs::create_dir_all(&paths.config_dir)
        .with_context(|| format!("无法创建配置目录 {}", paths.config_dir.display()))?;
    Ok(paths)
}

pub fn load_config_file(path: &Path) -> Result<AppConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("无法读取配置文件 {}", path.display()))?;
    let mut config: AppConfig = toml::from_str(&raw).context("配置文件格式错误")?;
    normalize_config(&mut config);
    Ok(config)
}

pub fn load_or_init_config() -> Result<(AppPaths, AppConfig)> {
    let paths = ensure_paths()?;
    if !paths.config_file.exists() {
        let mut config = default_config_from_env();
        let _ = crate::experimental_free::ensure_experimental_free(&mut config);
        normalize_config(&mut config);
        save_config(&paths, &config)?;
        return Ok((paths, config));
    }

    let mut config = load_config_file(&paths.config_file)?;
    let seeded = crate::experimental_free::ensure_experimental_free(&mut config);
    normalize_config(&mut config);
    if seeded {
        let _ = save_config(&paths, &config);
    }
    Ok((paths, config))
}

pub fn save_config(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    fs::create_dir_all(&paths.config_dir)?;
    let serialized = toml::to_string_pretty(config).context("配置序列化失败")?;
    // 原子写入：先写临时文件，再 rename，避免网关热加载读到半截文件
    let tmp = paths.config_file.with_extension("tmp");
    fs::write(&tmp, &serialized)
        .with_context(|| format!("无法写入临时文件 {}", tmp.display()))?;
    fs::rename(&tmp, &paths.config_file)
        .with_context(|| format!("无法重命名配置文件 {}", paths.config_file.display()))
}

pub fn normalize_config(config: &mut AppConfig) {
    for provider in &mut config.providers {
        provider.base_url = crate::model::resolve_provider_base_url(
            provider.base_url.clone(),
            &provider.protocol,
            provider.auto_adapt_base_url,
        );
    }

    if config.active_provider_id.is_none() {
        config.active_provider_id = config
            .providers
            .iter()
            .find(|provider| provider.enabled)
            .map(|provider| provider.id.clone());
    }

    config.workspace_rules.normalize();
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

    #[cfg(target_os = "macos")]
    #[test]
    fn migrate_legacy_leaves_symlink() {
        let root = std::env::temp_dir().join(format!(
            "sugt-migrate-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        let legacy = root.join("Documents").join(".sugt");
        let preferred = root.join("Library/Application Support/sugt");
        fs::create_dir_all(legacy.join("projects/default")).unwrap();
        fs::write(legacy.join("projects/default/a.txt"), "hi").unwrap();
        migrate_legacy_config_dir(&legacy, &preferred).unwrap();
        assert!(preferred.join("projects/default/a.txt").is_file());
        assert!(legacy.is_symlink() || fs::canonicalize(&legacy).unwrap() == fs::canonicalize(&preferred).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
