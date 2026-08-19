use crate::store::paths::{remove_dir_if_exists, StorePaths};
use crate::store::process::{hidden_git, run_hidden};
use crate::store::settings::{apply_github_proxy, load_settings};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepo {
    pub id: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub repo: String,
    pub branch: String,
    /// 完整 git 地址（https / ssh）。空则按 GitHub owner/repo 拼装（兼容旧配置）。
    #[serde(default)]
    pub clone_url: String,
    /// 权重越高，发现技能列表中该仓库的 skill 越靠前。
    #[serde(default)]
    pub weight: i32,
    pub enabled: bool,
    #[serde(default)]
    pub last_refresh_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedGitRepo {
    pub clone_url: String,
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub is_github: bool,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RepoStore {
    repos: Vec<SkillRepo>,
}

impl SkillRepo {
    pub fn new(owner: &str, repo: &str, branch: &str) -> Self {
        Self::from_parsed(
            &ParsedGitRepo {
                clone_url: format!("https://github.com/{}/{}", owner.trim(), repo.trim()),
                host: "github.com".into(),
                owner: owner.trim().to_string(),
                repo: repo.trim().to_string(),
                is_github: true,
                label: format!("{}/{}", owner.trim(), repo.trim()),
            },
            branch,
            0,
        )
    }

    pub fn from_parsed(parsed: &ParsedGitRepo, branch: &str, weight: i32) -> Self {
        let id = format!(
            "{}-{}",
            parsed.owner.to_lowercase(),
            parsed.repo.to_lowercase()
        );
        Self {
            id,
            owner: parsed.owner.clone(),
            repo: parsed.repo.clone(),
            branch: branch.trim().to_string(),
            clone_url: parsed.clone_url.clone(),
            weight,
            enabled: true,
            last_refresh_at: None,
            last_error: None,
        }
    }

    pub fn clone_url(&self) -> String {
        let custom = self.clone_url.trim();
        if !custom.is_empty() {
            return custom.to_string();
        }
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    pub fn label(&self) -> String {
        if !self.owner.is_empty() && !self.repo.is_empty() {
            return format!("{}/{}", self.owner, self.repo);
        }
        self.clone_url()
    }

    pub fn is_github(&self) -> bool {
        self.clone_url().to_lowercase().contains("github.com")
    }
}

/// 解析完整 git 地址，或兼容 `owner/repo`（默认 GitHub）。
pub fn parse_git_url(input: &str) -> Result<ParsedGitRepo> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(anyhow!("git 地址不能为空"));
    }

    let (host, owner, repo) = if let Some(rest) = raw.strip_prefix("git@") {
        // git@host:owner/repo(.git)
        let (host, path) = rest
            .split_once(':')
            .ok_or_else(|| anyhow!("SSH 地址格式应为 git@host:owner/repo"))?;
        let mut parts = path.trim_matches('/').split('/');
        let owner = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("无法解析 owner"))?;
        let repo = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("无法解析 repo"))?;
        if parts.next().is_some() {
            return Err(anyhow!("地址路径过深，请使用 host/owner/repo"));
        }
        (host.to_string(), owner.to_string(), strip_git_suffix(repo))
    } else {
        let without_scheme = raw
            .strip_prefix("https://")
            .or_else(|| raw.strip_prefix("http://"))
            .unwrap_or(raw);
        let path = without_scheme.trim_matches('/');
        let mut parts = path.split('/');
        let first = parts
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("无法解析地址"))?;
        let second = parts.next().filter(|s| !s.is_empty());
        let third = parts.next().filter(|s| !s.is_empty());
        if parts.next().is_some() {
            return Err(anyhow!("地址路径过深，请使用 https://host/owner/repo"));
        }

        match (second, third) {
            // host/owner/repo
            (Some(owner), Some(repo)) => (
                first.to_string(),
                owner.to_string(),
                strip_git_suffix(repo),
            ),
            // owner/repo → 默认 GitHub
            (Some(repo), None) => (
                "github.com".to_string(),
                first.to_string(),
                strip_git_suffix(repo),
            ),
            _ => {
                return Err(anyhow!(
                    "无法解析，请填写完整地址，例如 https://atomgit.com/sunwl88/sun-skills"
                ))
            }
        }
    };

    if owner.is_empty() || repo.is_empty() || host.is_empty() {
        return Err(anyhow!("owner / repo / host 不能为空"));
    }
    if owner.contains(' ') || repo.contains(' ') {
        return Err(anyhow!("地址含非法空格"));
    }

    let is_github = host.eq_ignore_ascii_case("github.com")
        || host.eq_ignore_ascii_case("www.github.com");
    let clone_url = format!("https://{}/{}/{}", host, owner, repo);
    Ok(ParsedGitRepo {
        label: format!("{}/{}", owner, repo),
        clone_url,
        host,
        owner,
        repo,
        is_github,
    })
}

