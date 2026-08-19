//! 跨平台公共工具，不依赖 Tauri 或 IO 层。

use std::path::PathBuf;

/// 返回用户主目录。
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|d| d.home_dir().to_path_buf())
}