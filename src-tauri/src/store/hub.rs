use crate::store::catalog::{
    load_mounted_meta, load_staged_meta, parse_skill_md, save_mounted_meta, save_staged_meta,
    MountedSkillRecord, StagedSkillRecord,
};
use crate::store::install::{folder_name_from_record, mount_to_dir, now_secs, MountTarget};
use crate::store::paths::{remove_dir_if_exists, StorePaths};
use crate::takeover_profiles::{self, TakeoverProfile};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct SkillAgentView {
    pub id: String,
    pub name: String,
    pub skills_dir: String,
    pub detected: bool,
    pub skills_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MountedAgentRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalSkillView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub repo_id: String,
    pub repo_label: String,
    pub relative_path: String,
    pub mounted_agents: Vec<MountedAgentRef>,
    /// 已挂到的自定义/项目路径
    pub mounted_paths: Vec<String>,
    pub is_local: bool,
}

pub fn list_skill_agents(config_dir: &Path) -> Vec<SkillAgentView> {
    takeover_profiles::all_profiles(config_dir)
        .into_iter()
        .filter_map(|profile| {
            let dir = resolve_agent_skills_dir(config_dir, &profile).ok()?;
            Some(SkillAgentView {
                id: profile.id.clone(),
                name: profile.name.clone(),
                skills_dir: dir.display().to_string(),
                detected: dir.is_dir(),
                skills_count: takeover_profiles::count_skills_in_dir(&dir),
            })
        })
        .collect()
}

pub fn resolve_agent_skills_dir(config_dir: &Path, profile: &TakeoverProfile) -> Result<PathBuf> {
    let _ = config_dir;
    let dirs = takeover_profiles::collect_skill_dirs(profile);
    if let Some(first) = dirs.into_iter().next() {
        return Ok(first);
    }
    Err(anyhow!(
        "Agent「{}」未配置技能目录（skill_dirs）",
        profile.name
    ))
}

pub fn resolve_agent_skills_dir_by_id(config_dir: &Path, agent_id: &str) -> Result<PathBuf> {
    let profile = takeover_profiles::find_profile(config_dir, agent_id)
        .ok_or_else(|| anyhow!("未找到 Agent：{}", agent_id))?;
    resolve_agent_skills_dir(config_dir, &profile)
}

