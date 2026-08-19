//! 开机自启（薄封装，平台实现收敛到 `platform` 模块）。

use anyhow::Result;
use std::path::Path;

pub fn set_enabled(enabled: bool, exe_path: &Path) -> Result<()> {
    crate::platform::set_autostart(enabled, exe_path)
}