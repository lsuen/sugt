//! 从 GitHub / Gitee 等托管地址一步安装 skill 到「技能 → 本地技能」。
//! 作者：sun / 孙文龙
//!
//! 复用既有确定性链路：`parse_git_url` 解析 → `git clone --depth 1`（含代理与分支回退）
//! → 扫 `SKILL.md` → `replace_dir` 原子落地 → `staged.json` 记录来源。
//! 不引入新依赖、不写客户端配置、不把写操作交给模型；失败按事务回滚并清理临时 clone。

use crate::store::catalog::{load_staged_meta, parse_skill_md, save_staged_meta, StagedSkillRecord};
use crate::store::paths::{remove_dir_if_exists, replace_dir, StorePaths};
use crate::store::process::hidden_git;
use crate::store::repos::{git_available, git_missing_hint, parse_git_url, resolve_clone_url, ParsedGitRepo, SkillRepo};
use crate::store::settings::load_settings;
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 仅允许公开代码托管域名，避免把任意 host 交给 git clone。
const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "www.github.com",
    "gitee.com",
    "gitlab.com",
    "bitbucket.org",
    "gitcode.com",
];
const MAX_SKILLS: usize = 60;
const MAX_DEPTH: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct GitSkillInstallResult {
    pub source: String,
    pub branch: String,
    pub installed: Vec<String>,
    pub skill_ids: Vec<String>,
    /// 面向用户的下一步引导（前端直接展示）
    pub note: String,
}

/// 解析并校验来源，拒绝非白名单 host 与路径穿越。
fn validate_source(parsed: &ParsedGitRepo) -> Result<()> {
    let host = parsed.host.to_ascii_lowercase();
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        return Err(anyhow!(
            "仅支持 GitHub / Gitee / GitLab / Bitbucket / GitCode 地址，当前为 {}",
            parsed.host
        ));
    }
    for part in [&parsed.owner, &parsed.repo] {
        if part.is_empty()
            || part == "."
            || part == ".."
            || !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(anyhow!("仓库地址含非法字符：{}", parsed.label));
        }
    }
    Ok(())
}

/// 清洗成单个目录名：字母数字 / `_` / `-` 保留，其余字符（含点号）折叠成 `-`。
/// id 会直接作为 `skills_dir` 下的目录名，因此绝不允许残留 `..`。
fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// 相对路径 → slug。先丢弃 `.` / `..` 等纯点号路径段（路径穿越语义），
/// 再逐段清洗，保证结果只含一个路径组件且不含 `..`。
fn sanitize_relative(relative: &str) -> String {
    let parts: Vec<&str> = relative
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.chars().all(|c| c == '.'))
        .collect();
    if parts.is_empty() {
        return "root".to_string();
    }
    sanitize(&parts.join("--"))
}

fn make_import_id(parsed: &ParsedGitRepo, relative: &str) -> String {
    let base = format!("{}-{}", sanitize(&parsed.owner), sanitize(&parsed.repo));
    format!("git__{}__{}", base, sanitize_relative(relative))
}

fn relative_label(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 与 `catalog::scan_repo_skills` 同规则：目录下有 `SKILL.md` 就是一个技能，不再往里嵌套扫。
fn scan_skill_dirs(root: &Path, current: &Path, depth: usize) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if current.join("SKILL.md").is_file() {
        out.push(current.to_path_buf());
        return Ok(out);
    }
    if depth >= MAX_DEPTH {
        return Ok(out);
    }
    let entries = match std::fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        out.extend(scan_skill_dirs(root, &path, depth + 1)?);
        if out.len() > MAX_SKILLS {
            break;
        }
    }
    Ok(out)
}

