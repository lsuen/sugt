use crate::model::AppConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

const CUSTOM_FILE: &str = "takeover-profiles.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeoverProfile {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub protocol: String,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub clear_vars: Vec<String>,
    #[serde(default)]
    pub settings_dirs: Vec<String>,
    /// 用户指定或覆盖的配置目录（绝对路径或 ~/...）
    #[serde(default)]
    pub settings_path: Option<String>,
    #[serde(default)]
    pub skill_dirs: Vec<String>,
    #[serde(default)]
    pub launch_command: Option<String>,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ProfileStore {
    #[serde(default)]
    profiles: Vec<TakeoverProfile>,
    #[serde(default)]
    deleted_builtin_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TakeoverProfileView {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub protocol: String,
    /// 偏好层「应接管」（跨重启保留）；退出清环境后仍为 true
    pub configured: bool,
    /// CLI 是否在 PATH / 常见安装路径中找到
    pub cli_detected: bool,
    /// 配置目录是否存在
    pub settings_detected: bool,
    /// 综合：已发现客户端（CLI 或配置目录任一）
    pub discovered: bool,
    pub skills_detected: bool,
    pub settings_path: Option<String>,
    pub cli_path: Option<String>,
    pub env_preview: BTreeMap<String, String>,
    pub env_vars: BTreeMap<String, String>,
    pub clear_vars: Vec<String>,
    pub settings_dirs: Vec<String>,
    pub skill_dirs: Vec<String>,
    pub launch_command: Option<String>,
    pub builtin: bool,
    pub can_delete: bool,
    /// 全局技能目录内扫到的 SKILL.md 数量
    pub skills_count: usize,
    pub plugins_count: usize,
}

pub fn builtin_profiles() -> Vec<TakeoverProfile> {
    vec![
        profile(
            "claude-code",
            "Claude Code",
            "Anthropic",
            "ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN",
            "anthropic",
            btreemap(&[
                ("ANTHROPIC_BASE_URL", "{anthropic_base}"),
                ("ANTHROPIC_AUTH_TOKEN", "{client_key}"),
            ]),
            vec!["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"],
            vec!["~/.claude"],
            vec!["~/.claude/skills"],
            Some("claude"),
        ),
        profile(
            "codex",
            "OpenAI Codex CLI",
            "OpenAI",
            "OPENAI_API_KEY + OPENAI_BASE_URL（/v1）",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
                ("OPENAI_API_BASE", "{openai_base}"),
            ]),
            vec![],
            vec!["~/.codex"],
            vec!["~/.agents/skills"],
            Some("codex"),
        ),
    ]
}

fn profile(
    id: &str,
    name: &str,
    vendor: &str,
    description: &str,
    protocol: &str,
    env_vars: BTreeMap<String, String>,
    clear_vars: Vec<&str>,
    settings_dirs: Vec<&str>,
    skill_dirs: Vec<&str>,
    launch_command: Option<&str>,
) -> TakeoverProfile {
    TakeoverProfile {
        id: id.to_string(),
        name: name.to_string(),
        vendor: vendor.to_string(),
        description: description.to_string(),
        protocol: protocol.to_string(),
        env_vars,
        clear_vars: clear_vars.into_iter().map(|s| s.to_string()).collect(),
        settings_dirs: settings_dirs.into_iter().map(|s| s.to_string()).collect(),
        settings_path: None,
        skill_dirs: skill_dirs.into_iter().map(|s| s.to_string()).collect(),
        launch_command: launch_command.map(|s| s.to_string()),
        builtin: true,
    }
}

