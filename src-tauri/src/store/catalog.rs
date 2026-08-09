use crate::store::paths::StorePaths;
use crate::store::repos::{load_repos, SkillRepo};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub repo_id: String,
    pub repo_label: String,
    pub relative_path: String,
    pub staged: bool,
    /// 任一客户端已挂载
    pub mounted: bool,
    pub mounted_claude: bool,
    pub mounted_codex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct StagedMeta {
    pub skills: HashMap<String, StagedSkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StagedSkillRecord {
    pub name: String,
    pub repo_id: String,
    pub relative_path: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct MountedMeta {
    pub skills: HashMap<String, MountedSkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MountedSkillRecord {
    #[serde(default)]
    pub claude_folder: Option<String>,
    #[serde(default)]
    pub codex_folder: Option<String>,
    /// v0.2.0 兼容字段，等同 claude_folder
    #[serde(default)]
    pub folder_name: Option<String>,
    #[serde(default)]
    pub mounted_at: Option<String>,
    /// agent 模板 id → 挂载目录下的文件夹名
    #[serde(default)]
    pub agents: HashMap<String, String>,
    /// 自定义/项目 skills 绝对路径 → 文件夹名
    #[serde(default)]
    pub custom_paths: HashMap<String, String>,
}

pub(crate) fn mount_flags(
    mounted: &MountedMeta,
    skill_id: &str,
) -> (bool, bool, bool) {
    let record = mounted.skills.get(skill_id);
    let claude = record.map(|r| r.is_mounted_claude()).unwrap_or(false);
    let codex = record.map(|r| r.is_mounted_codex()).unwrap_or(false);
    let any_agent = record.map(|r| !r.mounted_agent_ids().is_empty()).unwrap_or(false);
    (claude, codex, claude || codex || any_agent)
}

impl MountedSkillRecord {
    pub fn is_mounted_claude(&self) -> bool {
        self.claude_folder.is_some()
            || self.folder_name.is_some()
            || self.agents.contains_key("claude-code")
            || self.agents.contains_key("claude")
    }

    pub fn is_mounted_codex(&self) -> bool {
        self.codex_folder.is_some()
            || self.agents.contains_key("codex")
    }

    pub fn claude_folder_name(&self) -> Option<&str> {
        self.claude_folder
            .as_deref()
            .or(self.folder_name.as_deref())
            .or_else(|| self.agents.get("claude-code").map(|s| s.as_str()))
            .or_else(|| self.agents.get("claude").map(|s| s.as_str()))
    }

    pub fn mounted_agent_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.agents.keys().cloned().collect();
        if self.is_mounted_claude()
            && !ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case("claude-code") || id.eq_ignore_ascii_case("claude"))
        {
            ids.push("claude-code".into());
        }
        if self.codex_folder.is_some() && !ids.iter().any(|id| id.eq_ignore_ascii_case("codex")) {
            ids.push("codex".into());
        }
        ids.sort();
        ids.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        ids
    }

    pub fn set_agent_mount(&mut self, agent_id: &str, folder: &str) {
        self.agents.insert(agent_id.to_string(), folder.to_string());
        if agent_id.eq_ignore_ascii_case("claude-code") || agent_id.eq_ignore_ascii_case("claude") {
            self.claude_folder = Some(folder.to_string());
            self.folder_name = Some(folder.to_string());
        }
        if agent_id.eq_ignore_ascii_case("codex") {
            self.codex_folder = Some(folder.to_string());
        }
    }

    pub fn clear_agent_mount(&mut self, agent_id: &str) {
        self.agents
            .retain(|id, _| !id.eq_ignore_ascii_case(agent_id));
        if agent_id.eq_ignore_ascii_case("claude-code") || agent_id.eq_ignore_ascii_case("claude") {
            self.claude_folder = None;
            self.folder_name = None;
            self.agents
                .retain(|id, _| !id.eq_ignore_ascii_case("claude"));
        }
        if agent_id.eq_ignore_ascii_case("codex") {
            self.codex_folder = None;
        }
    }

    pub fn set_path_mount(&mut self, path_key: &str, folder: &str) {
        self.custom_paths
            .insert(path_key.to_string(), folder.to_string());
    }

    pub fn clear_path_mount(&mut self, path_key: &str) {
        self.custom_paths.remove(path_key);
    }

    pub fn mounted_custom_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.custom_paths.keys().cloned().collect();
        paths.sort();
        paths
    }

    pub fn is_empty_mount(&self) -> bool {
        self.mounted_agent_ids().is_empty() && self.custom_paths.is_empty()
    }
}

