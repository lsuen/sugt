//! 跨平台通用工具：不依赖 `#[cfg]` 的实现放在这里。

use std::path::{Path, PathBuf};

/// 在 `PATH` 中查找可执行文件，不调用 `where`/`which`。
pub fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    let path_val = std::env::var_os("PATH")?;
    let exts = path_extensions();

    std::env::split_paths(&path_val)
        .find_map(|dir| resolve_in_dir(&dir, cmd, &exts))
}

fn path_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var_os("PATHEXT")
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }
    #[cfg(not(windows))]
    Vec::new()
}

fn resolve_in_dir(dir: &Path, cmd: &str, exts: &[String]) -> Option<PathBuf> {
    let candidate = dir.join(cmd);
    if is_runnable(&candidate) {
        return Some(candidate);
    }
    #[cfg(windows)]
    {
        let has_ext = Path::new(cmd)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| exts.iter().any(|x| x.trim_start_matches('.') == e.to_ascii_lowercase()))
            .unwrap_or(false);
        if !has_ext {
            return exts
                .iter()
                .map(|ext| dir.join(format!("{cmd}{ext}")))
                .find(|p| is_runnable(p));
        }
    }
    let _ = exts;
    None
}

fn is_runnable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_common_tools_on_path() {
        #[cfg(windows)]
        assert!(find_command_on_path("cmd").is_some() || find_command_on_path("where").is_some());
        #[cfg(not(windows))]
        assert!(find_command_on_path("sh").is_some() || find_command_on_path("bash").is_some());
    }
}