fn btreemap(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn load_store(config_dir: &Path) -> ProfileStore {
    let path = config_dir.join(CUSTOM_FILE);
    if !path.exists() {
        return ProfileStore::default();
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    // 兼容旧版：纯数组
    if let Ok(list) = serde_json::from_str::<Vec<TakeoverProfile>>(&raw) {
        return ProfileStore {
            profiles: list,
            deleted_builtin_ids: Vec::new(),
        };
    }
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_store(config_dir: &Path, store: &ProfileStore) -> Result<()> {
    let path = config_dir.join(CUSTOM_FILE);
    fs::create_dir_all(config_dir)?;
    fs::write(path, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

pub fn load_custom_profiles(config_dir: &Path) -> Vec<TakeoverProfile> {
    load_store(config_dir).profiles
}

pub fn save_custom_profiles(config_dir: &Path, profiles: &[TakeoverProfile]) -> Result<()> {
    let mut store = load_store(config_dir);
    store.profiles = profiles.to_vec();
    save_store(config_dir, &store)
}

pub fn save_custom_profile(config_dir: &Path, profile: TakeoverProfile) -> Result<()> {
    upsert_profile(config_dir, profile)
}

pub fn upsert_profile(config_dir: &Path, profile: TakeoverProfile) -> Result<()> {
    let mut store = load_store(config_dir);
    // 重新添加已删除的内置模板时，清掉删除标记
    store
        .deleted_builtin_ids
        .retain(|x| !x.eq_ignore_ascii_case(&profile.id));
    store.profiles.retain(|p| !p.id.eq_ignore_ascii_case(&profile.id));
    store.profiles.push(profile);
    save_store(config_dir, &store)
}

pub fn delete_profile(config_dir: &Path, id: &str) -> Result<()> {
    let mut store = load_store(config_dir);
    let is_builtin_seed = builtin_profiles()
        .iter()
        .any(|p| p.id.eq_ignore_ascii_case(id));
    store.profiles.retain(|p| !p.id.eq_ignore_ascii_case(id));
    if is_builtin_seed {
        let key = id.to_ascii_lowercase();
        if !store
            .deleted_builtin_ids
            .iter()
            .any(|x| x.eq_ignore_ascii_case(&key))
        {
            store.deleted_builtin_ids.push(id.to_string());
        }
    }
    save_store(config_dir, &store)
}

fn merge_overlay(base: TakeoverProfile, overlay: TakeoverProfile) -> TakeoverProfile {
    TakeoverProfile {
        id: base.id,
        name: if overlay.name.trim().is_empty() {
            base.name
        } else {
            overlay.name
        },
        vendor: if overlay.vendor.trim().is_empty() {
            base.vendor
        } else {
            overlay.vendor
        },
        description: if overlay.description.trim().is_empty() {
            base.description
        } else {
            overlay.description
        },
        protocol: if overlay.protocol.trim().is_empty() {
            base.protocol
        } else {
            overlay.protocol
        },
        env_vars: if overlay.env_vars.is_empty() {
            base.env_vars
        } else {
            overlay.env_vars
        },
        clear_vars: if overlay.clear_vars.is_empty() {
            base.clear_vars
        } else {
            overlay.clear_vars
        },
        settings_dirs: if overlay.settings_dirs.is_empty() {
            base.settings_dirs
        } else {
            overlay.settings_dirs
        },
        settings_path: overlay.settings_path.or(base.settings_path),
        skill_dirs: if overlay.skill_dirs.is_empty() {
            base.skill_dirs
        } else {
            overlay.skill_dirs
        },
        launch_command: overlay.launch_command.or(base.launch_command),
        builtin: true,
    }
}

pub fn all_profiles(config_dir: &Path) -> Vec<TakeoverProfile> {
    let store = load_store(config_dir);
    let deleted: Vec<String> = store
        .deleted_builtin_ids
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let mut result = Vec::new();

    for base in builtin_profiles() {
        if deleted.iter().any(|d| d == &base.id.to_ascii_lowercase()) {
            continue;
        }
        if let Some(overlay) = store
            .profiles
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(&base.id))
        {
            result.push(merge_overlay(base, overlay.clone()));
        } else {
            result.push(base);
        }
    }

    for p in store.profiles {
        let is_builtin_id = builtin_profiles()
            .iter()
            .any(|b| b.id.eq_ignore_ascii_case(&p.id));
        if !is_builtin_id {
            let mut custom = p;
            custom.builtin = false;
            result.push(custom);
        }
    }
    result
}

pub fn find_profile(config_dir: &Path, id: &str) -> Option<TakeoverProfile> {
    all_profiles(config_dir)
        .into_iter()
        .find(|p| p.id.eq_ignore_ascii_case(id))
}

pub fn resolve_env(config: &AppConfig, profile: &TakeoverProfile) -> BTreeMap<String, String> {
    let anthropic_base = config.listen_url();
    let openai_base = format!("{}/v1", anthropic_base);
    let client_key = config.gateway_client_api_key.trim();
    let key = if client_key.is_empty() {
        "sugt-local-key"
    } else {
        client_key
    };

    profile
        .env_vars
        .iter()
        .map(|(name, template)| {
            let value = template
                .replace("{anthropic_base}", &anthropic_base)
                .replace("{openai_base}", &openai_base)
                .replace("{client_key}", key);
            (name.clone(), value)
        })
        .collect()
}

pub fn expand_home(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with("~/") || path.starts_with("~\\") {
        directories::UserDirs::new().map(|u| {
            u.home_dir().join(
                path.trim_start_matches('~')
                    .trim_start_matches('/')
                    .trim_start_matches('\\'),
            )
        })
    } else if path == "~" {
        directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
    } else {
        let p = Path::new(path);
        if p.is_absolute() {
            Some(p.to_path_buf())
        } else {
            // `.claude` 等相对路径按用户主目录解析
            directories::UserDirs::new().map(|u| u.home_dir().join(path))
        }
    }
}

pub fn resolved_settings_path(profile: &TakeoverProfile) -> Option<PathBuf> {
    if let Some(custom) = profile.settings_path.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
    {
        return expand_home(custom);
    }
    profile
        .settings_dirs
        .iter()
        .find_map(|dir| expand_home(dir).filter(|p| p.is_dir()))
}

pub fn detect_settings(profile: &TakeoverProfile) -> bool {
    resolved_settings_path(profile).is_some_and(|p| p.is_dir())
}

pub fn detect_skills(profile: &TakeoverProfile) -> bool {
    !collect_skill_dirs(profile).is_empty()
}

/// 统计目录下一层子目录中含 SKILL.md 的数量。
pub fn count_skills_in_dir(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir() && e.path().join("SKILL.md").is_file())
        .count()
}

/// 汇总该 Agent 应扫描的全部技能目录（去重，仅保留存在的目录）。
pub fn collect_skill_dirs(profile: &TakeoverProfile) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if !p.is_dir() {
            return;
        }
        if dirs.iter().any(|d| d == &p) {
            return;
        }
        dirs.push(p);
    };

    for dir in &profile.skill_dirs {
        if let Some(path) = expand_home(dir) {
            push_unique(path);
        }
    }
    if let Some(root) = resolved_settings_path(profile) {
        push_unique(root.join("skills"));
    }

    let id = profile.id.to_ascii_lowercase();
    if id.contains("opencode") || profile.name.to_ascii_lowercase().contains("opencode") {
        for extra in opencode_skill_dir_candidates() {
            push_unique(extra);
        }
    }
    if id.eq_ignore_ascii_case("claude-code") || id.eq_ignore_ascii_case("claude") {
        if let Ok(dir) = crate::store::paths::StorePaths::claude_skills_dir() {
            push_unique(dir);
        }
        for extra in claude_plugin_skill_dirs() {
            push_unique(extra);
        }
    }
    if id.eq_ignore_ascii_case("codex") {
        if let Ok(dir) = crate::store::paths::StorePaths::codex_skills_dir() {
            push_unique(dir);
        }
    }
    dirs
}

fn opencode_skill_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in [
        "~/.config/opencode/skills",
        "~/.opencode/skills",
        "~/.dev-agents/skills",
        "~/.claude/skills",
        "~/.agents/skills",
    ] {
        if let Some(p) = expand_home(raw) {
            out.push(p);
        }
    }
    // OpenCode / dev-agents：仓库内 skills 或 document-skills 等容器
    if let Some(repos) = expand_home("~/.dev-agents/repos") {
        if let Ok(entries) = fs::read_dir(repos) {
            for entry in entries.flatten() {
                let repo = entry.path();
                if !repo.is_dir() {
                    continue;
                }
                out.extend(discover_skill_container_dirs(&repo));
            }
        }
    }
    out
}

