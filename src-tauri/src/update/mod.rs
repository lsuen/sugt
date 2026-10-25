//! 应用内检查更新与安装：GitHub Release + 代理/镜像回退。

mod channel;
pub mod commands;

use crate::config::AppPaths;
use crate::platform;
use crate::store::paths::StorePaths;
use crate::store::settings::load_settings;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub use channel::{ChannelAttempt, ChannelState};

const RELEASES_LATEST_API: &str = "https://api.github.com/repos/lsuen/sugt/releases/latest";
const RELEASE_PAGE: &str = "https://github.com/lsuen/sugt/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_notes: String,
    pub release_url: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub asset_sha256: Option<String>,
    pub channel_label: String,
    pub channel_prefix: String,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyResult {
    pub message: String,
    pub will_exit: bool,
    pub channel_label: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    /// GitHub 新字段，形如 `sha256:abcd...`
    #[serde(default)]
    digest: Option<String>,
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn store_paths_from_config(config_dir: &Path) -> StorePaths {
    StorePaths::from_app(&AppPaths {
        config_dir: config_dir.to_path_buf(),
        config_file: config_dir.join("config.toml"),
        log_file: config_dir.join("sugt.log"),
    })
}

fn user_github_proxy(config_dir: &Path) -> String {
    load_settings(&store_paths_from_config(config_dir))
        .map(|s| s.github_proxy_prefix)
        .unwrap_or_default()
}

/// 比较 a 与 b（允许 v 前缀）。`a < b` 返回负，相等 0，大于正。
pub fn cmp_version(a: &str, b: &str) -> i32 {
    let pa = parse_version(a);
    let pb = parse_version(b);
    for i in 0..3 {
        if pa[i] < pb[i] {
            return -1;
        }
        if pa[i] > pb[i] {
            return 1;
        }
    }
    0
}

fn parse_version(raw: &str) -> [u64; 3] {
    let s = raw.trim().trim_start_matches('v').trim_start_matches('V');
    let mut out = [0u64; 3];
    for (i, part) in s.split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        out[i] = digits.parse().unwrap_or(0);
    }
    out
}

fn platform_key() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x64"
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        "unsupported"
    }
}

fn pick_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    let names: Vec<&str> = assets.iter().map(|a| a.name.as_str()).collect();
    let _ = names;

    #[cfg(target_os = "windows")]
    {
        return assets
            .iter()
            .find(|a| {
                let n = a.name.to_lowercase();
                n.contains("setup") && n.ends_with(".exe")
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = a.name.to_lowercase();
                    n.contains("windows") && n.ends_with(".zip")
                })
            });
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return assets.iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("macos") && n.contains("arm64") && n.ends_with(".zip")
        });
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return assets.iter().find(|a| {
            let n = a.name.to_lowercase();
            n.contains("macos") && (n.contains("x64") || n.contains("x86_64")) && n.ends_with(".zip")
        });
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = assets;
        None
    }
}

fn parse_sha256(digest: &Option<String>) -> Option<String> {
    let d = digest.as_ref()?.trim();
    let hex = if let Some(rest) = d.strip_prefix("sha256:") {
        rest
    } else if d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()) {
        d
    } else {
        return None;
    };
    Some(hex.to_ascii_lowercase())
}

pub async fn check_for_update(config_dir: &Path) -> Result<UpdateInfo> {
    let user_prefix = user_github_proxy(config_dir);
    let (release, attempt): (GhRelease, ChannelAttempt) =
        channel::get_json_with_fallback(config_dir, &user_prefix, RELEASES_LATEST_API).await?;

    let latest = release.tag_name.trim().trim_start_matches('v').to_string();
    let current = current_version();
    let update_available = cmp_version(&current, &latest) < 0;
    let asset = pick_asset(&release.assets);

    Ok(UpdateInfo {
        current_version: current,
        latest_version: latest,
        update_available,
        release_notes: release.body,
        release_url: if release.html_url.is_empty() {
            RELEASE_PAGE.to_string()
        } else {
            release.html_url
        },
        asset_name: asset.map(|a| a.name.clone()),
        asset_url: asset.map(|a| a.browser_download_url.clone()),
        asset_sha256: asset.and_then(|a| parse_sha256(&a.digest)),
        channel_label: attempt.label,
        channel_prefix: attempt.prefix,
        platform: platform_key().to_string(),
    })
}

