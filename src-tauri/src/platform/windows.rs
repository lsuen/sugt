//! Windows 平台实现。
//!
//! 优先用系统自带工具（reg、setx、tasklist、taskkill），避免引入额外依赖。
//! 全部子进程通过 CREATE_NO_WINDOW 隐藏黑窗口。

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

const AUTOSTART_REG_NAME: &str = "SUGT";

// ---------- 打开目录 / 文件 ----------

pub fn open_path(path: &Path) -> Result<()> {
    hidden("explorer").arg(path).spawn()?;
    Ok(())
}

// ---------- 开机自启 ----------

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    let status = if enabled {
        hidden("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                AUTOSTART_REG_NAME,
                "/t",
                "REG_SZ",
                "/d",
            ])
            .arg(format!("\"{}\"", exe_path.display()))
            .args(["/f"])
            .status()?
    } else {
        hidden("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                AUTOSTART_REG_NAME,
                "/f",
            ])
            .status()?
    };
    if !status.success() && enabled {
        return Err(anyhow::anyhow!("开机启动注册失败"));
    }
    Ok(())
}

// ---------- 用户环境变量持久化 ----------

pub fn set_user_env(name: &str, value: &str) -> Result<()> {
    let ok = hidden("setx")
        .arg(name)
        .arg(value)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    if ok {
        std::env::set_var(name, value);
        Ok(())
    } else {
        Err(anyhow::anyhow!("setx 写入 {} 失败", name))
    }
}

pub fn unset_user_env(name: &str) -> Result<()> {
    // 变量不存在时 reg delete 也会失败，清理场景视为成功
    let _ = hidden("reg")
        .args(["delete", r"HKCU\Environment", "/v", name, "/f"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    std::env::remove_var(name);
    Ok(())
}

pub fn read_user_env(name: &str) -> Option<String> {
    let output = hidden("reg")
        .args(["query", r"HKCU\Environment", "/v", name])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case(name) {
            if matches!(parts[1], "REG_SZ" | "REG_EXPAND_SZ") {
                return Some(parts[2..].join(" "));
            }
        }
    }
    None
}

// ---------- 进程生命周期 ----------

pub fn is_pid_running(pid: u32) -> bool {
    let output = hidden("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    output
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

pub fn kill_pid(pid: u32) {
    let _ = hidden("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn kill_process_by_image(image_name: &str) {
    let _ = hidden("taskkill")
        .args(["/IM", image_name, "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ---------- 机器指纹 ----------

pub fn machine_guid() -> Option<String> {
    let output = hidden("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.contains("MachineGuid"))
        .and_then(|l| l.split_whitespace().last())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---------- Agent 可执行文件过滤 ----------

pub fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "exe" | "cmd" | "bat" | "ps1")
}

// ---------- 启动脚本 ----------

pub fn launch_script_text(listen_url: &str, key: &str) -> String {
    format!(
        "set ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\n\
         set OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\n\
         set OPENAI_API_KEY={}\r\n",
        listen_url, key, listen_url, listen_url, key
    )
}

pub fn write_launch_scripts(dir: &Path, listen_url: &str, key: &str) -> Result<()> {
    let claude = format!(
        "@echo off\r\nset ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\n\
         set ANTHROPIC_API_KEY=\r\nset CLAUDE_CODE_API_KEY=\r\nclaude %*\r\n",
        listen_url, key
    );
    let codex = format!(
        "@echo off\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\n\
         set OPENAI_API_KEY={}\r\ncodex %*\r\n",
        listen_url, listen_url, key
    );
    std::fs::write(dir.join("claude-sugt.cmd"), claude)?;
    std::fs::write(dir.join("codex-sugt.cmd"), codex)?;
    Ok(())
}

// ---------- 从新终端启动客户端 ----------

pub fn launch_in_terminal(script: &Path, work_dir: &Path) -> Result<()> {
    hidden("cmd")
        .args([
            "/c",
            "start",
            "",
            "/D",
            work_dir.as_os_str().to_string_lossy().as_ref(),
            script.as_os_str().to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

// ---------- 注册表环境变量全量读取（Agent discover 用） ----------

pub fn read_reg_env(key: &str) -> Vec<(String, String)> {
    let output = match hidden("reg")
        .args(["query", key])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        if parts.len() >= 3 && matches!(parts[1], "REG_SZ" | "REG_EXPAND_SZ" | "REG_MULTI_SZ") {
            out.push((parts[0].to_string(), parts[2..].join(" ")));
        }
    }
    out
}

// ---------- 内部工具 ----------

fn hidden(program: impl AsRef<std::ffi::OsStr>) -> Command {
    use crate::platform::CommandPlatformExt;
    let mut cmd = Command::new(program);
    cmd.no_window().stdin(Stdio::null());
    cmd
}

// ---------- 测试辅助：公开内部函数给集成测试 ----------

#[cfg(test)]
pub mod test_hooks {
    pub use super::is_executable as is_executable_internal;
}
