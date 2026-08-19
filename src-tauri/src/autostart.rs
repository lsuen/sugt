use anyhow::{bail, Result};
use std::path::Path;

#[cfg(windows)]
pub fn set_enabled(enabled: bool, exe_path: &Path) -> Result<()> {
    use crate::process_util::hidden_command;
    let name = "SUGT";
    let status = if enabled {
        hidden_command("reg")
            .args([
                "add",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                name,
                "/t",
                "REG_SZ",
                "/d",
            ])
            .arg(format!("\"{}\"", exe_path.display()))
            .args(["/f"])
            .status()?
    } else {
        hidden_command("reg")
            .args([
                "delete",
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                "/v",
                name,
                "/f",
            ])
            .status()?
    };

    if !status.success() && enabled {
        bail!("开机启动注册失败")
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn set_enabled(enabled: bool, exe_path: &Path) -> Result<()> {
    const LABEL: &str = "com.sugt.app";
    let agents_dir = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join("Library/LaunchAgents"))
        .ok_or_else(|| anyhow::anyhow!("无法定位用户目录"))?;
    std::fs::create_dir_all(&agents_dir)?;
    let plist_path = agents_dir.join(format!("{}.plist", LABEL));

    if enabled {
        let plist = format!(
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
            label = LABEL,
            exe = xml_escape(&exe_path.display().to_string())
        );
        std::fs::write(&plist_path, plist)?;
        let status = std::process::Command::new("launchctl")
            .arg("load")
            .arg(&plist_path)
            .status()?;
        if !status.success() {
            bail!("launchctl load 失败");
        }
    } else if plist_path.exists() {
        // 卸载并删除 plist，忽略 unload 失败（未加载过时可能报错）
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&plist_path)
            .status();
        let _ = std::fs::remove_file(&plist_path);
    }
    Ok(())
}

/// plist 是 XML，路径中的 `&`、`<`、`>` 需要转义
#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_enabled(_enabled: bool, _exe_path: &Path) -> Result<()> {
    bail!("当前平台暂不支持自动配置开机启动")
}