/// `main` 失败自动回落 `master`；用户显式指定分支时不猜。
fn clone_with_branch_fallback(
    store_paths: &StorePaths,
    parsed: &ParsedGitRepo,
    requested: Option<&str>,
    dest: &Path,
) -> Result<String> {
    let settings = load_settings(store_paths)?;
    let repo = SkillRepo::from_parsed(parsed, "main", 0);
    let clone_url = resolve_clone_url(&repo, &settings.github_proxy_prefix);
    let branches: Vec<&str> = match requested.map(str::trim).filter(|b| !b.is_empty()) {
        Some(branch) => vec![branch],
        None => vec!["main", "master"],
    };
    let dest_str = dest.to_string_lossy().to_string();
    let mut last_error = String::new();
    for branch in &branches {
        let output = hidden_git()
            .args([
                "-c",
                "credential.helper=",
                "clone",
                "--depth",
                "1",
                "--single-branch",
                "--branch",
                branch,
                &clone_url,
                &dest_str,
            ])
            .output()
            .context("无法执行 git clone")?;
        if output.status.success() {
            return Ok((*branch).to_string());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // 失败的 clone 可能留下空目录，先清干净再试下一个分支。
        remove_dir_if_exists(dest).ok();
    }
    Err(anyhow!(
        "克隆失败（已尝试 {}）：{}",
        branches.join(" / "),
        if last_error.is_empty() {
            "请检查地址、网络或 GitHub 代理设置"
        } else {
            &last_error
        }
    ))
}

/// 一步安装。幂等：同一地址重复安装覆盖同一批 id，不会在列表里堆积重复项。
pub fn install_from_git_url(
    store_paths: &StorePaths,
    url: &str,
    branch: Option<&str>,
) -> Result<GitSkillInstallResult> {
    let parsed = parse_git_url(url)?;
    validate_source(&parsed)?;
    if !git_available() {
        return Err(anyhow!("{}", git_missing_hint()));
    }
    store_paths.ensure_dirs()?;

    let temp_id = format!(
        "sugt-import-{}-{}",
        sanitize(&parsed.owner),
        sanitize(&parsed.repo)
    );
    let clone_dir = store_paths.repo_cache_dir(&temp_id);
    remove_dir_if_exists(&clone_dir)?;

    let branch_used = match clone_with_branch_fallback(store_paths, &parsed, branch, &clone_dir) {
        Ok(branch) => branch,
        Err(err) => {
            remove_dir_if_exists(&clone_dir).ok();
            return Err(err);
        }
    };

    let skill_dirs = match scan_skill_dirs(&clone_dir, &clone_dir, 0) {
        Ok(dirs) if dirs.is_empty() => {
            remove_dir_if_exists(&clone_dir).ok();
            return Err(anyhow!(
                "仓库里没有找到 SKILL.md：{}。技能包必须包含 SKILL.md",
                parsed.label
            ));
        }
        Ok(dirs) if dirs.len() > MAX_SKILLS => {
            remove_dir_if_exists(&clone_dir).ok();
            return Err(anyhow!(
                "仓库含 {} 个技能，超过单次上限 {}，请先精简或改用「添加仓库」分批安装",
                dirs.len(),
                MAX_SKILLS
            ));
        }
        Ok(dirs) => dirs,
        Err(err) => {
            remove_dir_if_exists(&clone_dir).ok();
            return Err(err);
        }
    };

    let staged_before = load_staged_meta(store_paths)?;
    let mut staged = staged_before.clone();
    let mut written_ids: Vec<String> = Vec::new();
    let mut installed: Vec<String> = Vec::new();

    let outcome = (|| -> Result<()> {
        for dir in &skill_dirs {
            let relative = relative_label(&clone_dir, dir);
            let id = make_import_id(&parsed, &relative);
            let dest = store_paths.staged_skill_dir(&id);
            replace_dir(dir, &dest)?;
            written_ids.push(id.clone());
            let (name, _) = parse_skill_md(&dir.join("SKILL.md")).unwrap_or_else(|_| {
                (
                    dir.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| parsed.repo.clone()),
                    None,
                )
            });
            installed.push(name.clone());
            staged.skills.insert(
                id,
                StagedSkillRecord {
                    name,
                    repo_id: "git-import".to_string(),
                    relative_path: relative,
                    installed_at: crate::store::install::now_secs(),
                },
            );
        }
        save_staged_meta(store_paths, &staged)?;
        Ok(())
    })();

    if let Err(err) = outcome {
        // 事务式回滚：元数据还原到安装前，删除本次已落地的目录，绝不留半安装状态。
        save_staged_meta(store_paths, &staged_before).ok();
        for id in &written_ids {
            remove_dir_if_exists(&store_paths.staged_skill_dir(id)).ok();
        }
        remove_dir_if_exists(&clone_dir).ok();
        return Err(err.context("安装失败，已回滚到安装前状态"));
    }

    let count = installed.len();
    let note = format!(
        "已从 {} 安装 {} 个技能到「技能 → 本地技能」。请在该页勾选技能并挂载到目标 Agent；触发词与优化建议会一并展示。",
        parsed.label, count
    );
    remove_dir_if_exists(&clone_dir).ok();
    Ok(GitSkillInstallResult {
        source: parsed.label,
        branch: branch_used,
        installed,
        skill_ids: written_ids,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(host: &str) -> ParsedGitRepo {
        ParsedGitRepo {
            clone_url: format!("https://{}/owner/repo", host),
            host: host.to_string(),
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            is_github: host == "github.com",
            label: "owner/repo".to_string(),
        }
    }

    #[test]
    fn only_known_hosts_are_allowed() {
        for host in ["github.com", "www.github.com", "gitee.com", "gitlab.com"] {
            assert!(validate_source(&parsed(host)).is_ok(), "{host} 应被允许");
        }
        assert!(validate_source(&parsed("evil.example.com")).is_err());
    }

    #[test]
    fn path_traversal_in_repo_parts_is_rejected() {
        let mut bad = parsed("github.com");
        bad.owner = "..".to_string();
        assert!(validate_source(&bad).is_err());
        bad.owner = "owner".to_string();
        bad.repo = "repo/../../etc".to_string();
        assert!(validate_source(&bad).is_err());
    }

    #[test]
    fn import_ids_are_stable_and_sanitized() {
        let p = parsed("github.com");
        let first = make_import_id(&p, "skills/demo");
        assert_eq!(first, make_import_id(&p, "skills/demo"));
        assert_eq!(first, "git__owner-repo__skills--demo");
        assert!(make_import_id(&p, "").ends_with("__root"));
        // id 是目录名，任何路径穿越形态都不能残留 `..` 或路径分隔符。
        for evil in ["../evil", "../../etc/passwd", "a/../../b", "..\\evil", "a..b"] {
            let id = make_import_id(&p, evil);
            assert!(!id.contains(".."), "{evil} -> {id} 仍含 ..");
            assert!(!id.contains('/') && !id.contains('\\'), "{evil} -> {id} 含分隔符");
        }
        let mut dotted = parsed("github.com");
        dotted.owner = "owner.name".to_string();
        dotted.repo = "repo.js".to_string();
        assert!(!make_import_id(&dotted, "skills/demo").contains(".."));
        assert_eq!(make_import_id(&dotted, "skills/demo"), make_import_id(&dotted, "skills/demo"));
    }

    #[test]
    fn scan_finds_skill_md_and_skips_dot_dirs() {
        let root = std::env::temp_dir().join(format!("sugt-git-scan-{}", std::process::id()));
        remove_dir_if_exists(&root).ok();
        std::fs::create_dir_all(root.join("skills/one")).unwrap();
        std::fs::create_dir_all(root.join("skills/two")).unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join("skills/one/SKILL.md"), "---\nname: One\n---\n").unwrap();
        std::fs::write(root.join("skills/two/SKILL.md"), "---\nname: Two\n---\n").unwrap();
        let found = scan_skill_dirs(&root, &root, 0).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|p| !p.to_string_lossy().contains(".git/objects")));
        remove_dir_if_exists(&root).ok();
    }

    #[test]
    fn repo_root_can_be_a_single_skill() {
        let root = std::env::temp_dir().join(format!("sugt-git-root-{}", std::process::id()));
        remove_dir_if_exists(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("SKILL.md"), "---\nname: Root Skill\n---\n").unwrap();
        let found = scan_skill_dirs(&root, &root, 0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(relative_label(&root, &found[0]), "");
        remove_dir_if_exists(&root).ok();
    }
}
