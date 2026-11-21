use crate::store::catalog::{
    find_skill_source, load_mounted_meta, load_staged_meta, mount_flags, parse_skill_md,
    save_mounted_meta, save_staged_meta, SkillCatalogItem, StagedSkillRecord, MountedSkillRecord,
};
use crate::store::paths::{
    remove_dir_if_exists, replace_dir, validate_folder_name, validate_skill_id, StorePaths,
};
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountTarget {
    Claude,
    Codex,
    Both,
}

impl MountTarget {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("both").trim().to_lowercase().as_str() {
            "claude" => MountTarget::Claude,
            "codex" => MountTarget::Codex,
            _ => MountTarget::Both,
        }
    }
}

pub fn install_skill(store_paths: &StorePaths, skill_id: &str) -> Result<SkillCatalogItem> {
    validate_skill_id(skill_id)?;
    let catalog = crate::store::catalog::build_catalog(store_paths)?;
    let item = catalog
        .into_iter()
        .find(|entry| entry.id == skill_id)
        .ok_or_else(|| anyhow!("未找到技能：{}", skill_id))?;
    let source = find_skill_source(store_paths, &item.repo_id, &item.relative_path)?;
    let dest = store_paths.staged_skill_dir(skill_id);
    replace_dir(&source, &dest)?;

    let (name, _) = parse_skill_md(&source.join("SKILL.md")).unwrap_or((item.name.clone(), None));
    let mut staged = load_staged_meta(store_paths)?;
    staged.skills.insert(
        skill_id.to_string(),
        StagedSkillRecord {
            name,
            repo_id: item.repo_id.clone(),
            relative_path: item.relative_path.clone(),
            installed_at: now_secs(),
        },
    );
    save_staged_meta(store_paths, &staged)?;

    let mounted = load_mounted_meta(store_paths)?;
    let (mounted_claude, mounted_codex, mounted_any) = mount_flags(&mounted, skill_id);
    Ok(SkillCatalogItem {
        id: skill_id.to_string(),
        name: item.name,
        description: item.description,
        repo_id: item.repo_id,
        repo_label: item.repo_label,
        relative_path: item.relative_path,
        staged: true,
        mounted: mounted_any,
        mounted_claude,
        mounted_codex,
    })
}

pub fn uninstall_skill(store_paths: &StorePaths, skill_id: &str) -> Result<()> {
    validate_skill_id(skill_id)?;
    let mut staged = load_staged_meta(store_paths)?;
    staged.skills.remove(skill_id);
    save_staged_meta(store_paths, &staged)?;
    remove_dir_if_exists(&store_paths.staged_skill_dir(skill_id))?;
    unmount_skill(store_paths, skill_id, MountTarget::Both).ok();
    Ok(())
}

pub fn mount_skill(
    store_paths: &StorePaths,
    skill_id: &str,
    target: MountTarget,
) -> Result<SkillCatalogItem> {
    validate_skill_id(skill_id)?;
    let staged = load_staged_meta(store_paths)?;
    let record = staged
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未安装到暂存区：{}", skill_id))?;
    let source = store_paths.staged_skill_dir(skill_id);
    if !source.exists() {
        return Err(anyhow!("暂存目录不存在，请重新安装"));
    }

    let folder_name = mount_folder_name(skill_id, record)?;
    let mut mounted = load_mounted_meta(store_paths)?;
    let entry = mounted
        .skills
        .entry(skill_id.to_string())
        .or_insert_with(|| MountedSkillRecord {
            claude_folder: None,
            codex_folder: None,
            folder_name: None,
            mounted_at: None,
            agents: Default::default(),
            custom_paths: Default::default(),
        });

    if matches!(target, MountTarget::Claude | MountTarget::Both) {
        mount_to_dir(&source, &StorePaths::claude_skills_dir()?, &folder_name)?;
        entry.set_agent_mount("claude-code", &folder_name);
    }
    if matches!(target, MountTarget::Codex | MountTarget::Both) {
        mount_to_dir(&source, &StorePaths::codex_skills_dir()?, &folder_name)?;
        entry.set_agent_mount("codex", &folder_name);
    }
    entry.mounted_at = Some(now_secs());
    save_mounted_meta(store_paths, &mounted)?;

    let (mounted_claude, mounted_codex, mounted_any) = mount_flags(&mounted, skill_id);
    Ok(SkillCatalogItem {
        id: skill_id.to_string(),
        name: record.name.clone(),
        description: None,
        repo_id: record.repo_id.clone(),
        repo_label: record.repo_id.clone(),
        relative_path: record.relative_path.clone(),
        staged: true,
        mounted: mounted_any,
        mounted_claude,
        mounted_codex,
    })
}

