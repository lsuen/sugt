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

/// 校验 skill_id 必须是单一合法路径段：非空、不是 `.` / `..`、不含分隔符、盘符冒号、
/// NUL 或控制字符，也不是绝对路径。越界一律 Err，避免 `skills_dir.join(id)` 逃出暂存区。
pub fn validate_skill_id(skill_id: &str) -> anyhow::Result<&str> {
    validate_single_segment(skill_id, "技能 ID")
}

/// 校验挂载目录名（`mounted.json` 的 folder 字段 / `relative_path` 末段）：规则与
/// [`validate_skill_id`] 完全一致，是 `skills_dir.join(folder)` 之前的唯一闸门。
/// 空串会让 `join` 退化成 skills 目录本身，`..` 会逃出 skills 目录，都必须拒绝。
pub fn validate_folder_name(folder: &str) -> anyhow::Result<&str> {
    validate_single_segment(folder, "挂载目录名")
}

pub fn validate_single_segment<'a>(raw: &'a str, what: &str) -> anyhow::Result<&'a str> {
    let bad = raw.is_empty()
        || raw.trim() != raw
        // 全点号（`.` / `..` / `...`）：Windows 会把尾部点号当空，统一拒绝。
        || raw.chars().all(|c| c == '.')
        || raw
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control());
    if bad {
        return Err(anyhow::anyhow!("非法{what}：{raw:?}"));
    }
    let mut comps = Path::new(raw).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(raw),
        _ => Err(anyhow::anyhow!("非法{what}：{raw:?}")),
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
    fn validate_skill_id_accepts_legit_and_rejects_traversal() {
        for ok in ["repo__a--b", "local__weekly", "imported__周报助手", "a..b", "x.y"] {
            assert!(validate_skill_id(ok).is_ok(), "应放行：{ok}");
        }
        for bad in [
            "", " ", ".", "..", "../x", "a/../b", "a/b", "a\\b", "..\\x", "/etc/passwd",
            "C:\\Windows", "C:", "a\0b", "a\nb", " x", "...",
        ] {
            assert!(validate_skill_id(bad).is_err(), "应拒绝：{bad:?}");
        }
    }

    #[test]
    fn validate_folder_name_shares_single_segment_rules() {
        for ok in ["demo", "my-skill", "周报助手", "a..b", "v1.2"] {
            assert!(validate_folder_name(ok).is_ok(), "应放行：{ok}");
        }
        for bad in [
            "", ".", "..", "...", "../x", "a/b", "a\\b", "/etc", "C:\\x", "C:", " x", "x ", "a\0b",
        ] {
            assert!(validate_folder_name(bad).is_err(), "应拒绝：{bad:?}");
        }
    }

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