/// Claude 插件市场里挂的 skills（不存在则跳过，不影响无插件用户）
fn claude_plugin_skill_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(marketplaces) = expand_home("~/.claude/plugins/marketplaces") else {
        return out;
    };
    if !marketplaces.is_dir() {
        return out;
    }
    let Ok(vendors) = fs::read_dir(&marketplaces) else {
        return out;
    };
    for vendor in vendors.flatten() {
        let vendor_path = vendor.path();
        if !vendor_path.is_dir() {
            continue;
        }
        for group in ["plugins", "external_plugins"] {
            let group_dir = vendor_path.join(group);
            let Ok(plugins) = fs::read_dir(&group_dir) else {
                continue;
            };
            for plugin in plugins.flatten() {
                let skills = plugin.path().join("skills");
                if count_skills_in_dir(&skills) > 0 {
                    out.push(skills);
                }
            }
        }
    }
    out
}

/// 在目录下找出「下一层子目录含 SKILL.md」的容器（如 skills/、document-skills/）。
fn discover_skill_container_dirs(base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let skills = base.join("skills");
    if count_skills_in_dir(&skills) > 0 {
        out.push(skills);
    }
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && count_skills_in_dir(&path) > 0 {
                out.push(path);
            }
        }
    }
    out
}

pub fn count_profile_skills(_config_dir: &Path, profile: &TakeoverProfile) -> usize {
    collect_skill_dirs(profile)
        .iter()
        .map(|d| count_skills_in_dir(d))
        .sum()
}