pub fn open_release_page(url: Option<&str>) -> Result<()> {
    let target = url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(RELEASE_PAGE);
    open_url(target)
}

fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open").arg(url).status()?;
        if !status.success() {
            bail!("无法打开浏览器");
        }
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = Command::new("xdg-open").arg(url).status()?;
        if !status.success() {
            bail!("无法打开浏览器");
        }
        Ok(())
    }
}

pub async fn download_and_apply(
    config_dir: &Path,
    info: &UpdateInfo,
    on_progress: impl Fn(UpdateProgress),
) -> Result<ApplyResult> {
    if !info.update_available {
        bail!("当前已是最新版本");
    }
    let asset_url = info
        .asset_url
        .as_deref()
        .ok_or_else(|| anyhow!("当前平台暂无自动安装包，请打开发布页手动下载"))?;
    let asset_name = info
        .asset_name
        .as_deref()
        .unwrap_or("update.bin");

    if platform_key() == "unsupported" {
        bail!("当前平台暂不支持应用内更新，请打开发布页手动下载");
    }

    on_progress(UpdateProgress {
        phase: "download".into(),
        downloaded: 0,
        total: None,
        message: "正在下载…".into(),
    });

    let user_prefix = user_github_proxy(config_dir);
    let preferred = if info.channel_prefix.is_empty() {
        None
    } else {
        Some(info.channel_prefix.as_str())
    };

    let (bytes, attempt) = channel::download_bytes_with_fallback(
        config_dir,
        &user_prefix,
        preferred,
        asset_url,
        |downloaded, total| {
            on_progress(UpdateProgress {
                phase: "download".into(),
                downloaded,
                total,
                message: match total {
                    Some(t) if t > 0 => format!(
                        "下载中 {:.0}%",
                        (downloaded as f64 / t as f64) * 100.0
                    ),
                    _ => format!("已下载 {} KB", downloaded / 1024),
                },
            });
        },
    )
    .await?;

    if let Some(expect) = info.asset_sha256.as_deref() {
        on_progress(UpdateProgress {
            phase: "verify".into(),
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            message: "校验文件…".into(),
        });
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let got = format!("{:x}", hasher.finalize());
        if !got.eq_ignore_ascii_case(expect) {
            bail!("校验失败：SHA256 不匹配（可能镜像被篡改，请换通道或打开发布页）");
        }
    }

    let staging = config_dir.join("updates").join(&info.latest_version);
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("无法创建 {}", staging.display()))?;
    let package_path = staging.join(asset_name);
    std::fs::write(&package_path, &bytes)
        .with_context(|| format!("无法写入 {}", package_path.display()))?;

    on_progress(UpdateProgress {
        phase: "install".into(),
        downloaded: bytes.len() as u64,
        total: Some(bytes.len() as u64),
        message: "准备安装…".into(),
    });

    let result = apply_package(&package_path, asset_name)?;
    Ok(ApplyResult {
        message: result.message,
        will_exit: result.will_exit,
        channel_label: attempt.label,
    })
}

struct LocalApply {
    message: String,
    will_exit: bool,
}

fn apply_package(package_path: &Path, asset_name: &str) -> Result<LocalApply> {
    #[cfg(target_os = "windows")]
    {
        let _ = asset_name;
        return apply_windows(package_path);
    }
    #[cfg(target_os = "macos")]
    {
        return apply_macos(package_path, asset_name);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (package_path, asset_name);
        bail!("当前平台暂不支持应用内安装")
    }
}

#[cfg(target_os = "windows")]
fn apply_windows(package_path: &Path) -> Result<LocalApply> {
    let lower = package_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower == "exe" {
        // Tauri NSIS：/S 静默安装；安装程序会处理替换，随后我们退出。
        Command::new(package_path)
            .arg("/S")
            .spawn()
            .context("无法启动安装程序")?;
        return Ok(LocalApply {
            message: "安装程序已启动，应用即将退出。安装完成后请重新打开 SUTAI。".into(),
            will_exit: true,
        });
    }
    // zip 便携包：解压并打开目录
    let dest = package_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("extracted");
    unzip_file(package_path, &dest)?;
    platform::open_path(&dest)?;
    Ok(LocalApply {
        message: "已下载并解压便携包，请在打开的文件夹中运行 sugt.exe。".into(),
        will_exit: false,
    })
}

