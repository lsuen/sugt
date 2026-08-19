//! Linux 平台实现（最小可用）。
//!
//! 功能不完整，逐步补齐。未实现的走 platform::mod.rs 中的默认返回。

use std::path::{Path, PathBuf};

use anyhow::Result;

pub fn open_path(path: &Path) -> Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

pub fn config_dir(app_name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            crate::common::home_dir().map(|h| h.join(".config"))
        })?;
    Some(base.join(app_name.to_ascii_lowercase()))
}