pub fn unmount_skill(
    store_paths: &StorePaths,
    skill_id: &str,
    target: MountTarget,
) -> Result<()> {
    validate_skill_id(skill_id)?;
    let mut mounted = load_mounted_meta(store_paths)?;
    let record = mounted
        .skills
        .get_mut(skill_id)
        .ok_or_else(|| anyhow!("技能未挂载：{}", skill_id))?;

    // 先校验再动手：mounted.json 里的 folder 来自磁盘，不可信；任何一个非法就整体拒绝，
    // 不留下「删了一半」的状态。
    let want_claude = matches!(target, MountTarget::Claude | MountTarget::Both);
    let want_codex = matches!(target, MountTarget::Codex | MountTarget::Both);
    let claude_folder = if want_claude {
        record.claude_folder_name().map(str::to_string)
    } else {
        None
    };
    let codex_folder = if want_codex {
        record
            .codex_folder
            .clone()
            .or_else(|| record.agents.get("codex").cloned())
    } else {
        None
    };
    for folder in [&claude_folder, &codex_folder].into_iter().flatten() {
        validate_folder_name(folder)?;
    }

    if want_claude {
        if let Some(folder) = &claude_folder {
            remove_mounted_folder(&StorePaths::claude_skills_dir()?, folder)?;
        }
        record.clear_agent_mount("claude-code");
    }
    if want_codex {
        if let Some(folder) = &codex_folder {
            remove_mounted_folder(&StorePaths::codex_skills_dir()?, folder)?;
        }
        record.clear_agent_mount("codex");
    }

    if record.is_empty_mount() {
        mounted.skills.remove(skill_id);
    }
    save_mounted_meta(store_paths, &mounted)?;
    Ok(())
}

pub fn resolve_skill_path(
    store_paths: &StorePaths,
    skill_id: &str,
    staged: bool,
    client: Option<&str>,
) -> Result<PathBuf> {
    validate_skill_id(skill_id)?;
    if staged {
        let path = store_paths.staged_skill_dir(skill_id);
        if path.exists() {
            return Ok(path);
        }
        return Err(anyhow!("暂存技能不存在"));
    }
    let mounted = load_mounted_meta(store_paths)?;
    let record = mounted
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未挂载"))?;

    let use_codex = client.map(|c| c.eq_ignore_ascii_case("codex")).unwrap_or(false);
    if use_codex {
        if let Some(folder) = &record.codex_folder {
            validate_folder_name(folder)?;
            let path = StorePaths::codex_skills_dir()?.join(folder);
            if path.exists() {
                return Ok(path);
            }
        }
        return Err(anyhow!("Codex skills 目录中找不到该技能"));
    }

    if let Some(folder) = record.claude_folder_name() {
        validate_folder_name(folder)?;
        let path = StorePaths::claude_skills_dir()?.join(folder);
        if path.exists() {
            return Ok(path);
        }
    }
    Err(anyhow!("Claude skills 目录中找不到该技能"))
}

pub fn repo_skill_count(store_paths: &StorePaths, repo_id: &str) -> usize {
    crate::store::catalog::build_catalog(store_paths)
        .map(|items| items.iter().filter(|i| i.repo_id == repo_id).count())
        .unwrap_or(0)
}

/// 挂载写入的唯一出口：`folder_name` 必须是单一路径段，否则 `join` 会逃出 skills 目录，
/// 空串更会让 `replace_dir` 把整个 skills 目录当目标替换掉。
pub(crate) fn mount_to_dir(source: &PathBuf, skills_dir: &PathBuf, folder_name: &str) -> Result<()> {
    validate_folder_name(folder_name)?;
    std::fs::create_dir_all(skills_dir)?;
    replace_dir(source, &skills_dir.join(folder_name))
}

/// 卸载删除的唯一出口：与 [`mount_to_dir`] 对称，folder 来自 `mounted.json`，必须先校验。
pub(crate) fn remove_mounted_folder(skills_dir: &Path, folder: &str) -> Result<()> {
    validate_folder_name(folder)?;
    remove_dir_if_exists(&skills_dir.join(folder))
}

/// 挂载时使用的目录名：取 `relative_path` 末段；末段不是合法单段（空串，例如仓库根目录
/// 本身就是技能；或被篡改的 `..`）时回退到已校验的 `skill_id`，保证 `join` 不会越界。
pub(crate) fn mount_folder_name(skill_id: &str, record: &StagedSkillRecord) -> Result<String> {
    let last = record.relative_path.split('/').last().unwrap_or("");
    match validate_folder_name(last) {
        Ok(folder) => Ok(folder.to_string()),
        Err(_) => Ok(validate_skill_id(skill_id)?.to_string()),
    }
}

pub(crate) fn folder_name_from_record(record: &StagedSkillRecord) -> String {
    record
        .relative_path
        .split('/')
        .last()
        .unwrap_or(&record.name)
        .to_string()
}