#[cfg(target_os = "macos")]
fn apply_macos(package_path: &Path, _asset_name: &str) -> Result<LocalApply> {
    let dest = package_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("extracted");
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&dest);
    }
    std::fs::create_dir_all(&dest)?;
    unzip_file(package_path, &dest)?;

    let app_bundle = find_app_bundle(&dest)?;
    if let Some(target) = writable_install_target() {
        // 等本进程退出后再 ditto，避免覆盖正在运行的 bundle。
        let pid = std::process::id();
        let script = format!(
            r#"#!/bin/bash
set -e
while kill -0 {pid} 2>/dev/null; do sleep 0.4; done
sleep 0.8
ditto "{src}" "{dst}"
open "{dst}"
"#,
            pid = pid,
            src = app_bundle.display(),
            dst = target.display()
        );
        let script_path = package_path
            .parent()
            .unwrap_or(Path::new("/tmp"))
            .join("sugt-finish-update.sh");
        std::fs::write(&script_path, script)?;
        let _ = Command::new("chmod").args(["+x"]).arg(&script_path).status();
        Command::new("bash")
            .arg(&script_path)
            .spawn()
            .context("无法启动更新脚本")?;
        return Ok(LocalApply {
            message: format!(
                "已准备安装到 {}。应用即将退出，完成后会自动打开新版本。",
                target.display()
            ),
            will_exit: true,
        });
    }

    platform::open_path(app_bundle.parent().unwrap_or(dest.as_path()))?;
    Ok(LocalApply {
        message: format!(
            "已下载新版本到 {}。请将「{}」拖入「应用程序」后重新打开。",
            dest.display(),
            app_bundle
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("SUTAI.app")
        ),
        will_exit: false,
    })
}

#[cfg(target_os = "macos")]
fn find_app_bundle(dir: &Path) -> Result<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 4 {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("app") {
                return Some(path);
            }
            if path.is_dir() {
                if let Some(found) = walk(&path, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(dir, 0).ok_or_else(|| anyhow!("压缩包中未找到 .app"))
}

#[cfg(target_os = "macos")]
fn writable_install_target() -> Option<PathBuf> {
    // 若当前在 .app 内运行，尝试覆盖同一路径
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.as_path();
        for _ in 0..5 {
            if cur.extension().and_then(|e| e.to_str()) == Some("app") {
                if cur.starts_with("/Applications") {
                    return Some(cur.to_path_buf());
                }
                // 用户目录下的 .app 也可试写
                if std::fs::metadata(cur).map(|m| !m.permissions().readonly()).unwrap_or(false) {
                    return Some(cur.to_path_buf());
                }
                break;
            }
            cur = cur.parent()?;
        }
    }
    let apps = PathBuf::from("/Applications/SUTAI.app");
    if apps.exists() {
        // 探测是否可写
        let probe = apps.join(".sugt_write_probe");
        if std::fs::write(&probe, b"ok").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Some(apps);
        }
    } else {
        // 尝试创建（首次装到 Applications）
        let parent = Path::new("/Applications");
        let probe = parent.join(".sugt_write_probe");
        if std::fs::write(&probe, b"ok").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Some(apps);
        }
    }
    None
}

fn unzip_file(zip_path: &Path, dest: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("ditto")
            .args(["-x", "-k"])
            .arg(zip_path)
            .arg(dest)
            .status()
            .context("解压失败")?;
        if !status.success() {
            bail!("解压失败");
        }
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        // PowerShell Expand-Archive
        let status = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                    zip_path.display(),
                    dest.display()
                ),
            ])
            .status()
            .context("解压失败")?;
        if !status.success() {
            bail!("解压失败");
        }
        return Ok(());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (zip_path, dest);
        bail!("当前平台不支持解压")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_cmp_basic() {
        assert!(cmp_version("1.1.6", "1.1.7") < 0);
        assert!(cmp_version("v1.1.7", "1.1.7") == 0);
        assert!(cmp_version("1.2.0", "1.1.9") > 0);
    }
}
