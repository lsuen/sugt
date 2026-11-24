use crate::store::catalog::{
    load_mounted_meta, load_staged_meta, parse_skill_meta, save_mounted_meta, save_staged_meta,
    MountedSkillRecord, StagedSkillRecord,
};
use crate::store::install::{
    mount_folder_name, mount_to_dir, now_secs, remove_mounted_folder,
};
use crate::store::paths::{validate_folder_name, validate_skill_id, StorePaths};
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
    /// SKILL.md frontmatter 里声明的触发词（未声明则为空）
    pub triggers: Vec<String>,
    /// 规则化优化建议，LLM 不可用时仍可展示
    pub suggestions: Vec<String>,
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
    // 目录尚不存在时仍返回 adapter 的 canonical 路径，交由前端显示
    // detected=false，而不是在这里把整个 Agent 静默丢弃。
    crate::agent_adapters::canonical_skill_dir(&profile.id)
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
        let meta = parse_skill_meta(&skill_dir.join("SKILL.md")).unwrap_or_else(|_| {
            crate::store::catalog::SkillMeta {
                name: record.name.clone(),
                description: None,
                triggers: Vec::new(),
                suggestions: vec!["SKILL.md 缺失或不可读，建议补一份标准技能说明".to_string()],
            }
        });
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
            name: meta.name,
            description: meta.description,
            triggers: meta.triggers,
            suggestions: meta.suggestions,
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
    validate_skill_id(skill_id)?;
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
    let folder_name = mount_folder_name(skill_id, record)?;
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
    validate_skill_id(skill_id)?;
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
    let folder_name = mount_folder_name(skill_id, record)?;
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
    validate_skill_id(skill_id)?;
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
    // mounted.json 的 folder 来自磁盘，删除前必须是单一路径段。
    validate_folder_name(&folder)?;
    if let Ok(skills_dir) = resolve_agent_skills_dir_by_id(config_dir, agent_id) {
        remove_mounted_folder(&skills_dir, &folder)?;
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
    validate_skill_id(skill_id)?;
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
    remove_mounted_folder(&skills_dir, &folder)?;
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
    // 模板自带 frontmatter / 触发词 / 分节，新建技能不用再从零猜格式。
    let skill_md = format!(
        "---\nname: {name}\ndescription: 说明这个技能做什么、什么时候用（建议写清适用场景与触发条件）。\ntriggers: \n---\n\n# {name}\n\n## 使用场景\n\n在这里写清什么情况下应该调用本技能。\n\n## 步骤\n\n1. 第一步\n2. 第二步\n\n## 示例\n\n在这里给一个最小可用示例。\n"
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
        description: Some("说明这个技能做什么、什么时候用（建议写清适用场景与触发条件）。".into()),
        repo_id: "local".into(),
        repo_label: "本地自建".into(),
        relative_path: slug,
        mounted_agents: vec![],
        mounted_paths: vec![],
        is_local: true,
        triggers: vec![],
        suggestions: vec!["triggers 为空：填入用户可能说出的词，Agent 更容易命中本技能".into()],
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

    fn temp_store(tag: &str) -> (PathBuf, StorePaths) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sugt-hub-{tag}-{stamp}"));
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
    fn custom_path_mount_of_root_skill_does_not_replace_skills_dir() {
        let (root, paths) = temp_store("rootskill");
        // 仓库根目录即技能：relative_path 为空
        let sid = "git__o-r__";
        let src = paths.staged_skill_dir(sid);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "root skill").unwrap();
        std::fs::write(
            &paths.staged_meta,
            format!(
                r#"{{"skills":{{"{sid}":{{"name":"r","repo_id":"git-import","relative_path":"","installed_at":"0"}}}}}}"#
            ),
        )
        .unwrap();
        let project = root.join("project").join("skills");
        let neighbour = project.join("neighbour");
        std::fs::create_dir_all(&neighbour).unwrap();
        std::fs::write(neighbour.join("SKILL.md"), "keep").unwrap();

        mount_to_custom_path(&paths, sid, project.to_str().unwrap()).unwrap();
        assert!(neighbour.join("SKILL.md").exists(), "已有技能不应被整目录替换");
        assert!(project.join(sid).join("SKILL.md").exists(), "回退为 skill_id 目录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unmount_rejects_tampered_folder_and_keeps_files() {
        let (root, paths) = temp_store("tamper");
        let project = root.join("project").join("skills");
        let neighbour = project.join("neighbour");
        std::fs::create_dir_all(&neighbour).unwrap();
        std::fs::write(neighbour.join("SKILL.md"), "keep").unwrap();
        let key = path_key(&project);

        for evil in ["..", "", "../outside", "a/b"] {
            let raw = serde_json::json!({
                "skills": { "demo": {
                    "custom_paths": { key.clone(): evil },
                    "agents": { "claude-code": evil }
                }}
            });
            std::fs::write(&paths.mounted_meta, raw.to_string()).unwrap();

            let r = unmount_from_custom_path(&paths, "demo", project.to_str().unwrap());
            assert!(r.is_err(), "custom_paths folder={evil:?} 应拒绝");
            // 校验先于 resolve_agent_skills_dir_by_id 与任何删除
            let r = unmount_one(&paths, root.as_path(), "demo", "claude-code");
            assert!(r.is_err(), "agents folder={evil:?} 应拒绝");
            assert!(neighbour.join("SKILL.md").exists(), "folder={evil:?} 不应删到邻居");
            assert!(project.exists(), "folder={evil:?} 不应删掉 skills 目录本身");
        }

        // 合法 folder 仍能卸载，且只删自己
        let own = project.join("demo-folder");
        std::fs::create_dir_all(&own).unwrap();
        let raw = serde_json::json!({
            "skills": { "demo": { "custom_paths": { key.clone(): "demo-folder" } } }
        });
        std::fs::write(&paths.mounted_meta, raw.to_string()).unwrap();
        assert!(unmount_from_custom_path(&paths, "demo", project.to_str().unwrap()).unwrap());
        assert!(!own.exists());
        assert!(neighbour.join("SKILL.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hub_mount_unmount_reject_bad_skill_ids() {
        let (root, paths) = temp_store("ids");
        assert!(mount_to_custom_path(&paths, "../x", "/tmp/skills").is_err());
        assert!(unmount_from_custom_path(&paths, "..", "/tmp/skills").is_err());
        assert!(unmount_one(&paths, root.as_path(), "a/b", "codex").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
