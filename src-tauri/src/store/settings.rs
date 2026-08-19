use crate::store::paths::StorePaths;
use crate::store::process::hidden_command;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSettings {
    pub editor_command: String,
    /// GitHub 克隆代理前缀，如 https://ghfast.top/ ，拼接在 https://github.com/... 之前
    #[serde(default)]
    pub github_proxy_prefix: String,
}

impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            editor_command: String::new(),
            github_proxy_prefix: String::new(),
        }
    }
}

pub const GITHUB_TEST_REPO_URL: &str = "https://github.com/lsuen/testconnect";

/// 将代理前缀拼到 GitHub HTTPS URL 前。前缀为空则原样返回。
pub fn apply_github_proxy(prefix: &str, github_url: &str) -> String {
    let prefix = prefix.trim();
    let url = github_url.trim();
    if prefix.is_empty() {
        return url.to_string();
    }
    let normalized = prefix.trim_end_matches('/');
    if url.starts_with(normalized) {
        return url.to_string();
    }
    format!("{}/{}", normalized, url.trim_start_matches('/'))
}

pub fn test_github_proxy(prefix: &str) -> Result<String> {
    if !crate::store::repos::git_available() {
        return Err(anyhow::anyhow!("{}", crate::store::repos::git_missing_hint()));
    }
    let url = apply_github_proxy(prefix, GITHUB_TEST_REPO_URL);
    let output = hidden_command("git")
        .args(["ls-remote", "--heads", &url])
        .output()
        .context("无法执行 git ls-remote")?;
    if output.status.success() {
        Ok(format!("连接成功（{}）", url))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        Err(anyhow::anyhow!(
            "无法访问测试仓库{}{}",
            url,
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", detail)
            }
        ))
    }
}

pub fn load_settings(store_paths: &StorePaths) -> Result<StoreSettings> {
    if !store_paths.settings_file.exists() {
        let settings = StoreSettings::default();
        save_settings(store_paths, &settings)?;
        return Ok(settings);
    }
    let raw = std::fs::read_to_string(&store_paths.settings_file)
        .with_context(|| format!("无法读取 {}", store_paths.settings_file.display()))?;
    let settings: StoreSettings = toml::from_str(&raw).unwrap_or_default();
    Ok(settings)
}

pub fn save_settings(store_paths: &StorePaths, settings: &StoreSettings) -> Result<()> {
    store_paths.ensure_dirs()?;
    let serialized = toml::to_string_pretty(settings).context("商店设置序列化失败")?;
    std::fs::write(&store_paths.settings_file, serialized)
        .with_context(|| format!("无法写入 {}", store_paths.settings_file.display()))?;
    Ok(())
}

pub fn open_with_editor(editor_command: &str, path: &std::path::Path) -> Result<()> {
    if editor_command.trim().is_empty() {
        #[cfg(windows)]
        {
            hidden_command("explorer").arg(path).spawn()?;
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            hidden_command("open").arg(path).spawn()?;
            return Ok(());
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            hidden_command("xdg-open").arg(path).spawn()?;
            return Ok(());
        }
    }

    let mut parts = editor_command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("编辑器命令无效"))?;
    let args: Vec<&str> = parts.collect();
    let mut command = hidden_command(program);
    for arg in args {
        command.arg(arg);
    }
    command.arg(path);
    command.spawn().context("无法启动编辑器")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_proxy_prefix_joins_url() {
        assert_eq!(
            apply_github_proxy("https://ghfast.top/", "https://github.com/lsuen/testconnect"),
            "https://ghfast.top/https://github.com/lsuen/testconnect"
        );
        assert_eq!(
            apply_github_proxy("", "https://github.com/foo/bar"),
            "https://github.com/foo/bar"
        );
    }
}
