//! 跨平台接口：Windows / macOS / Linux 的系统层差异在此收敛。
//!
//! 业务代码只调用本模块导出的函数，不直接写 `#[cfg(target_os)]`。
//! 每个平台实现在对应子文件中，未实现的平台默认返回 `Err` 或空值。

pub mod common;

#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
pub mod linux;

use std::path::{Path, PathBuf};

use anyhow::Result;

// ---------- 打开目录 / 文件 ----------

pub fn open_path(path: &Path) -> Result<()> {
    #[cfg(windows)]
    return windows::open_path(path);
    #[cfg(target_os = "macos")]
    return macos::open_path(path);
    #[cfg(all(unix, not(target_os = "macos")))]
    return linux::open_path(path);
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("当前平台不支持打开目录"))
}

// ---------- 开机自启 ----------

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    #[cfg(windows)]
    return windows::set_autostart(enabled, exe_path);
    #[cfg(target_os = "macos")]
    return macos::set_autostart(enabled, exe_path);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (enabled, exe_path);
        Err(anyhow::anyhow!("当前平台暂不支持自动配置开机启动"))
    }
}

// ---------- 用户环境变量持久化 ----------

pub fn set_user_env(name: &str, value: &str) -> Result<()> {
    #[cfg(windows)]
    return windows::set_user_env(name, value);
    #[cfg(unix)]
    return unix_set_user_env(name, value);
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("当前平台暂不支持自动写入用户环境变量"))
}

pub fn unset_user_env(name: &str) -> Result<()> {
    #[cfg(windows)]
    return windows::unset_user_env(name);
    #[cfg(unix)]
    return unix_unset_user_env(name);
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("当前平台暂不支持自动删除用户环境变量"))
}

pub fn read_user_env(name: &str) -> Option<String> {
    #[cfg(windows)]
    return windows::read_user_env(name);
    #[cfg(unix)]
    return unix_read_user_env(name);
    #[allow(unreachable_code)]
    None
}

// ---------- 进程：隐藏启动 / 查找可执行 ----------

pub fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    common::find_command_on_path(cmd)
}

// ---------- 进程生命周期 ----------

pub fn is_pid_running(pid: u32) -> bool {
    #[cfg(windows)]
    return windows::is_pid_running(pid);
    #[cfg(unix)]
    return unix_is_pid_running(pid);
    #[allow(unreachable_code)]
    false
}

pub fn kill_pid(pid: u32) {
    #[cfg(windows)]
    windows::kill_pid(pid);
    #[cfg(unix)]
    unix_kill_pid(pid);
}

pub fn kill_process_by_name(name: &str) {
    #[cfg(windows)]
    windows::kill_process_by_image(format!("{}.exe", name).as_str());
    #[cfg(unix)]
    unix_kill_process_by_name(name);
}

// ---------- 机器指纹 ----------

pub fn machine_guid() -> Option<String> {
    #[cfg(windows)]
    return windows::machine_guid();
    #[cfg(target_os = "macos")]
    return macos::machine_guid();
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}

// ---------- Agent 可执行文件过滤 ----------

/// 判断一个文件是否可执行（按平台规则）。
pub fn is_executable(path: &Path) -> bool {
    #[cfg(windows)]
    return windows::is_executable(path);
    #[cfg(unix)]
    return unix_is_executable(path);
}

// ---------- 启动脚本生成 ----------

pub fn print_launch_script(listen_url: &str, api_key: &str) -> String {
    #[cfg(windows)]
    return windows::launch_script_text(listen_url, api_key);
    #[cfg(unix)]
    return unix_launch_script_text(listen_url, api_key);
    #[allow(unreachable_code)]
    String::new()
}

pub fn write_launch_scripts(config_dir: &Path, listen_url: &str, api_key: &str) -> Result<()> {
    #[cfg(windows)]
    return windows::write_launch_scripts(config_dir, listen_url, api_key);
    #[cfg(unix)]
    return unix_write_launch_scripts(config_dir, listen_url, api_key);
    #[allow(unreachable_code)]
    Ok(())
}