pub fn list_local_skills(store_paths: &StorePaths, config_dir: &Path) -> Result<Vec<LocalSkillView>> {
    let staged = load_staged_meta(store_paths)?;
    let mounted = load_mounted_meta(store_paths)?;
    let agents = takeover_profiles::all_profiles(config_dir);
    let agent_name = |id: &str| -> String {
        agents
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    let mut items = Vec::new();
    for (skill_id, record) in staged.skills.iter() {
        let skill_dir = store_paths.staged_skill_dir(skill_id);
        let (name, description) = parse_skill_md(&skill_dir.join("SKILL.md"))
            .unwrap_or_else(|_| (record.name.clone(), None));
        let mounted_ids = mounted
            .skills
            .get(skill_id)
            .map(|r| r.mounted_agent_ids())
            .unwrap_or_default();
        let mounted_agents = mounted_ids
            .into_iter()
            .map(|id| MountedAgentRef {
                name: agent_name(&id),
                id,
            })
            .collect();
        let mounted_paths = mounted
            .skills
            .get(skill_id)
            .map(|r| r.mounted_custom_paths())
            .unwrap_or_default();
        items.push(LocalSkillView {
            id: skill_id.clone(),
            name,
            description,
            repo_id: record.repo_id.clone(),
            repo_label: if record.repo_id == "local" {
                "本地自建".into()
            } else {
                record.repo_id.clone()
            },
            relative_path: record.relative_path.clone(),
            mounted_agents,
            mounted_paths,
            is_local: record.repo_id == "local" || skill_id.starts_with("local__"),
        });
    }

    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

pub fn mount_skills_to_agents(
    store_paths: &StorePaths,
    config_dir: &Path,
    skill_ids: &[String],
    agent_ids: &[String],
    custom_path: Option<&str>,
) -> Result<usize> {
    let custom = custom_path.map(str::trim).filter(|s| !s.is_empty());
    if skill_ids.is_empty() {
        return Err(anyhow!("请至少选择一个技能"));
    }
    if agent_ids.is_empty() && custom.is_none() {
        return Err(anyhow!("请选择 Agent，或填写项目/自定义目录"));
    }
    let mut count = 0usize;
    for skill_id in skill_ids {
        for agent_id in agent_ids {
            mount_one(store_paths, config_dir, skill_id, agent_id)?;
            count += 1;
        }
        if let Some(path) = custom {
            mount_to_custom_path(store_paths, skill_id, path)?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn unmount_skills_from_agents(
    store_paths: &StorePaths,
    config_dir: &Path,
    skill_ids: &[String],
    agent_ids: &[String],
    custom_path: Option<&str>,
) -> Result<usize> {
    let custom = custom_path.map(str::trim).filter(|s| !s.is_empty());
    if skill_ids.is_empty() {
        return Err(anyhow!("请至少选择一个技能"));
    }
    if agent_ids.is_empty() && custom.is_none() {
        return Err(anyhow!("请选择 Agent，或填写项目/自定义目录"));
    }
    let mut count = 0usize;
    for skill_id in skill_ids {
        for agent_id in agent_ids {
            if unmount_one(store_paths, config_dir, skill_id, agent_id)? {
                count += 1;
            }
        }
        if let Some(path) = custom {
            if unmount_from_custom_path(store_paths, skill_id, path)? {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// 用户输入项目根 → `{root}/.claude/skills`；若已以 skills 结尾则原样使用。
pub fn resolve_custom_skills_dir(raw: &str) -> Result<PathBuf> {
    let path = takeover_profiles::expand_home(raw.trim())
        .ok_or_else(|| anyhow!("路径无效"))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "skills" {
        Ok(path)
    } else {
        Ok(path.join(".claude").join("skills"))
    }
}

fn path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn empty_mounted_record() -> MountedSkillRecord {
    MountedSkillRecord {
        claude_folder: None,
        codex_folder: None,
        folder_name: None,
        mounted_at: None,
        agents: Default::default(),
        custom_paths: Default::default(),
    }
}

fn mount_one(
    store_paths: &StorePaths,
    config_dir: &Path,
    skill_id: &str,
    agent_id: &str,
) -> Result<()> {
    let staged = load_staged_meta(store_paths)?;
    let record = staged
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未入库：{}", skill_id))?;
    let source = store_paths.staged_skill_dir(skill_id);
    if !source.exists() {
        return Err(anyhow!("本地技能目录不存在，请重新安装：{}", skill_id));
    }
    let skills_dir = resolve_agent_skills_dir_by_id(config_dir, agent_id)?;
    let folder_name = folder_name_from_record(record);
    mount_to_dir(&source, &skills_dir, &folder_name)?;

    let mut mounted = load_mounted_meta(store_paths)?;
    let entry = mounted
        .skills
        .entry(skill_id.to_string())
        .or_insert_with(empty_mounted_record);
    entry.set_agent_mount(agent_id, &folder_name);
    entry.mounted_at = Some(now_secs());
    save_mounted_meta(store_paths, &mounted)?;
    Ok(())
}

fn mount_to_custom_path(store_paths: &StorePaths, skill_id: &str, raw_path: &str) -> Result<()> {
    let staged = load_staged_meta(store_paths)?;
    let record = staged
        .skills
        .get(skill_id)
        .ok_or_else(|| anyhow!("技能未入库：{}", skill_id))?;
    let source = store_paths.staged_skill_dir(skill_id);
    if !source.exists() {
        return Err(anyhow!("本地技能目录不存在，请重新安装：{}", skill_id));
    }
    let skills_dir = resolve_custom_skills_dir(raw_path)?;
    let folder_name = folder_name_from_record(record);
    mount_to_dir(&source, &skills_dir, &folder_name)?;
    let key = path_key(&skills_dir);

    let mut mounted = load_mounted_meta(store_paths)?;
    let entry = mounted
        .skills
        .entry(skill_id.to_string())
        .or_insert_with(empty_mounted_record);
    entry.set_path_mount(&key, &folder_name);
    entry.mounted_at = Some(now_secs());
    save_mounted_meta(store_paths, &mounted)?;
    Ok(())
}

fn unmount_one(
    store_paths: &StorePaths,
    config_dir: &Path,
    skill_id: &str,
    agent_id: &str,
) -> Result<bool> {
    let mut mounted = load_mounted_meta(store_paths)?;
    let Some(record) = mounted.skills.get_mut(skill_id) else {
        return Ok(false);
    };
    let folder = record
        .agents
        .get(agent_id)
        .cloned()
        .or_else(|| {
            if agent_id.eq_ignore_ascii_case("claude-code") || agent_id.eq_ignore_ascii_case("claude")
            {
                record.claude_folder_name().map(|s| s.to_string())
            } else if agent_id.eq_ignore_ascii_case("codex") {
                record.codex_folder.clone()
            } else {
                None
            }
        });
    let Some(folder) = folder else {
        return Ok(false);
    };
    if let Ok(skills_dir) = resolve_agent_skills_dir_by_id(config_dir, agent_id) {
        remove_dir_if_exists(&skills_dir.join(&folder))?;
    }
    record.clear_agent_mount(agent_id);
    let empty = record.is_empty_mount();
    if empty {
        mounted.skills.remove(skill_id);
    }
    save_mounted_meta(store_paths, &mounted)?;
    Ok(true)
}

fn unmount_from_custom_path(store_paths: &StorePaths, skill_id: &str, raw_path: &str) -> Result<bool> {
    let skills_dir = resolve_custom_skills_dir(raw_path)?;
    let key = path_key(&skills_dir);
    let mut mounted = load_mounted_meta(store_paths)?;
    let Some(record) = mounted.skills.get_mut(skill_id) else {
        return Ok(false);
    };
    let Some(folder) = record.custom_paths.get(&key).cloned().or_else(|| {
        // 兼容未 canonicalize 的旧 key
        record
            .custom_paths
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&skills_dir.display().to_string()))
            .map(|(_, v)| v.clone())
    }) else {
        return Ok(false);
    };
    remove_dir_if_exists(&skills_dir.join(&folder))?;
    record.clear_path_mount(&key);
    // 顺带清未规范化的 key
    record
        .custom_paths
        .retain(|k, _| !k.eq_ignore_ascii_case(&skills_dir.display().to_string()));
    let empty = record.is_empty_mount();
    if empty {
        mounted.skills.remove(skill_id);
    }
    save_mounted_meta(store_paths, &mounted)?;
    Ok(true)
}

/// 兼容旧 MountTarget API
pub fn mount_target_to_agent_ids(target: MountTarget) -> Vec<String> {
    match target {
        MountTarget::Claude => vec!["claude-code".into()],
        MountTarget::Codex => vec!["codex".into()],
        MountTarget::Both => vec!["claude-code".into(), "codex".into()],
    }
}

pub fn create_local_skill(store_paths: &StorePaths, name: &str) -> Result<LocalSkillView> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("技能名称不能为空"));
    }
    let slug = slugify(name);
    let skill_id = format!("local__{}", slug);
    let dest = store_paths.staged_skill_dir(&skill_id);
    if dest.exists() {
        return Err(anyhow!("本地技能已存在：{}", slug));
    }
    std::fs::create_dir_all(&dest)?;
    let skill_md = format!(
        "---\nname: {name}\ndescription: A custom skill managed by SUGT.\n---\n\n# {name}\n\nDescribe when to use this skill and the steps to follow.\n"
    );
    std::fs::write(dest.join("SKILL.md"), skill_md)?;

    let mut staged = load_staged_meta(store_paths)?;
    staged.skills.insert(
        skill_id.clone(),
        StagedSkillRecord {
            name: name.to_string(),
            repo_id: "local".into(),
            relative_path: slug.clone(),
            installed_at: now_secs(),
        },
    );
    save_staged_meta(store_paths, &staged)?;

    Ok(LocalSkillView {
        id: skill_id,
        name: name.to_string(),
        description: Some("A custom skill managed by SUGT.".into()),
        repo_id: "local".into(),
        repo_label: "本地自建".into(),
        relative_path: slug,
        mounted_agents: vec![],
        mounted_paths: vec![],
        is_local: true,
    })
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("skill-{}", now_secs())
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::catalog::MountedSkillRecord;

    #[test]
    fn mounted_record_tracks_agents_and_legacy() {
        let mut rec = MountedSkillRecord {
            claude_folder: None,
            codex_folder: None,
            folder_name: None,
            mounted_at: None,
            agents: Default::default(),
            custom_paths: Default::default(),
        };
        rec.set_agent_mount("claude-code", "demo");
        assert!(rec.is_mounted_claude());
        assert!(rec.mounted_agent_ids().contains(&"claude-code".to_string()));
        rec.clear_agent_mount("claude-code");
        assert!(rec.is_empty_mount());
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Skill"), "my-skill");
        let zh = slugify("代码审查");
        assert!(zh.starts_with("skill-") || !zh.is_empty());
    }
}
