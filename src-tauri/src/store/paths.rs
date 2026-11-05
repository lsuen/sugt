use crate::config::AppPaths;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct StorePaths {
    pub root: PathBuf,
    pub skills_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub repos_file: PathBuf,
    pub settings_file: PathBuf,
    pub staged_meta: PathBuf,
    pub mounted_meta: PathBuf,
}

impl StorePaths {
    pub fn from_app(paths: &AppPaths) -> Self {
        let root = paths.config_dir.join("store");
        Self {
            root: root.clone(),
            skills_dir: root.join("skills"),
            cache_dir: root.join("cache"),
            repos_file: root.join("repos.toml"),
            settings_file: root.join("settings.toml"),
            staged_meta: root.join("staged.json"),
            mounted_meta: root.join("mounted.json"),
        }
    }

    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.skills_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        Ok(())
    }

    pub fn repo_cache_dir(&self, repo_id: &str) -> PathBuf {
        self.cache_dir.join(repo_id)
    }

    pub fn staged_skill_dir(&self, skill_id: &str) -> PathBuf {
        self.skills_dir.join(skill_id)
    }

    pub fn claude_skills_dir() -> anyhow::Result<PathBuf> {
        crate::agent_adapters::canonical_skill_dir("claude-code")
    }

    pub fn codex_skills_dir() -> anyhow::Result<PathBuf> {
        crate::agent_adapters::canonical_skill_dir("codex")
    }

    pub fn claude_plugins_dir() -> anyhow::Result<PathBuf> {
        let home = Self::user_home()?;
        Ok(home.join(".claude").join("plugins"))
    }

    fn user_home() -> anyhow::Result<PathBuf> {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .ok_or_else(|| anyhow::anyhow!("无法解析用户主目录"))
    }
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub fn remove_dir_if_exists(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// 原子替换目录：先完整拷到临时目录，再切换；失败时回滚旧目录。
///
/// 替代「先 remove_dir_all 再 copy」，避免拷贝中途失败把用户已有技能删光。
pub fn replace_dir(source: &Path, dest: &Path) -> anyhow::Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("目标目录没有父目录：{}", dest.display()))?;
    std::fs::create_dir_all(parent)?;
    let name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("dir");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = parent.join(format!(".{name}.sugt-stage-{stamp}"));
    let backup = parent.join(format!(".{name}.sugt-old-{stamp}"));

    remove_dir_if_exists(&staging)?;
    copy_dir_all(source, &staging)?;

    let had_old = dest.exists();
    if had_old {
        remove_dir_if_exists(&backup)?;
        std::fs::rename(dest, &backup)?;
    }
    if let Err(err) = std::fs::rename(&staging, dest) {
        if had_old {
            let _ = std::fs::rename(&backup, dest);
        }
        let _ = remove_dir_if_exists(&staging);
        return Err(err.into());
    }
    if had_old {
        let _ = remove_dir_if_exists(&backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_dir_swaps_content_without_leaving_temp_dirs() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sugt-replace-{stamp}"));
        let source = root.join("source");
        let dest = root.join("skills").join("my-skill");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(source.join("fresh.txt"), "fresh").unwrap();
        std::fs::write(dest.join("old.txt"), "old").unwrap();

        replace_dir(&source, &dest).unwrap();

        assert_eq!(std::fs::read_to_string(dest.join("fresh.txt")).unwrap(), "fresh");
        assert!(!dest.join("old.txt").exists(), "旧内容应被完整替换");
        let leftovers: Vec<String> = std::fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains("sugt-stage") || n.contains("sugt-old"))
            .collect();
        assert!(leftovers.is_empty(), "临时目录未清理：{leftovers:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
