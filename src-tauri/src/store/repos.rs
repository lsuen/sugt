use crate::store::paths::{remove_dir_if_exists, StorePaths};
use crate::store::process::{hidden_command, run_hidden};
use crate::store::settings::{apply_github_proxy, load_settings};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepo {
    pub id: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub enabled: bool,
    #[serde(default)]
    pub last_refresh_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RepoStore {
    repos: Vec<SkillRepo>,
}

impl SkillRepo {
    pub fn new(owner: &str, repo: &str, branch: &str) -> Self {
        let id = format!(
            "{}-{}",
            owner.to_lowercase(),
            repo.to_lowercase()
        );
        Self {
            id,
            owner: owner.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            enabled: true,
            last_refresh_at: None,
            last_error: None,
        }
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

pub fn default_repos() -> Vec<SkillRepo> {
    vec![
        SkillRepo::new("ComposioHQ", "awesome-claude-skills", "master"),
        SkillRepo::new("JimLiu", "baoyu-skills", "main"),
        SkillRepo::new("anthropics", "skills", "main"),
        SkillRepo::new("cexll", "myclaude", "master"),
    ]
}

pub fn load_repos(store_paths: &StorePaths) -> Result<Vec<SkillRepo>> {
    if !store_paths.repos_file.exists() {
        let defaults = default_repos();
        save_repos(store_paths, &defaults)?;
        return Ok(defaults);
    }
    let raw = std::fs::read_to_string(&store_paths.repos_file)
        .with_context(|| format!("无法读取 {}", store_paths.repos_file.display()))?;
    let store: RepoStore = toml::from_str(&raw).context("仓库配置格式错误")?;
    Ok(store.repos)
}

pub fn save_repos(store_paths: &StorePaths, repos: &[SkillRepo]) -> Result<()> {
    store_paths.ensure_dirs()?;
    let store = RepoStore {
        repos: repos.to_vec(),
    };
    let serialized = toml::to_string_pretty(&store).context("仓库配置序列化失败")?;
    std::fs::write(&store_paths.repos_file, serialized)
        .with_context(|| format!("无法写入 {}", store_paths.repos_file.display()))?;
    Ok(())
}

pub fn git_available() -> bool {
    run_hidden("git", &["--version"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn refresh_repo(store_paths: &StorePaths, repo: &mut SkillRepo) -> Result<()> {
    if !git_available() {
        return Err(anyhow!("未检测到 git，请先安装 Git for Windows"));
    }
    let dest = store_paths.repo_cache_dir(&repo.id);
    remove_dir_if_exists(&dest)?;
    let dest_str = dest.to_str().unwrap_or_default();
    let settings = load_settings(store_paths)?;
    let clone_url = apply_github_proxy(&settings.github_proxy_prefix, &repo.clone_url());
    let output = hidden_command("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            &repo.branch,
            &clone_url,
            dest_str,
        ])
        .output()
        .context("无法执行 git clone")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(anyhow!(
            "git clone 失败：{} ({}){}",
            repo.label(),
            repo.branch,
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", detail)
            }
        ));
    }
    repo.last_error = None;
    repo.last_refresh_at = Some(format_timestamp(SystemTime::now()));
    Ok(())
}

fn format_timestamp(time: SystemTime) -> String {
    use std::time::UNIX_EPOCH;
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

pub fn add_repo(
    store_paths: &StorePaths,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Vec<SkillRepo>> {
    let owner = owner.trim();
    let repo_name = repo.trim();
    let branch = branch.trim();
    if owner.is_empty() || repo_name.is_empty() || branch.is_empty() {
        return Err(anyhow!("owner、repo、branch 不能为空"));
    }
    let mut repos = load_repos(store_paths)?;
    let new_repo = SkillRepo::new(owner, repo_name, branch);
    if repos.iter().any(|item| item.id == new_repo.id) {
        return Err(anyhow!("仓库已存在：{}", new_repo.label()));
    }
    repos.push(new_repo);
    save_repos(store_paths, &repos)?;
    Ok(repos)
}

pub fn remove_repo(store_paths: &StorePaths, repo_id: &str) -> Result<Vec<SkillRepo>> {
    let mut repos = load_repos(store_paths)?;
    repos.retain(|item| item.id != repo_id);
    save_repos(store_paths, &repos)?;
    remove_dir_if_exists(&store_paths.repo_cache_dir(repo_id))?;
    Ok(repos)
}

pub fn refresh_repo_by_id(store_paths: &StorePaths, repo_id: &str) -> Result<SkillRepo> {
    let mut repos = load_repos(store_paths)?;
    let repo = repos
        .iter_mut()
        .find(|item| item.id == repo_id)
        .ok_or_else(|| anyhow!("未找到仓库：{}", repo_id))?;
    match refresh_repo(store_paths, repo) {
        Ok(()) => {}
        Err(err) => {
            repo.last_error = Some(err.to_string());
            save_repos(store_paths, &repos)?;
            return Err(err);
        }
    }
    save_repos(store_paths, &repos)?;
    Ok(repos
        .into_iter()
        .find(|item| item.id == repo_id)
        .expect("repo exists"))
}

pub fn refresh_all_repos(store_paths: &StorePaths) -> Result<Vec<SkillRepo>> {
    let mut repos = load_repos(store_paths)?;
    for repo in &mut repos {
        if !repo.enabled {
            continue;
        }
        match refresh_repo(store_paths, repo) {
            Ok(()) => {}
            Err(err) => repo.last_error = Some(err.to_string()),
        }
    }
    save_repos(store_paths, &repos)?;
    Ok(repos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_id_is_stable() {
        let repo = SkillRepo::new("ComposioHQ", "awesome-claude-skills", "master");
        assert_eq!(repo.id, "composiohq-awesome-claude-skills");
        assert_eq!(
            repo.clone_url(),
            "https://github.com/ComposioHQ/awesome-claude-skills"
        );
    }
}