fn strip_git_suffix(name: &str) -> String {
    name.strip_suffix(".git")
        .unwrap_or(name)
        .trim()
        .to_string()
}

pub fn resolve_clone_url(repo: &SkillRepo, github_proxy_prefix: &str) -> String {
    let url = repo.clone_url();
    if repo.is_github() {
        apply_github_proxy(github_proxy_prefix, &url)
    } else {
        url
    }
}

pub fn test_repo_access(url: &str, branch: &str) -> Result<String> {
    if !git_available() {
        return Err(anyhow!("{}", git_missing_hint()));
    }
    let parsed = parse_git_url(url)?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(anyhow!("branch 不能为空"));
    }
    let output = hidden_git()
        .args([
            "-c",
            "credential.helper=",
            "ls-remote",
            "--heads",
            &parsed.clone_url,
            branch,
        ])
        .output()
        .context("无法执行 git ls-remote")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(anyhow!(
            "无法访问 {} ({}){}",
            parsed.label,
            branch,
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", detail)
            }
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Err(anyhow!(
            "仓库可访问，但未找到分支 {}（请确认分支名）",
            branch
        ));
    }
    Ok(format!(
        "解析成功：{} · {} · 分支 {} 可访问",
        parsed.host, parsed.label, branch
    ))
}

pub const PREFERRED_SUN_SKILLS_URL: &str = "https://gitee.com/swlgitee/sun-skills.git";
pub const PREFERRED_SUN_SKILLS_ID: &str = "swlgitee-sun-skills";

