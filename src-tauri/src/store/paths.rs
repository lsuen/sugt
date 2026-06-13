use crate::config::AppPaths;
use std::path::{Path, PathBuf};

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
        let home = Self::user_home()?;
        Ok(home.join(".claude").join("skills"))
    }

    pub fn codex_skills_dir() -> anyhow::Result<PathBuf> {
        let home = Self::user_home()?;
        Ok(home.join(".agents").join("skills"))
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
