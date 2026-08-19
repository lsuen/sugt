//! 子进程快捷构造器（薄封装，平台实现收敛到 `platform` 模块）。
//!
//! 所有外部调用方应通过本模块获取 `Command`，确保跨平台行为一致。

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use crate::platform::CommandPlatformExt;

/// 创建无控制台窗口的子进程命令。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.no_window().stdin(Stdio::null());
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

/// 运行隐藏命令并捕获输出。
pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    hidden_command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

/// 在 PATH 中查找可执行文件（不调用 which/where，避免弹窗）。
pub fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    crate::platform::common::find_command_on_path(cmd)
}