pub fn default_repos() -> Vec<SkillRepo> {
    vec![
        {
            // 官方推荐技能仓（Gitee，优先展示；启动时默认拉取）
            let mut r = SkillRepo::from_parsed(
                &parse_git_url(PREFERRED_SUN_SKILLS_URL).expect("default url"),
                "main",
                200,
            );
            r.enabled = true;
            r
        },
        {
            // Anthropic 官方技能（含 frontend-design 等）；国内需 GitHub 代理
            let mut r = SkillRepo::new("anthropics", "skills", "main");
            r.enabled = true;
            r.weight = 150;
            r
        },
        // 其它 GitHub 仓默认停用，避免「刷新全部」被境外大仓拖死；可在技能库配置里启用
        {
            let mut r = SkillRepo::new("ComposioHQ", "awesome-claude-skills", "master");
            r.enabled = false;
            r.weight = 50;
            r
        },
        {
            let mut r = SkillRepo::new("JimLiu", "baoyu-skills", "main");
            r.enabled = false;
            r.weight = 40;
            r
        },
        {
            let mut r = SkillRepo::new("cexll", "myclaude", "master");
            r.enabled = false;
            r.weight = 20;
            r
        },
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
    let mut repos = store.repos;
    if ensure_preferred_repos(&mut repos)? {
        save_repos(store_paths, &repos)?;
    }
    Ok(repos)
}

/// 已有用户配置：确保 Gitee sun-skills 存在且权重最高；弱化旧 AtomGit 源。
fn ensure_preferred_repos(repos: &mut Vec<SkillRepo>) -> Result<bool> {
    let preferred = parse_git_url(PREFERRED_SUN_SKILLS_URL)?;
    let preferred_id = PREFERRED_SUN_SKILLS_ID.to_string();
    let mut changed = false;

    let existing = repos.iter_mut().find(|repo| {
        repo.id == preferred_id
            || repo
                .clone_url
                .to_ascii_lowercase()
                .contains("gitee.com/swlgitee/sun-skills")
    });
    if let Some(repo) = existing {
        if repo.weight < 200 {
            repo.weight = 200;
            changed = true;
        }
        if repo.clone_url.trim().is_empty()
            || !repo
                .clone_url
                .to_ascii_lowercase()
                .contains("gitee.com/swlgitee/sun-skills")
        {
            repo.clone_url = preferred.clone_url.clone();
            repo.owner = preferred.owner.clone();
            repo.repo = preferred.repo.clone();
            changed = true;
        }
        if !repo.enabled {
            repo.enabled = true;
            changed = true;
        }
        if repo.branch.trim().is_empty() {
            repo.branch = "main".into();
            changed = true;
        }
    } else {
        let mut repo = SkillRepo::from_parsed(&preferred, "main", 200);
        repo.enabled = true;
        repos.insert(0, repo);
        changed = true;
    }

    // 确保 Anthropic 官方仓存在且启用（frontend-design 等出名技能在此）
    let anthropics_id = "anthropics-skills";
    if let Some(repo) = repos.iter_mut().find(|r| r.id == anthropics_id) {
        if repo.weight < 150 {
            repo.weight = 150;
            changed = true;
        }
        if !repo.enabled {
            repo.enabled = true;
            changed = true;
        }
    } else {
        let mut repo = SkillRepo::new("anthropics", "skills", "main");
        repo.enabled = true;
        repo.weight = 150;
        repos.insert(1.min(repos.len()), repo);
        changed = true;
    }

    for repo in repos.iter_mut() {
        let is_old_atomgit = repo.id == "sunwl88-sun-skills"
            || (repo
                .clone_url
                .to_ascii_lowercase()
                .contains("atomgit.com")
                && repo.repo.eq_ignore_ascii_case("sun-skills"));
        if is_old_atomgit && repo.weight > 10 {
            repo.weight = 10;
            changed = true;
        }
    }

    // 从未成功拉取过的「可选」GitHub 大仓：自动停用（不含 anthropics 官方仓）
    const DEFAULT_GH_IDLE: &[&str] = &[
        "composiohq-awesome-claude-skills",
        "jimliu-baoyu-skills",
        "cexll-myclaude",
    ];
    for repo in repos.iter_mut() {
        if DEFAULT_GH_IDLE.contains(&repo.id.as_str())
            && repo.enabled
            && repo.last_refresh_at.is_none()
        {
            repo.enabled = false;
            changed = true;
        }
    }

    if changed {
        repos.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label().cmp(&b.label())));
    }
    Ok(changed)
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

/// 按平台给出安装 Git 的指引文案
pub fn git_missing_hint() -> String {
    crate::platform::git_missing_hint().to_string()
}