pub fn launch_script_basename(client: &str) -> String {
    #[cfg(windows)]
    return format!("{}-sugt.cmd", client.to_ascii_lowercase());
    #[cfg(not(windows))]
    return format!("{}-sugt.sh", client.to_ascii_lowercase());
}

// ---------- 从新终端启动客户端 ----------

pub fn launch_client_in_terminal(script: &Path, work_dir: &Path) -> Result<()> {
    #[cfg(windows)]
    return windows::launch_in_terminal(script, work_dir);
    #[cfg(target_os = "macos")]
    return macos::launch_in_terminal(script, work_dir);
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = (script, work_dir);
        Err(anyhow::anyhow!("当前平台暂不支持从新终端启动客户端"))
    }
}

// ---------- CLI 可执行名与常见安装位置 ----------

pub fn cli_exe_name(name: &str) -> String {
    #[cfg(windows)]
    return format!("{}.exe", name);
    #[cfg(not(windows))]
    return name.to_string();
}

pub fn is_app_exe(current_exe_name: &str, app_name: &str) -> bool {
    #[cfg(windows)]
    return current_exe_name.eq_ignore_ascii_case(&format!("{}.exe", app_name));
    #[cfg(not(windows))]
    return current_exe_name == app_name;
}

pub fn default_cli_search_paths(cmd: &str) -> Vec<PathBuf> {
    use crate::common::home_dir;
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        #[cfg(windows)]
        {
            if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                out.push(local.join("Programs").join(cmd).join(format!("{cmd}.exe")));
                out.push(local.join(cmd).join(format!("{cmd}.exe")));
                out.push(local.join("npm").join(format!("{cmd}.cmd")));
                out.push(local.join("npm").join(format!("{cmd}.exe")));
            }
            if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
                out.push(appdata.join("npm").join(format!("{cmd}.cmd")));
                out.push(appdata.join("npm").join(format!("{cmd}.exe")));
            }
            out.push(home.join("AppData").join("Roaming").join("npm").join(format!("{cmd}.cmd")));
            out.push(home.join(".local").join("bin").join(format!("{cmd}.exe")));
        }
        #[cfg(not(windows))]
        {
            out.push(home.join(".local").join("bin").join(cmd));
            out.push(PathBuf::from("/usr/local/bin").join(cmd));
            out.push(PathBuf::from("/opt/homebrew/bin").join(cmd));
            out.push(home.join(".npm-global").join("bin").join(cmd));
            let nvm_node = home.join(".nvm").join("versions").join("node");
            if let Ok(entries) = std::fs::read_dir(nvm_node) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("bin").join(cmd);
                    if bin.is_file() {
                        out.push(bin);
                        break;
                    }
                }
            }
        }
    }
    out
}

// ---------- 显示主窗口（Win 下需要 always_on_top 闪烁激活） ----------

pub fn activate_window(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.set_skip_taskbar(false);
    let _ = window.show();
    let _ = window.set_focus();
    #[cfg(windows)]
    {
        let _ = window.set_always_on_top(true);
        let _ = window.set_always_on_top(false);
    }
}

// ---------- Git 缺失提示 ----------

pub fn git_missing_hint() -> &'static str {
    #[cfg(windows)]
    return "未检测到 git，请先安装 Git for Windows";
    #[cfg(target_os = "macos")]
    return "未检测到 git，macOS 可在终端执行 xcode-select --install 安装";
    #[cfg(all(unix, not(target_os = "macos")))]
    return "未检测到 git，请安装 git";
}

// ---------- 子进程启动标记（隐藏窗口 / 进程分离） ----------

use std::process::Command;

pub trait CommandPlatformExt {
    fn no_window(&mut self) -> &mut Self;
    fn detached(&mut self) -> &mut Self;
}

impl CommandPlatformExt for Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }

    fn detached(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            self.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            self.process_group(0);
        }
        self
    }
}

// ========== Unix 共享实现 ==========

