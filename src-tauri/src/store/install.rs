use crate::store::catalog::{
    find_skill_source, load_mounted_meta, load_staged_meta, mount_flags, parse_skill_md,
    save_mounted_meta, save_staged_meta, SkillCatalogItem, StagedSkillRecord, MountedSkillRecord,
};
use crate::store::paths::{copy_dir_all, remove_dir_if_exists, StorePaths};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
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
    let catalog = crate::store::catalog::build_catalog(store_paths)?;
    let item = catalog
        .into_iter()
        .find(|entry| entry.id == skill_id)
        .ok_or_else(|| anyhow!("未找到技能：{}", skill_id))?;
    let source = find_skill_source(store_paths, &item.repo_id, &item.relative_path)?;
    let dest = store_paths.staged_skill_dir(skill_id);
    remove_dir_if_exists(&dest)?;
    copy_dir_all(&source, &dest)?;

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
    let staged = load_staged_meta(store_paths)?;
    let record = staged
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未安装到暂存区：{}", skill_id))?;
    let source = store_paths.staged_skill_dir(skill_id);
    if !source.exists() {
        return Err(anyhow!("暂存目录不存在，请重新安装"));
    }

    let folder_name = folder_name_from_record(record);
    let mut mounted = load_mounted_meta(store_paths)?;
    let entry = mounted
        .skills
        .entry(skill_id.to_string())
        .or_insert_with(|| MountedSkillRecord {
            claude_folder: None,
            codex_folder: None,
            folder_name: None,
            mounted_at: None,
        });

    if matches!(target, MountTarget::Claude | MountTarget::Both) {
        mount_to_dir(&source, &StorePaths::claude_skills_dir()?, &folder_name)?;
        entry.claude_folder = Some(folder_name.clone());
        entry.folder_name = Some(folder_name.clone());
    }
    if matches!(target, MountTarget::Codex | MountTarget::Both) {
        mount_to_dir(&source, &StorePaths::codex_skills_dir()?, &folder_name)?;
        entry.codex_folder = Some(folder_name);
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
    let mut mounted = load_mounted_meta(store_paths)?;
    let record = mounted
        .skills
        .get_mut(skill_id)
        .ok_or_else(|| anyhow!("技能未挂载：{}", skill_id))?;

    if matches!(target, MountTarget::Claude | MountTarget::Both) {
        if let Some(folder) = record.claude_folder_name() {
            let claude_dir = StorePaths::claude_skills_dir()?;
            remove_dir_if_exists(&claude_dir.join(folder))?;
        }
        record.claude_folder = None;
        record.folder_name = None;
    }
    if matches!(target, MountTarget::Codex | MountTarget::Both) {
        if let Some(folder) = &record.codex_folder {
            let codex_dir = StorePaths::codex_skills_dir()?;
            remove_dir_if_exists(&codex_dir.join(folder))?;
        }
        record.codex_folder = None;
    }

    if !record.is_mounted_claude() && record.codex_folder.is_none() {
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
            let path = StorePaths::codex_skills_dir()?.join(folder);
            if path.exists() {
                return Ok(path);
            }
        }
        return Err(anyhow!("Codex skills 目录中找不到该技能"));
    }

    if let Some(folder) = record.claude_folder_name() {
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

fn mount_to_dir(source: &PathBuf, skills_dir: &PathBuf, folder_name: &str) -> Result<()> {
    std::fs::create_dir_all(skills_dir)?;
    let dest = skills_dir.join(folder_name);
    remove_dir_if_exists(&dest)?;
    copy_dir_all(source, &dest)?;
    Ok(())
}

fn folder_name_from_record(record: &StagedSkillRecord) -> String {
    record
        .relative_path
        .split('/')
        .last()
        .unwrap_or(&record.name)
        .to_string()
}

fn now_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
