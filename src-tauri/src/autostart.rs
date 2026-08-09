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
pub fn set_enabled(_enabled: bool, _exe_path: &Path) -> Result<()> {
    bail!("macOS 开机启动将在后续版本通过 LaunchAgent 支持")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_enabled(_enabled: bool, _exe_path: &Path) -> Result<()> {
    bail!("当前平台暂不支持自动配置开机启动")
}