pub fn refresh_repo(store_paths: &StorePaths, repo: &mut SkillRepo) -> Result<()> {
    if !git_available() {
        return Err(anyhow!("{}", git_missing_hint()));
    }
    let dest = store_paths.repo_cache_dir(&repo.id);
    let dest_str = dest.to_str().unwrap_or_default();
    let settings = load_settings(store_paths)?;
    let clone_url = resolve_clone_url(repo, &settings.github_proxy_prefix);
    let git_dir = dest.join(".git");

    let output = if git_dir.is_dir() {
        // 增量：fetch + hard reset，避免每次删库重下
        let fetch = hidden_git()
            .args([
                "-C",
                dest_str,
                "-c",
                "credential.helper=",
                "fetch",
                "--depth",
                "1",
                "origin",
                &repo.branch,
            ])
            .output()
            .context("无法执行 git fetch")?;
        if fetch.status.success() {
            hidden_git()
                .args(["-C", dest_str, "reset", "--hard", "FETCH_HEAD"])
                .output()
                .context("无法执行 git reset")?
        } else {
            remove_dir_if_exists(&dest)?;
            clone_repo_fresh(&clone_url, &repo.branch, dest_str)?
        }
    } else {
        remove_dir_if_exists(&dest)?;
        clone_repo_fresh(&clone_url, &repo.branch, dest_str)?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(anyhow!(
            "git 同步失败：{} ({}){}",
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

fn clone_repo_fresh(
    clone_url: &str,
    branch: &str,
    dest_str: &str,
) -> Result<std::process::Output> {
    hidden_git()
        .args([
            "-c",
            "credential.helper=",
            "clone",
            "--depth",
            "1",
            "--single-branch",
            "--branch",
            branch,
            clone_url,
            dest_str,
        ])
        .output()
        .context("无法执行 git clone")
}

/// 启动/发现页：预热推荐仓（Gitee）与 Anthropic 官方仓（含 frontend-design）。
/// 已有缓存则跳过；Anthropic 失败不阻断 Gitee 成功结果。
pub fn ensure_preferred_repo_cached(store_paths: &StorePaths) -> Result<PreferredWarmResult> {
    store_paths.ensure_dirs()?;
    let mut repos = load_repos(store_paths)?;
    let mut warmed_any = false;
    let mut messages: Vec<String> = Vec::new();

    let targets: Vec<String> = repos
        .iter()
        .filter(|repo| {
            repo.id == PREFERRED_SUN_SKILLS_ID
                || repo
                    .clone_url
                    .to_ascii_lowercase()
                    .contains("gitee.com/swlgitee/sun-skills")
                || repo.id == "anthropics-skills"
        })
        .map(|repo| repo.id.clone())
        .collect();

    if targets.is_empty() {
        return Ok(PreferredWarmResult {
            warmed: false,
            skipped: true,
            message: "未找到推荐技能仓库".into(),
        });
    }

    for id in targets {
        let Some(idx) = repos.iter().position(|r| r.id == id) else {
            continue;
        };
        let cache = store_paths.repo_cache_dir(&repos[idx].id);
        if cache.join(".git").is_dir() {
            continue;
        }
        match refresh_repo(store_paths, &mut repos[idx]) {
            Ok(()) => {
                warmed_any = true;
                messages.push(format!("已拉取 {}", repos[idx].label()));
            }
            Err(err) => {
                repos[idx].last_error = Some(err.to_string());
                messages.push(format!("{} 拉取失败（可配 GitHub 代理后重试）", repos[idx].label()));
            }
        }
    }

    save_repos(store_paths, &repos)?;
    if warmed_any {
        Ok(PreferredWarmResult {
            warmed: true,
            skipped: false,
            message: messages.join("；"),
        })
    } else if messages.is_empty() {
        Ok(PreferredWarmResult {
            warmed: false,
            skipped: true,
            message: "推荐技能库已就绪".into(),
        })
    } else {
        // 全是失败提示，仍返回 Ok 让前端展示 message，避免阻断发现页
        Ok(PreferredWarmResult {
            warmed: false,
            skipped: false,
            message: messages.join("；"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferredWarmResult {
    pub warmed: bool,
    pub skipped: bool,
    pub message: String,
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
    url: &str,
    branch: &str,
    weight: i32,
) -> Result<Vec<SkillRepo>> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(anyhow!("branch 不能为空"));
    }
    let parsed = parse_git_url(url)?;
    let mut repos = load_repos(store_paths)?;
    let new_repo = SkillRepo::from_parsed(&parsed, branch, weight);
    if repos.iter().any(|item| item.id == new_repo.id) {
        return Err(anyhow!("仓库已存在：{}", new_repo.label()));
    }
    repos.push(new_repo);
    repos.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label().cmp(&b.label())));
    save_repos(store_paths, &repos)?;
    Ok(repos)
}

pub fn update_repo(
    store_paths: &StorePaths,
    repo_id: &str,
    branch: Option<&str>,
    weight: Option<i32>,
    enabled: Option<bool>,
) -> Result<Vec<SkillRepo>> {
    let mut repos = load_repos(store_paths)?;
    let repo = repos
        .iter_mut()
        .find(|item| item.id == repo_id)
        .ok_or_else(|| anyhow!("未找到仓库：{}", repo_id))?;
    if let Some(branch) = branch.map(str::trim).filter(|s| !s.is_empty()) {
        repo.branch = branch.to_string();
    }
    if let Some(weight) = weight {
        repo.weight = weight;
    }
    if let Some(enabled) = enabled {
        repo.enabled = enabled;
    }
    repos.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label().cmp(&b.label())));
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
    // 高权重优先（推荐 Gitee 仓先完成），避免被境外 GitHub 拖住首屏
    repos.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.label().cmp(&b.label())));
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

    #[test]
    fn parses_https_atomgit_and_github() {
        let a = parse_git_url("https://atomgit.com/sunwl88/sun-skills.git").unwrap();
        assert_eq!(a.host, "atomgit.com");
        assert_eq!(a.owner, "sunwl88");
        assert_eq!(a.repo, "sun-skills");
        assert!(!a.is_github);
        assert_eq!(a.clone_url, "https://atomgit.com/sunwl88/sun-skills");

        let g = parse_git_url("https://github.com/anthropics/skills").unwrap();
        assert!(g.is_github);
        assert_eq!(g.owner, "anthropics");
        assert_eq!(g.repo, "skills");
    }

    #[test]
    fn parses_ssh_and_owner_repo_shorthand() {
        let s = parse_git_url("git@atomgit.com:sunwl88/sun-skills.git").unwrap();
        assert_eq!(s.host, "atomgit.com");
        assert_eq!(s.repo, "sun-skills");

        let short = parse_git_url("ComposioHQ/awesome-claude-skills").unwrap();
        assert_eq!(short.host, "github.com");
        assert!(short.is_github);
    }

    #[test]
    fn github_proxy_only_for_github() {
        let gh = SkillRepo::new("a", "b", "main");
        let ag = SkillRepo::from_parsed(
            &parse_git_url("https://atomgit.com/sunwl88/sun-skills").unwrap(),
            "main",
            10,
        );
        assert_eq!(
            resolve_clone_url(&gh, "https://ghfast.top"),
            "https://ghfast.top/https://github.com/a/b"
        );
        assert_eq!(
            resolve_clone_url(&ag, "https://ghfast.top"),
            "https://atomgit.com/sunwl88/sun-skills"
        );
    }

    #[test]
    fn default_repos_prefer_gitee_sun_skills() {
        let repos = default_repos();
        assert_eq!(repos[0].id, "swlgitee-sun-skills");
        assert_eq!(repos[0].weight, 200);
        assert!(repos[0]
            .clone_url
            .contains("gitee.com/swlgitee/sun-skills"));
        assert!(repos[0].enabled);
        assert_eq!(repos[1].id, "anthropics-skills");
        assert!(repos[1].enabled);
        assert!(repos.iter().skip(2).all(|r| !r.enabled));
    }

    #[test]
    fn ensure_preferred_repos_upserts_gitee() {
        let mut repos = vec![SkillRepo::new("anthropics", "skills", "main")];
        assert!(ensure_preferred_repos(&mut repos).unwrap());
        assert!(repos.iter().any(|r| r.id == "swlgitee-sun-skills" && r.weight == 200));
    }
}
