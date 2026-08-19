//! macOS 平台实现。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

const LAUNCHAGENT_LABEL: &str = "com.sugt.app";

// ---------- 打开目录 / 文件 ----------

pub fn open_path(path: &Path) -> Result<()> {
    Command::new("open").arg(path).spawn()?;
    Ok(())
}

// ---------- 开机自启（LaunchAgent） ----------

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    let agents_dir = crate::common::home_dir()
        .map(|h| h.join("Library/LaunchAgents"))
        .ok_or_else(|| anyhow::anyhow!("无法定位用户目录"))?;
    std::fs::create_dir_all(&agents_dir)?;
    let plist_path = agents_dir.join(format!("{LAUNCHAGENT_LABEL}.plist"));

    if enabled {
        let plist = plist_content(LAUNCHAGENT_LABEL, &exe_path.display().to_string());
        std::fs::write(&plist_path, plist)?;
        let status = Command::new("launchctl")
            .arg("load")
            .arg(&plist_path)
            .status()?;
        if !status.success() {
            return Err(anyhow::anyhow!("launchctl load 失败"));
        }
    } else if plist_path.exists() {
        // 未加载时 unload 也可能失败，忽略
        let _ = Command::new("launchctl")
            .arg("unload")
            .arg(&plist_path)
            .status();
        let _ = std::fs::remove_file(&plist_path);
    }
    Ok(())
}

fn plist_content(label: &str, exe: &str) -> String {
    let exe_escaped = xml_escape(exe);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        label = label,
        exe = exe_escaped,
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------- 机器指纹 ----------

pub fn machine_guid() -> Option<String> {
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().find_map(|line| {
        let idx = line.find("\"IOPlatformUUID\"")?;
        let rest = &line[idx..];
        let eq = rest.find('=')?;
        let value = rest[eq + 1..].trim().trim_matches('"');
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
}

// ---------- 从新终端启动客户端（AppleScript 调 Terminal.app） ----------

pub fn launch_in_terminal(script: &Path, work_dir: &Path) -> Result<()> {
    let work = work_dir.to_string_lossy();
    let script_str = script.to_string_lossy();
    // 单引号内层用 '\'' 转义，然后用双引号包整个 do script 参数
    let command = format!("cd '{}' && '{}'", work, script_str);
    let escaped = command.replace('\\', "\\\\").replace('"', "\\\"");
    Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "tell application \"Terminal\" to do script \"{}\"",
            escaped
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

// ---------- 预留：macOS 特有配置助手辅助函数 ----------

/// 返回主要 shell rc 文件路径（根据 $SHELL 判断）。
pub fn shell_rc_path() -> PathBuf {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let home = crate::common::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match shell.as_str() {
        s if s.ends_with("/bash") => home.join(".bashrc"),
        s if s.ends_with("/fish") => home.join(".config/fish/config.fish"),
        _ => home.join(".zshrc"),
    }
}