pub fn count_profile_plugins(profile: &TakeoverProfile) -> usize {
    // Claude 系：~/.claude/plugins
    if profile.id.eq_ignore_ascii_case("claude-code")
        || profile.id.eq_ignore_ascii_case("claude")
        || profile.protocol.to_ascii_lowercase().contains("anthropic")
    {
        if let Ok(dir) = crate::store::paths::StorePaths::claude_plugins_dir() {
            if dir.is_dir() {
                return std::fs::read_dir(&dir)
                    .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
                    .unwrap_or(0);
            }
        }
    }
    0
}

/// 在 PATH 与常见安装位置查找 CLI，返回可执行路径。
pub fn resolve_cli_path(profile: &TakeoverProfile) -> Option<PathBuf> {
    let cmd = profile.launch_command.as_deref()?.trim();
    if cmd.is_empty() {
        return None;
    }
    if let Some(found) = find_command_on_path(cmd) {
        return Some(found);
    }
    // 常见安装位置兜底
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        if let Some(local) = local {
            candidates.push(local.join("Programs").join(cmd).join(format!("{cmd}.exe")));
            candidates.push(local.join(cmd).join(format!("{cmd}.exe")));
            // npm / bun 全局
            candidates.push(local.join("npm").join(format!("{cmd}.cmd")));
            candidates.push(local.join("npm").join(format!("{cmd}.exe")));
        }
        if let Some(appdata) = appdata {
            candidates.push(appdata.join("npm").join(format!("{cmd}.cmd")));
            candidates.push(appdata.join("npm").join(format!("{cmd}.exe")));
        }
        candidates.push(home.join("AppData").join("Roaming").join("npm").join(format!("{cmd}.cmd")));
        candidates.push(home.join(".local").join("bin").join(format!("{cmd}.exe")));
    }
    #[cfg(not(windows))]
    {
        candidates.push(home.join(".local").join("bin").join(cmd));
        candidates.push(PathBuf::from("/usr/local/bin").join(cmd));
        candidates.push(PathBuf::from("/opt/homebrew/bin").join(cmd));
        candidates.push(home.join(".npm-global").join("bin").join(cmd));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    crate::process_util::find_command_on_path(cmd)
}

pub fn detect_cli(profile: &TakeoverProfile) -> bool {
    resolve_cli_path(profile).is_some()
}

pub fn display_settings_path(profile: &TakeoverProfile) -> Option<String> {
    if let Some(custom) = profile
        .settings_path
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(custom);
    }
    resolved_settings_path(profile).map(|p| p.display().to_string())
}