pub(crate) fn now_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (PathBuf, StorePaths) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sugt-install-{tag}-{stamp}"));
        let store = root.join("store");
        let paths = StorePaths {
            root: store.clone(),
            skills_dir: store.join("skills"),
            cache_dir: store.join("cache"),
            repos_file: store.join("repos.toml"),
            settings_file: store.join("settings.toml"),
            staged_meta: store.join("staged.json"),
            mounted_meta: store.join("mounted.json"),
        };
        paths.ensure_dirs().unwrap();
        (root, paths)
    }

    #[test]
    fn uninstall_rejects_traversal_ids_and_keeps_outside_files() {
        let (root, paths) = temp_store("uninstall");
        // skills_dir 之外的“受害者”目录与文件
        let victim = paths.root.join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("keep.txt"), "keep").unwrap();
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "keep").unwrap();

        for evil in ["../victim", "..", "../../outside", "a/../../victim", "..\\victim", ""] {
            assert!(
                uninstall_skill(&paths, evil).is_err(),
                "越界 id 应被拒绝：{evil:?}"
            );
        }
        assert!(victim.join("keep.txt").exists(), "staging 外文件不应被删除");
        assert!(outside.join("keep.txt").exists());

        let abs = victim.display().to_string();
        assert!(uninstall_skill(&paths, &abs).is_err());
        assert!(victim.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uninstall_legit_id_still_removes_staged_dir() {
        let (root, paths) = temp_store("legit");
        let dir = paths.staged_skill_dir("local__demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "x").unwrap();
        uninstall_skill(&paths, "local__demo").unwrap();
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_mount_unmount_reject_bad_ids() {
        let (root, paths) = temp_store("resolve");
        std::fs::create_dir_all(paths.root.join("victim")).unwrap();
        assert!(resolve_skill_path(&paths, "../victim", true, None).is_err());
        assert!(resolve_skill_path(&paths, "a/b", false, None).is_err());
        assert!(mount_skill(&paths, "../victim", MountTarget::Both).is_err());
        assert!(unmount_skill(&paths, "..", MountTarget::Both).is_err());
        assert!(install_skill(&paths, "../victim").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn staged_record(relative: &str) -> StagedSkillRecord {
        StagedSkillRecord {
            name: "demo".into(),
            repo_id: "git-import".into(),
            relative_path: relative.into(),
            installed_at: "0".into(),
        }
    }

    #[test]
    fn mount_folder_name_falls_back_to_skill_id_for_unsafe_segments() {
        assert_eq!(mount_folder_name("r__a--b", &staged_record("a/b")).unwrap(), "b");
        // 仓库根目录即技能：relative_path 为空，不能得到 ""（join 后等于 skills 目录本身）
        assert_eq!(
            mount_folder_name("git__o-r__", &staged_record("")).unwrap(),
            "git__o-r__"
        );
        // 被篡改的 staged.json：.. / 结尾斜杠 / 反斜杠 → 回退 skill_id，绝不返回危险段
        for evil in ["..", "a/..", "a/", "a\\b", "../.."] {
            assert_eq!(
                mount_folder_name("sid", &staged_record(evil)).unwrap(),
                "sid",
                "{evil:?}"
            );
        }
        assert!(mount_folder_name("../x", &staged_record("..")).is_err());
    }

    #[test]
    fn mount_to_dir_and_remove_reject_unsafe_folder_and_keep_skills_dir() {
        let (root, _paths) = temp_store("folder");
        let skills = root.join("agent-skills");
        let keep = skills.join("other-skill");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("SKILL.md"), "keep").unwrap();
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "keep").unwrap();
        let source = root.join("src-skill");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "new").unwrap();

        for evil in ["", ".", "..", "../outside", "a/b", "a\\b", "/etc"] {
            assert!(mount_to_dir(&source, &skills, evil).is_err(), "mount 应拒绝 {evil:?}");
            assert!(
                remove_mounted_folder(&skills, evil).is_err(),
                "remove 应拒绝 {evil:?}"
            );
        }
        assert_eq!(std::fs::read_to_string(keep.join("SKILL.md")).unwrap(), "keep");
        assert!(outside.join("keep.txt").exists(), "skills 目录外文件不应被动");

        // 合法 folder 仍可挂载 / 卸载，且只动自己那一份
        mount_to_dir(&source, &skills, "demo").unwrap();
        assert!(skills.join("demo").join("SKILL.md").exists());
        remove_mounted_folder(&skills, "demo").unwrap();
        assert!(!skills.join("demo").exists());
        assert!(keep.join("SKILL.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tampered_mounted_json_folder_is_rejected_before_any_delete() {
        let (root, paths) = temp_store("tamper");
        for evil in ["..", "", "../x", "a/b"] {
            let raw = format!(
                r#"{{"skills":{{"demo":{{"claude_folder":{evil:?},"codex_folder":"ok","agents":{{}}}}}}}}"#
            );
            std::fs::write(&paths.mounted_meta, raw).unwrap();
            // 校验发生在解析 skills 目录与任何删除之前：不触碰真实 ~/.claude / ~/.codex
            assert!(
                unmount_skill(&paths, "demo", MountTarget::Both).is_err(),
                "claude_folder={evil:?} 应拒绝卸载"
            );
            assert!(
                resolve_skill_path(&paths, "demo", false, None).is_err(),
                "claude_folder={evil:?} 应拒绝打开"
            );
        }
        let raw = r#"{"skills":{"demo":{"codex_folder":"..","agents":{}}}}"#;
        std::fs::write(&paths.mounted_meta, raw).unwrap();
        assert!(unmount_skill(&paths, "demo", MountTarget::Codex).is_err());
        assert!(resolve_skill_path(&paths, "demo", false, Some("codex")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
