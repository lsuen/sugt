//! Windows 下隐藏控制台子进程，避免 GUI 操作时黑窗口闪烁。

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// 创建不弹控制台的子进程命令（Windows：CREATE_NO_WINDOW）。
pub fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
}

/// Git：隐藏窗口 + 禁止交互凭据提示。
pub fn hidden_git() -> Command {
    let mut cmd = hidden_command("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GCM_INTERACTIVE", "never");
    cmd.env("GIT_ASKPASS", "echo");
    cmd
}

pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    hidden_command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

/// 在 PATH 中查找可执行文件，不调用 `where`/`which`（避免控制台闪烁）。
pub fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    let path_val = std::env::var_os("PATH")?;
    let exts = path_extensions();

    for dir in std::env::split_paths(&path_val) {
        if let Some(found) = resolve_in_dir(&dir, cmd, &exts) {
            return Some(found);
        }
    }
    None
}

fn path_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        let raw = std::env::var_os("PATHEXT")
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        raw.split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn resolve_in_dir(dir: &Path, cmd: &str, exts: &[String]) -> Option<PathBuf> {
    let candidate = dir.join(cmd);
    if is_runnable(&candidate) {
        return Some(candidate);
    }
    #[cfg(windows)]
    {
        let lower = cmd.to_ascii_lowercase();
        let has_ext = Path::new(cmd)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| exts.iter().any(|x| x.trim_start_matches('.') == e.to_ascii_lowercase()))
            .unwrap_or(false);
        if !has_ext {
            for ext in exts {
                let with_ext = dir.join(format!("{cmd}{ext}"));
                if is_runnable(&with_ext) {
                    return Some(with_ext);
                }
                // 也兼容用户传入已带点后缀的大小写差异
                let _ = lower;
            }
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
    fn finds_cmd_like_tools_without_where() {
        // Windows 上 `where`/`cmd` 通常在 System32；用 PATH 扫描应能找到
        #[cfg(windows)]
        {
            assert!(find_command_on_path("cmd").is_some() || find_command_on_path("where").is_some());
        }
        #[cfg(not(windows))]
        {
            assert!(find_command_on_path("sh").is_some() || find_command_on_path("bash").is_some());
        }
    }
}