pub fn set_settings_path(config_dir: &Path, id: &str, path: Option<String>) -> Result<TakeoverProfile> {
    let mut profile = find_profile(config_dir, id).ok_or_else(|| anyhow!("未找到模板：{}", id))?;
    let trimmed = path.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    profile.settings_path = trimmed.clone();
    if let Some(ref p) = trimmed {
        profile.settings_dirs = vec![p.clone()];
    }
    // 种子模板也写入 store 作为覆盖
    let to_save = profile.clone();
    upsert_profile(config_dir, to_save)?;
    find_profile(config_dir, id).ok_or_else(|| anyhow!("保存后未找到模板：{}", id))
}

pub fn is_env_active(profile: &TakeoverProfile, config: &AppConfig) -> bool {
    let expected = resolve_env(config, profile);
    if expected.is_empty() {
        return false;
    }
    expected.iter().all(|(name, value)| {
        std::env::var(name).map(|v| v == *value).unwrap_or(false)
    })
}

pub fn profile_view(config: &AppConfig, profile: &TakeoverProfile) -> TakeoverProfileView {
    profile_view_with_config_dir(None, config, profile)
}

pub fn profile_view_with_config_dir(
    config_dir: Option<&Path>,
    config: &AppConfig,
    profile: &TakeoverProfile,
) -> TakeoverProfileView {
    let preferred = config
        .active_takeover_ids
        .iter()
        .any(|id| id.eq_ignore_ascii_case(&profile.id));
    let cli_path = resolve_cli_path(profile);
    let cli_detected = cli_path.is_some();
    let settings_detected = detect_settings(profile);
    let skills_count = config_dir
        .map(|d| count_profile_skills(d, profile))
        .unwrap_or_else(|| {
            // 无 config_dir 时仍按 skill_dirs 尽力统计
            profile
                .skill_dirs
                .iter()
                .find_map(|dir| expand_home(dir))
                .map(|p| count_skills_in_dir(&p))
                .unwrap_or(0)
        });
    TakeoverProfileView {
        id: profile.id.clone(),
        name: profile.name.clone(),
        vendor: profile.vendor.clone(),
        description: profile.description.clone(),
        protocol: profile.protocol.clone(),
        configured: preferred,
        cli_detected,
        settings_detected,
        discovered: cli_detected || settings_detected,
        skills_detected: detect_skills(profile),
        settings_path: display_settings_path(profile),
        cli_path: cli_path.map(|p| p.display().to_string()),
        env_preview: resolve_env(config, profile),
        env_vars: profile.env_vars.clone(),
        clear_vars: profile.clear_vars.clone(),
        settings_dirs: profile.settings_dirs.clone(),
        skill_dirs: profile.skill_dirs.clone(),
        launch_command: profile.launch_command.clone(),
        builtin: profile.builtin,
        can_delete: true,
        skills_count,
        plugins_count: count_profile_plugins(profile),
    }
}

pub fn list_profile_views(config_dir: &Path, config: &AppConfig) -> Vec<TakeoverProfileView> {
    all_profiles(config_dir)
        .iter()
        .map(|p| profile_view_with_config_dir(Some(config_dir), config, p))
        .collect()
}

pub fn apply_profile(config_dir: &Path, config: &AppConfig, id: &str) -> Result<TakeoverProfileView> {
    let profile = find_profile(config_dir, id).ok_or_else(|| anyhow!("未找到接管模板：{}", id))?;
    let env = resolve_env(config, &profile);
    for name in &profile.clear_vars {
        let _ = crate::clients::unset_user_env_public(name);
    }
    for (name, value) in env {
        crate::clients::set_user_env_public(&name, &value)?;
    }
    if profile.id.contains("codex") || profile.launch_command.as_deref() == Some("codex") {
        let _ = crate::codex_config::apply_codex_config(config);
    }
    // 注入 SUGT 自带 skill，让 Agent 能调用 sugt-cli
    let targets = crate::sugt_skill::inject_targets_for_profile(&profile);
    let _ = crate::sugt_skill::inject_sugt_skill(&targets);
    Ok(profile_view_with_config_dir(Some(config_dir), config, &profile))
}

