use crate::store::catalog::{
    find_skill_source, load_mounted_meta, load_staged_meta, parse_skill_md,
    save_mounted_meta, save_staged_meta, SkillCatalogItem, StagedSkillRecord,
    MountedSkillRecord,
};
use crate::store::paths::{copy_dir_all, remove_dir_if_exists, StorePaths};
use anyhow::{anyhow, Result};
use std::time::{SystemTime, UNIX_EPOCH};

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
    Ok(SkillCatalogItem {
        id: skill_id.to_string(),
        name: item.name,
        description: item.description,
        repo_id: item.repo_id,
        repo_label: item.repo_label,
        relative_path: item.relative_path,
        staged: true,
        mounted: mounted.skills.contains_key(skill_id),
    })
}

pub fn uninstall_skill(store_paths: &StorePaths, skill_id: &str) -> Result<()> {
    let mut staged = load_staged_meta(store_paths)?;
    staged.skills.remove(skill_id);
    save_staged_meta(store_paths, &staged)?;
    remove_dir_if_exists(&store_paths.staged_skill_dir(skill_id))?;
    unmount_skill(store_paths, skill_id).ok();
    Ok(())
}

pub fn mount_skill(store_paths: &StorePaths, skill_id: &str) -> Result<SkillCatalogItem> {
    let staged = load_staged_meta(store_paths)?;
    let record = staged
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未安装到暂存区：{}", skill_id))?;
    let source = store_paths.staged_skill_dir(skill_id);
    if !source.exists() {
        return Err(anyhow!("暂存目录不存在，请重新安装"));
    }

    let folder_name = record
        .relative_path
        .split('/')
        .last()
        .unwrap_or(&record.name)
        .to_string();
    let claude_dir = StorePaths::claude_skills_dir()?;
    std::fs::create_dir_all(&claude_dir)?;
    let dest = claude_dir.join(&folder_name);
    remove_dir_if_exists(&dest)?;
    copy_dir_all(&source, &dest)?;

    let mut mounted = load_mounted_meta(store_paths)?;
    mounted.skills.insert(
        skill_id.to_string(),
        MountedSkillRecord {
            folder_name,
            mounted_at: now_secs(),
        },
    );
    save_mounted_meta(store_paths, &mounted)?;

    Ok(SkillCatalogItem {
        id: skill_id.to_string(),
        name: record.name.clone(),
        description: None,
        repo_id: record.repo_id.clone(),
        repo_label: record.repo_id.clone(),
        relative_path: record.relative_path.clone(),
        staged: true,
        mounted: true,
    })
}

pub fn unmount_skill(store_paths: &StorePaths, skill_id: &str) -> Result<()> {
    let mut mounted = load_mounted_meta(store_paths)?;
    let record = mounted
        .skills
        .remove(skill_id)
        .ok_or_else(|| anyhow!("技能未挂载到 Claude：{}", skill_id))?;
    let claude_dir = StorePaths::claude_skills_dir()?;
    remove_dir_if_exists(&claude_dir.join(&record.folder_name))?;
    save_mounted_meta(store_paths, &mounted)?;
    Ok(())
}

pub fn resolve_skill_path(store_paths: &StorePaths, skill_id: &str, staged: bool) -> Result<std::path::PathBuf> {
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
    let claude_dir = StorePaths::claude_skills_dir()?;
    let path = claude_dir.join(&record.folder_name);
    if path.exists() {
        Ok(path)
    } else {
        Err(anyhow!("Claude skills 目录中找不到该技能"))
    }
}

pub fn catalog_skill_count(store_paths: &StorePaths) -> Result<usize> {
    Ok(crate::store::catalog::build_catalog(store_paths)?.len())
}

pub fn repo_skill_count(store_paths: &StorePaths, repo_id: &str) -> usize {
    crate::store::catalog::build_catalog(store_paths)
        .map(|items| items.iter().filter(|i| i.repo_id == repo_id).count())
        .unwrap_or(0)
}

fn now_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