pub fn build_catalog(store_paths: &StorePaths) -> Result<Vec<SkillCatalogItem>> {
    let repos = load_repos(store_paths)?;
    let staged = load_staged_meta(store_paths)?;
    let mounted = load_mounted_meta(store_paths)?;
    let weight_by_repo: HashMap<&str, i32> = repos
        .iter()
        .map(|repo| (repo.id.as_str(), repo.weight))
        .collect();
    let mut items = Vec::new();

    for repo in repos.iter().filter(|repo| repo.enabled) {
        let repo_dir = store_paths.repo_cache_dir(&repo.id);
        if !repo_dir.exists() {
            continue;
        }
        scan_repo_skills(&repo_dir, &repo_dir, repo, &staged, &mounted, &mut items);
    }

    items.sort_by(|a, b| {
        let wa = weight_by_repo.get(a.repo_id.as_str()).copied().unwrap_or(0);
        let wb = weight_by_repo.get(b.repo_id.as_str()).copied().unwrap_or(0);
        wb.cmp(&wa)
            .then_with(|| a.repo_label.cmp(&b.repo_label))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(items)
}

fn scan_repo_skills(
    root: &Path,
    current: &Path,
    repo: &SkillRepo,
    staged: &StagedMeta,
    mounted: &MountedMeta,
    out: &mut Vec<SkillCatalogItem>,
) {
    let entries = match std::fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if skill_md.exists() {
            let relative = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.to_string_lossy().to_string());
            let skill_id = make_skill_id(repo, &relative);
            let (parsed_name, description) = parse_skill_md(&skill_md).unwrap_or_else(|_| {
                (
                    path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| skill_id.clone()),
                    None,
                )
            });
            let staged_flag = staged.skills.contains_key(&skill_id);
            let (mounted_claude, mounted_codex, mounted_any) = mount_flags(mounted, &skill_id);
            out.push(SkillCatalogItem {
                id: skill_id,
                name: parsed_name,
                description,
                repo_id: repo.id.clone(),
                repo_label: repo.label(),
                relative_path: relative,
                staged: staged_flag,
                mounted: mounted_any,
                mounted_claude,
                mounted_codex,
            });
        } else {
            scan_repo_skills(root, &path, repo, staged, mounted, out);
        }
    }
}

pub fn make_skill_id(repo: &SkillRepo, relative_path: &str) -> String {
    let slug = relative_path
        .replace('/', "--")
        .replace('\\', "--")
        .replace(' ', "-");
    format!("{}__{}", repo.id, slug)
}

pub fn parse_skill_md(path: &Path) -> Result<(String, Option<String>)> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("无法读取 {}", path.display()))?;
    let mut name = None;
    let mut description = None;

    if raw.starts_with("---") {
        if let Some(end) = raw[3..].find("---") {
            let front = &raw[3..3 + end];
            for line in front.lines() {
                let line = line.trim();
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim().to_lowercase();
                    let value = value.trim().trim_matches('"');
                    if key == "name" && !value.is_empty() {
                        name = Some(value.to_string());
                    }
                    if key == "description" && !value.is_empty() {
                        description = Some(value.to_string());
                    }
                }
            }
        }
    }

    if name.is_none() {
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                name = Some(trimmed[2..].trim().to_string());
                break;
            }
        }
    }

    if description.is_none() {
        let mut after_heading = false;
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                after_heading = true;
                continue;
            }
            if after_heading && !trimmed.is_empty() && !trimmed.starts_with('#') {
                description = Some(trimmed.to_string());
                break;
            }
        }
    }

    let fallback = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_string());
    Ok((name.unwrap_or(fallback), description))
}

pub(crate) fn load_staged_meta(store_paths: &StorePaths) -> Result<StagedMeta> {
    if !store_paths.staged_meta.exists() {
        return Ok(StagedMeta::default());
    }
    let raw = std::fs::read_to_string(&store_paths.staged_meta)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub(crate) fn save_staged_meta(store_paths: &StorePaths, meta: &StagedMeta) -> Result<()> {
    store_paths.ensure_dirs()?;
    let raw = serde_json::to_string_pretty(meta)?;
    std::fs::write(&store_paths.staged_meta, raw)?;
    Ok(())
}

pub(crate) fn load_mounted_meta(store_paths: &StorePaths) -> Result<MountedMeta> {
    if !store_paths.mounted_meta.exists() {
        return Ok(MountedMeta::default());
    }
    let raw = std::fs::read_to_string(&store_paths.mounted_meta)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub(crate) fn save_mounted_meta(store_paths: &StorePaths, meta: &MountedMeta) -> Result<()> {
    store_paths.ensure_dirs()?;
    let raw = serde_json::to_string_pretty(meta)?;
    std::fs::write(&store_paths.mounted_meta, raw)?;
    Ok(())
}

pub fn list_staged_skills(store_paths: &StorePaths) -> Result<Vec<SkillCatalogItem>> {
    let meta = load_staged_meta(store_paths)?;
    let mounted = load_mounted_meta(store_paths)?;
    let mut items = Vec::new();
    for (id, record) in meta.skills {
        let (mounted_claude, mounted_codex, mounted_any) = mount_flags(&mounted, &id);
        items.push(SkillCatalogItem {
            id,
            name: record.name,
            description: None,
            repo_id: record.repo_id.clone(),
            repo_label: record.repo_id,
            relative_path: record.relative_path,
            staged: true,
            mounted: mounted_any,
            mounted_claude,
            mounted_codex,
        });
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

pub fn find_skill_source(
    store_paths: &StorePaths,
    repo_id: &str,
    relative_path: &str,
) -> Result<PathBuf> {
    let repo_dir = store_paths.repo_cache_dir(repo_id);
    let source = repo_dir.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if source.exists() {
        Ok(source)
    } else {
        Err(anyhow::anyhow!("技能源目录不存在，请先刷新仓库"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_skill_md() {
        let dir = std::env::temp_dir().join("sugt-skill-parse-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: Demo Skill\ndescription: Hello world\n---\n# ignored\n",
        )
        .unwrap();
        let (name, desc) = parse_skill_md(&path).unwrap();
        assert_eq!(name, "Demo Skill");
        assert_eq!(desc.as_deref(), Some("Hello world"));
        std::fs::remove_dir_all(dir).ok();
    }
}
