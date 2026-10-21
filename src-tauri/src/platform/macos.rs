//! macOS 平台实现。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

const LAUNCHAGENT_LABEL: &str = "com.sugt.app";

// ---------- 打开目录 / 文件 ----------

pub fn open_path(path: &Path) -> Result<()> {
    // 规范化后再打开，避免相对路径 / iCloud「Mobile Documents」别名让访达跳到错误位置。
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let status = Command::new("open").arg(&path).status()?;
    if !status.success() {
        anyhow::bail!("open 失败：{}（status {status}）", path.display());
    }
    Ok(())
}

// ---------- 开机自启（LaunchAgent） ----------

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<()> {
    let agents_dir = crate::common::home_dir()
        .map(|h| h.join("Library/LaunchAgents"))
        .ok_or_else(|| anyhow::anyhow!("无法定位用户目录"))?;
    apply_launch_agent(&agents_dir, LAUNCHAGENT_LABEL, enabled, exe_path)
}

/// 写入或移除 LaunchAgent，并用 `bootstrap` / `bootout` 让当前登录会话立即生效。
///
/// macOS 15 上对已加载的 plist 再执行 `launchctl load` 会打印失败，但退出码仍是 0，
/// 新的可执行路径不会换上去。所以启用时先 bootout 再 bootstrap。
fn apply_launch_agent(
    agents_dir: &Path,
    label: &str,
    enabled: bool,
    exe_path: &Path,
) -> Result<()> {
    std::fs::create_dir_all(agents_dir)?;
    let plist_path = agents_dir.join(format!("{label}.plist"));
    let service = format!("gui/{}/{label}", current_uid());

    if enabled {
        let plist = plist_content(label, &exe_path.display().to_string());
        std::fs::write(&plist_path, plist)?;
        let _ = launchctl(&["bootout", &service]);
        let domain = format!("gui/{}", current_uid());
        let plist_display = plist_path.display().to_string();
        let output = launchctl(&["bootstrap", &domain, &plist_display])?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("launchctl bootstrap 失败: {}", err.trim()));
        }
    } else {
        let _ = launchctl(&["bootout", &service]);
        if plist_path.exists() {
            let _ = std::fs::remove_file(&plist_path);
        }
    }
    Ok(())
}

fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("launchctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| anyhow::anyhow!("无法执行 launchctl: {err}"))
}

fn current_uid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
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
    parse_platform_uuid(&String::from_utf8_lossy(&output.stdout))
}

fn parse_platform_uuid(text: &str) -> Option<String> {
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
    rc_file_for_shell(&shell, &home)
}

fn rc_file_for_shell(shell: &str, home: &Path) -> PathBuf {
    if shell.ends_with("/bash") {
        home.join(".bashrc")
    } else if shell.ends_with("/fish") {
        home.join(".config/fish/config.fish")
    } else {
        home.join(".zshrc")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ioreg_platform_uuid() {
        let sample = r#"
+-o Root  <class IORegistryEntry>
  | {
  |   "IOPlatformUUID" = "FF50318C-4B1F-5B3B-BBA9-7CF90447D4C3"
  | }
"#;
        assert_eq!(
            parse_platform_uuid(sample).as_deref(),
            Some("FF50318C-4B1F-5B3B-BBA9-7CF90447D4C3")
        );
    }

    #[test]
    fn machine_guid_matches_live_platform_uuid() {
        let id = machine_guid().expect("应能读到 IOPlatformUUID");
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn plist_escapes_xml_and_keeps_path_as_one_argument() {
        let plist = plist_content("com.sugt.app", "/Users/a&b/<SUTAI>.app/Contents/MacOS/sugt");
        assert!(plist.contains("<string>com.sugt.app</string>"));
        assert!(plist.contains("/Users/a&amp;b/&lt;SUTAI&gt;.app/Contents/MacOS/sugt"));
        assert!(!plist.contains("/Users/a&b/"));
    }

    #[test]
    fn shell_rc_follows_login_shell() {
        let home = Path::new("/Users/demo");
        assert_eq!(rc_file_for_shell("/bin/zsh", home), home.join(".zshrc"));
        assert_eq!(rc_file_for_shell("/bin/bash", home), home.join(".bashrc"));
        assert_eq!(
            rc_file_for_shell("/opt/homebrew/bin/fish", home),
            home.join(".config/fish/config.fish")
        );
    }

    #[test]
    fn launch_agent_reloads_when_enabled_twice() {
        let uid = current_uid();
        let label = format!("com.sugt.test.{}.{}", uid, std::process::id());
        let dir = std::env::temp_dir().join(format!("sugt-launch-{label}"));
        let plist = dir.join(format!("{label}.plist"));
        let _guard = LaunchAgentGuard {
            service: format!("gui/{uid}/{label}"),
            plist: plist.clone(),
            dir: dir.clone(),
        };

        apply_launch_agent(&dir, &label, true, Path::new("/usr/bin/true")).expect("首次 bootstrap");
        apply_launch_agent(&dir, &label, true, Path::new("/usr/bin/true"))
            .expect("已加载时再次启用仍应成功");

        let printed = Command::new("launchctl")
            .args(["print", &format!("gui/{uid}/{label}")])
            .output()
            .expect("launchctl print");
        assert!(printed.status.success());
        let text = String::from_utf8_lossy(&printed.stdout);
        assert!(text.contains("/usr/bin/true"));

        apply_launch_agent(&dir, &label, false, Path::new("/usr/bin/true")).expect("关闭自启");
        let gone = Command::new("launchctl")
            .args(["print", &format!("gui/{uid}/{label}")])
            .output()
            .expect("launchctl print after bootout");
        assert!(!gone.status.success());
        assert!(!plist.exists());
    }

    struct LaunchAgentGuard {
        service: String,
        plist: PathBuf,
        dir: PathBuf,
    }

    impl Drop for LaunchAgentGuard {
        fn drop(&mut self) {
            let _ = Command::new("launchctl")
                .args(["bootout", &self.service])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = std::fs::remove_file(&self.plist);
            let _ = std::fs::remove_dir(&self.dir);
        }
    }
}