pub fn release_profile(config_dir: &Path, config: &AppConfig, id: &str) -> Result<TakeoverProfileView> {
    let profile = find_profile(config_dir, id).ok_or_else(|| anyhow!("未找到接管模板：{}", id))?;
    let env = resolve_env(config, &profile);
    for name in env.keys() {
        let _ = crate::clients::unset_user_env_public(name);
    }
    for name in &profile.clear_vars {
        let _ = crate::clients::unset_user_env_public(name);
    }
    if profile.id.contains("codex") || profile.launch_command.as_deref() == Some("codex") {
        let _ = crate::codex_config::clear_sugt_codex_overrides(config);
    }
    // 清理 SUGT skill（仅在没有任何其他 active 接管时才完全移除）
    let still_active = config
        .active_takeover_ids
        .iter()
        .filter(|other| other.as_str() != id)
        .count();
    if still_active == 0 {
        let targets = crate::sugt_skill::inject_targets_for_profile(&profile);
        let _ = crate::sugt_skill::remove_sugt_skill(&targets);
    }
    Ok(profile_view_with_config_dir(Some(config_dir), config, &profile))
}

pub fn restore_active_profiles(config_dir: &Path, config: &AppConfig) -> Result<()> {
    for id in &config.active_takeover_ids {
        let _ = apply_profile(config_dir, config, id);
    }
    Ok(())
}

pub fn clear_active_env_writes(config_dir: &Path, config: &AppConfig) -> Result<()> {
    for id in &config.active_takeover_ids {
        let _ = release_profile(config_dir, config, id);
    }
    // 旧版 Claude/Codex 一键写入也清掉
    let _ = crate::clients::uninstall(config_dir, config);
    Ok(())
}

/// 启发式解析配置文件（无大模型时）；返回建议的自定义 profile。
pub fn parse_config_file_heuristic(path: &Path, _config: &AppConfig) -> Result<TakeoverProfile> {
    let raw = fs::read_to_string(path)?;
    let lower = raw.to_lowercase();
    let mut env_vars = BTreeMap::new();
    if lower.contains("anthropic") {
        env_vars.insert("ANTHROPIC_BASE_URL".to_string(), "{anthropic_base}".to_string());
        env_vars.insert("ANTHROPIC_AUTH_TOKEN".to_string(), "{client_key}".to_string());
    }
    if lower.contains("openai") || lower.contains("api_key") {
        env_vars.insert("OPENAI_API_KEY".to_string(), "{client_key}".to_string());
        env_vars.insert("OPENAI_BASE_URL".to_string(), "{openai_base}".to_string());
    }
    if env_vars.is_empty() {
        env_vars.insert("OPENAI_API_KEY".to_string(), "{client_key}".to_string());
        env_vars.insert("OPENAI_BASE_URL".to_string(), "{openai_base}".to_string());
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("custom");
    let protocol = if lower.contains("anthropic") {
        "anthropic"
    } else {
        "openai"
    };
    Ok(TakeoverProfile {
        id: format!("custom-{}", stem),
        name: format!("自定义 · {}", stem),
        vendor: "Custom".to_string(),
        description: format!("自 {} 启发式解析", path.display()),
        protocol: protocol.to_string(),
        env_vars,
        clear_vars: vec![],
        settings_dirs: vec![],
        settings_path: None,
        skill_dirs: vec![],
        launch_command: None,
        builtin: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_include_claude_and_codex() {
        let ids: Vec<String> = builtin_profiles().into_iter().map(|p| p.id).collect();
        assert!(ids.contains(&"claude-code".to_string()));
        assert!(ids.contains(&"codex".to_string()));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn expand_home_resolves_dot_dirs_under_user_home() {
        let home = directories::UserDirs::new().unwrap().home_dir().to_path_buf();
        let got = expand_home(".claude").unwrap();
        assert_eq!(got, home.join(".claude"));
        let got2 = expand_home("~/.codex").unwrap();
        assert_eq!(got2, home.join(".codex"));
    }
}
