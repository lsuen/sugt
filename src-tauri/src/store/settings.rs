use crate::store::paths::StorePaths;
use crate::store::process::hidden_command;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSettings {
    pub editor_command: String,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            editor_command: String::new(),
        }
    }
}

pub fn load_settings(store_paths: &StorePaths) -> Result<StoreSettings> {
    if !store_paths.settings_file.exists() {
        let settings = StoreSettings::default();
        save_settings(store_paths, &settings)?;
        return Ok(settings);
    }
    let raw = std::fs::read_to_string(&store_paths.settings_file)
        .with_context(|| format!("无法读取 {}", store_paths.settings_file.display()))?;
    let settings: StoreSettings = toml::from_str(&raw).unwrap_or_default();
    Ok(settings)
}

pub fn save_settings(store_paths: &StorePaths, settings: &StoreSettings) -> Result<()> {
    store_paths.ensure_dirs()?;
    let serialized = toml::to_string_pretty(settings).context("商店设置序列化失败")?;
    std::fs::write(&store_paths.settings_file, serialized)
        .with_context(|| format!("无法写入 {}", store_paths.settings_file.display()))?;
    Ok(())
}

pub fn open_with_editor(editor_command: &str, path: &std::path::Path) -> Result<()> {
    if editor_command.trim().is_empty() {
        #[cfg(windows)]
        {
            hidden_command("explorer").arg(path).spawn()?;
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            hidden_command("open").arg(path).spawn()?;
            return Ok(());
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            hidden_command("xdg-open").arg(path).spawn()?;
            return Ok(());
        }
    }

    let mut parts = editor_command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("编辑器命令无效"))?;
    let args: Vec<&str> = parts.collect();
    let mut command = hidden_command(program);
    for arg in args {
        command.arg(arg);
    }
    command.arg(path);
    command.spawn().context("无法启动编辑器")?;
    Ok(())
}