#[cfg(unix)]
fn unix_read_user_env(name: &str) -> Option<String> {
    let rc = std::fs::read_to_string(unix_shell_rc_path()).ok()?;
    rc.lines()
        .map(|l| l.trim_start())
        .filter_map(|l| l.strip_prefix("export "))
        .filter_map(|rest| rest.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| unix_unquote(v.trim()))
}

#[cfg(unix)]
fn unix_set_user_env(name: &str, value: &str) -> Result<()> {
    let path = unix_shell_rc_path();
    let rc = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = rc
        .lines()
        .filter(|l| !unix_is_export_line(l, name))
        .map(str::to_string)
        .collect();
    lines.push(format!("export {}={}", name, unix_shell_quote(value)));
    std::fs::write(&path, lines.join("\n") + "\n")?;
    std::env::set_var(name, value);
    Ok(())
}

#[cfg(unix)]
fn unix_unset_user_env(name: &str) -> Result<()> {
    let path = unix_shell_rc_path();
    let rc = std::fs::read_to_string(&path).unwrap_or_default();
    if rc.is_empty() {
        return Ok(());
    }
    let total = rc.lines().count();
    let kept: Vec<String> = rc
        .lines()
        .filter(|l| !unix_is_export_line(l, name))
        .map(str::to_string)
        .collect();
    if kept.len() == total {
        return Ok(());
    }
    std::fs::write(&path, kept.join("\n") + "\n")?;
    std::env::remove_var(name);
    Ok(())
}

#[cfg(unix)]
fn unix_is_export_line(line: &str, name: &str) -> bool {
    line.trim_start()
        .strip_prefix("export ")
        .and_then(|rest| rest.split('=').next())
        .map(|k| k.trim() == name)
        .unwrap_or(false)
}

#[cfg(unix)]
fn unix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn unix_unquote(value: &str) -> String {
    let b = value.as_bytes();
    if b.len() >= 2 {
        match (b[0], b[b.len() - 1]) {
            (b'\'', b'\'') | (b'"', b'"') => return value[1..value.len() - 1].to_string(),
            _ => {}
        }
    }
    value.to_string()
}

#[cfg(unix)]
fn unix_shell_rc_path() -> PathBuf {
    use crate::common::home_dir;
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zshrc")
}

#[cfg(unix)]
fn unix_is_pid_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn unix_kill_pid(pid: u32) {
    use std::process::Stdio;
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn unix_kill_process_by_name(name: &str) {
    use std::process::Stdio;
    let _ = std::process::Command::new("pkill")
        .args(["-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn unix_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(unix)]
fn unix_launch_script_text(listen_url: &str, key: &str) -> String {
    format!(
        "export ANTHROPIC_BASE_URL={}\nexport ANTHROPIC_AUTH_TOKEN={}\n\
         export OPENAI_BASE_URL={}/v1\nexport OPENAI_API_BASE={}/v1\n\
         export OPENAI_API_KEY={}\n",
        listen_url, key, listen_url, listen_url, key
    )
}

#[cfg(unix)]
fn unix_write_launch_scripts(dir: &Path, listen_url: &str, key: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let make = |name: &str, client_cmd: &str| -> Result<()> {
        let body = format!(
            "#!/bin/sh\nexport ANTHROPIC_BASE_URL={}\nexport ANTHROPIC_AUTH_TOKEN={}\n\
             export ANTHROPIC_API_KEY=\nexport CLAUDE_CODE_API_KEY=\n\
             export OPENAI_BASE_URL={}/v1\nexport OPENAI_API_BASE={}/v1\n\
             export OPENAI_API_KEY={}\nexec {} \"$@\"\n",
            listen_url, key, listen_url, listen_url, key, client_cmd
        );
        let p = dir.join(name);
        std::fs::write(&p, body)?;
        let mut perms = std::fs::metadata(&p)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(&p, perms)?;
        Ok(())
    };
    make("claude-sugt.sh", "claude")?;
    make("codex-sugt.sh", "codex")?;
    Ok(())
}

// ---------- 公共工具（跨平台通用） ----------

// 这里导出一些函数，让现有模块可以通过 `platform::common::xxx` 调用。
// 同时在本文件 `pub use` 最常用的几个。
