//! 客户端「高级」子页：Skills / 插件 / 配置盘点。

use crate::store::catalog::{load_mounted_meta, load_staged_meta, parse_skill_md, save_staged_meta, StagedSkillRecord};
use crate::store::install::{folder_name_from_record, now_secs};
use crate::store::paths::{copy_dir_all, StorePaths};
use crate::store::plugins::PluginItemView;
use crate::takeover_profiles::{self, expand_home, TakeoverProfile};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_CONFIG_CHARS: usize = 200_000;

#[derive(Debug, Clone, Serialize)]
pub struct AgentAdvancedSkill {
    pub folder: String,
    pub name: String,
    pub description: Option<String>,
    pub path: String,
    /// none | library | mounted
    pub sugt_mark: Option<String>,
    pub sugt_skill_id: Option<String>,
    pub in_library: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentAdvancedConfig {
    pub path: Option<String>,
    pub exists: bool,
    pub content: Option<String>,
    pub truncated: bool,
    pub error: Option<String>,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentAdvancedView {
    pub profile_id: String,
    pub profile_name: String,
    pub settings_root: Option<String>,
    pub settings_root_exists: bool,
    pub skills_dir: Option<String>,
    pub skills_dir_exists: bool,
    /// 实际扫描到的全部技能目录
    pub skills_dirs: Vec<String>,
    /// 用户配置的 skill_dirs（;` 分隔，可编辑）
    pub skill_dirs_text: String,
    pub plugins_dir: Option<String>,
    pub plugins_dir_exists: bool,
    pub skills: Vec<AgentAdvancedSkill>,
    pub plugins: Vec<PluginItemView>,
    pub config: AgentAdvancedConfig,
}

pub fn get_agent_advanced(
    store_paths: &StorePaths,
    config_dir: &Path,
    profile_id: &str,
) -> Result<AgentAdvancedView> {
    let profile = takeover_profiles::find_profile(config_dir, profile_id)
        .ok_or_else(|| anyhow!("未找到 Agent：{}", profile_id))?;

    let settings_root = takeover_profiles::resolved_settings_path(&profile);
    let settings_root_exists = settings_root.as_ref().is_some_and(|p| p.is_dir());

    let skills_dirs = takeover_profiles::collect_skill_dirs(&profile);
    let skills_dir = skills_dirs.first().cloned();
    let skills_dir_exists = skills_dir.as_ref().is_some_and(|p| p.is_dir());
    let skills_dirs_display: Vec<String> = skills_dirs
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    let plugins_dir = resolve_plugins_dir(&profile);
    let plugins_dir_exists = plugins_dir.as_ref().is_some_and(|p| p.is_dir());

    let mut skills = Vec::new();
    for dir in &skills_dirs {
        match list_agent_skills(store_paths, &profile, dir) {
            Ok(mut items) => skills.append(&mut items),
            Err(_) => continue,
        }
    }
    // 同名文件夹去重（保留先扫到的）
    {
        let mut seen = std::collections::HashSet::new();
        skills.retain(|s| seen.insert(s.folder.to_ascii_lowercase()));
    }
    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let plugins = if let Some(ref dir) = plugins_dir {
        if dir.is_dir() {
            list_plugins_in_dir(dir)?
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let config = load_config_view(&profile, settings_root.as_deref());

    Ok(AgentAdvancedView {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        settings_root: settings_root.map(|p| p.display().to_string()),
        settings_root_exists,
        skills_dir: skills_dir.map(|p| p.display().to_string()),
        skills_dir_exists,
        skills_dirs: skills_dirs_display,
        skill_dirs_text: profile.skill_dirs.join(";"),
        plugins_dir: plugins_dir.map(|p| p.display().to_string()),
        plugins_dir_exists,
        skills,
        plugins,
        config,
    })
}

fn resolve_plugins_dir(profile: &TakeoverProfile) -> Option<PathBuf> {
    if let Some(root) = takeover_profiles::resolved_settings_path(profile) {
        let under = root.join("plugins");
        if under.is_dir() {
            return Some(under);
        }
    }
    if profile.id.eq_ignore_ascii_case("claude-code")
        || profile.id.eq_ignore_ascii_case("claude")
        || profile.protocol.to_ascii_lowercase().contains("anthropic")
    {
        return StorePaths::claude_plugins_dir().ok();
    }
    None
}

fn list_agent_skills(
    store_paths: &StorePaths,
    profile: &TakeoverProfile,
    skills_dir: &Path,
) -> Result<Vec<AgentAdvancedSkill>> {
    let staged = load_staged_meta(store_paths)?;
    let mounted = load_mounted_meta(store_paths)?;

    // folder_name → skill_id（sugt 挂到本 Agent 的）
    let mut mounted_folders: HashMap<String, String> = HashMap::new();
    for (skill_id, rec) in &mounted.skills {
        if let Some(folder) = rec.agents.get(&profile.id) {
            mounted_folders.insert(folder.clone(), skill_id.clone());
        } else if profile.id.eq_ignore_ascii_case("claude-code")
            || profile.id.eq_ignore_ascii_case("claude")
        {
            if let Some(folder) = rec.claude_folder_name() {
                mounted_folders.insert(folder.to_string(), skill_id.clone());
            }
        } else if profile.id.eq_ignore_ascii_case("codex") {
            if let Some(folder) = &rec.codex_folder {
                mounted_folders.insert(folder.clone(), skill_id.clone());
            }
        }
    }

    // folder_name → skill_id（中控库已有，按目录名匹配）
    let mut library_folders: HashMap<String, String> = HashMap::new();
    for (skill_id, rec) in &staged.skills {
        let folder = folder_name_from_record(rec);
        library_folders.insert(folder, skill_id.clone());
        library_folders.insert(skill_id.clone(), skill_id.clone());
    }

    let mut items = Vec::new();
    let entries = std::fs::read_dir(skills_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("SKILL.md").is_file() {
            continue;
        }
        let folder = entry.file_name().to_string_lossy().to_string();
        if folder.starts_with('.') {
            continue;
        }
        let (name, description) =
            parse_skill_md(&path.join("SKILL.md")).unwrap_or_else(|_| (folder.clone(), None));

        let (sugt_mark, sugt_skill_id, in_library) =
            if let Some(id) = mounted_folders.get(&folder) {
                (Some("mounted".into()), Some(id.clone()), true)
            } else if let Some(id) = library_folders.get(&folder) {
                (Some("library".into()), Some(id.clone()), true)
            } else {
                (None, None, false)
            };

        items.push(AgentAdvancedSkill {
            folder,
            name,
            description,
            path: path.display().to_string(),
            sugt_mark,
            sugt_skill_id,
            in_library,
        });
    }
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

fn list_plugins_in_dir(dir: &Path) -> Result<Vec<PluginItemView>> {
    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        items.push(PluginItemView {
            name,
            path: path.display().to_string(),
            source: None,
        });
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

fn load_config_view(profile: &TakeoverProfile, settings_root: Option<&Path>) -> AgentAdvancedConfig {
    let candidates = config_candidates(profile, settings_root);
    let candidate_strs: Vec<String> = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    let Some(path) = candidates.into_iter().find(|p| p.is_file()) else {
        return AgentAdvancedConfig {
            path: None,
            exists: false,
            content: None,
            truncated: false,
            error: Some("未找到配置文件".into()),
            candidates: candidate_strs,
        };
    };

    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let truncated = raw.len() > MAX_CONFIG_CHARS;
            let content = if truncated {
                let mut s = raw.chars().take(MAX_CONFIG_CHARS).collect::<String>();
                s.push_str("\n\n…（内容过长已截断）");
                s
            } else {
                raw
            };
            AgentAdvancedConfig {
                path: Some(path.display().to_string()),
                exists: true,
                content: Some(content),
                truncated,
                error: None,
                candidates: candidate_strs,
            }
        }
        Err(e) => AgentAdvancedConfig {
            path: Some(path.display().to_string()),
            exists: true,
            content: None,
            truncated: false,
            error: Some(format!("读取失败：{e}")),
            candidates: candidate_strs,
        },
    }
}

fn config_candidates(profile: &TakeoverProfile, settings_root: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let id = profile.id.to_ascii_lowercase();
    if let Some(root) = settings_root {
        if id.contains("codex") {
            out.push(root.join("config.toml"));
            out.push(root.join("config.json"));
        } else if id.contains("claude") || profile.protocol.to_ascii_lowercase().contains("anthropic")
        {
            out.push(root.join("settings.json"));
            out.push(root.join("settings.local.json"));
            out.push(root.join(".claude.json"));
        } else if id.contains("cursor") {
            out.push(root.join("argv.json"));
            out.push(root.join("mcp.json"));
        } else {
            out.push(root.join("settings.json"));
            out.push(root.join("config.toml"));
            out.push(root.join("config.json"));
        }
    }
    // 内置兜底
    if id.contains("codex") {
        if let Some(p) = expand_home("~/.codex/config.toml") {
            out.push(p);
        }
    }
    if id.contains("claude") {
        if let Some(p) = expand_home("~/.claude/settings.json") {
            out.push(p);
        }
    }
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    out.dedup();
    out
}

/// 将 Agent 本地技能目录复制进 SUGT 中控库。
pub fn import_skill_to_library(
    store_paths: &StorePaths,
    source_path: &str,
) -> Result<crate::store::hub::LocalSkillView> {
    let source = expand_home(source_path.trim())
        .filter(|p| p.is_dir())
        .ok_or_else(|| anyhow!("技能目录不存在：{}", source_path))?;
    if !source.join("SKILL.md").is_file() {
        return Err(anyhow!("目录内没有 SKILL.md：{}", source.display()));
    }

    let folder = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let (name, description) =
        parse_skill_md(&source.join("SKILL.md")).unwrap_or_else(|_| (folder.clone(), None));

    let slug = slugify(&folder);
    let skill_id = format!("imported__{}", slug);

    let staged = load_staged_meta(store_paths)?;
    // 已存在同文件夹名的库内技能 → 视为已入库
    for (id, rec) in &staged.skills {
        if folder_name_from_record(rec) == folder || id == &skill_id {
            return Err(anyhow!("中控库已存在该技能：{}", rec.name));
        }
    }

    let dest = store_paths.staged_skill_dir(&skill_id);
    if dest.exists() {
        return Err(anyhow!("中控库目录已存在：{}", skill_id));
    }
    copy_dir_all(&source, &dest)?;

    let mut staged = load_staged_meta(store_paths)?;
    staged.skills.insert(
        skill_id.clone(),
        StagedSkillRecord {
            name: name.clone(),
            repo_id: "import".into(),
            relative_path: folder.clone(),
            installed_at: now_secs(),
        },
    );
    save_staged_meta(store_paths, &staged)?;

    Ok(crate::store::hub::LocalSkillView {
        id: skill_id,
        name,
        description,
        repo_id: "import".into(),
        repo_label: "从客户端入库".into(),
        relative_path: folder,
        mounted_agents: vec![],
        mounted_paths: vec![],
        is_local: true,
    })
}

/// 保存高级页路径。
/// - `path`：Agent 配置根（settings_path）
/// - `skill_dirs`：技能目录，`;` / 换行 / `|` 分隔；传空字符串可清空为仅用内置指纹
pub fn save_agent_root_path(
    config_dir: &Path,
    profile_id: &str,
    path: Option<String>,
    skill_dirs: Option<String>,
) -> Result<()> {
    let mut profile =
        takeover_profiles::find_profile(config_dir, profile_id).ok_or_else(|| anyhow!("未找到 Agent"))?;

    if let Some(path_raw) = path {
        let trimmed = path_raw.trim().to_string();
        if trimmed.is_empty() {
            profile.settings_path = None;
        } else {
            let expanded =
                expand_home(&trimmed).ok_or_else(|| anyhow!("无法解析路径：{}", trimmed))?;
            if !expanded.exists() {
                return Err(anyhow!("目录不存在：{}", expanded.display()));
            }
            if !expanded.is_dir() {
                return Err(anyhow!("不是目录：{}", expanded.display()));
            }
            profile.settings_path = Some(trimmed.clone());
            profile.settings_dirs = vec![trimmed];
        }
    }

    if let Some(dirs_raw) = skill_dirs {
        let parsed = parse_multi_dirs(&dirs_raw);
        // 允许保存尚未创建的路径（仅校验能解析）；存在的必须是目录
        for raw in &parsed {
            let Some(expanded) = expand_home(raw) else {
                return Err(anyhow!("无法解析路径：{}", raw));
            };
            if expanded.exists() && !expanded.is_dir() {
                return Err(anyhow!("不是目录：{}", expanded.display()));
            }
        }
        profile.skill_dirs = parsed;
    }

    takeover_profiles::upsert_profile(config_dir, profile)?;
    Ok(())
}

fn parse_multi_dirs(raw: &str) -> Vec<String> {
    raw.split(|c| c == ';' || c == '\n' || c == '|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
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
