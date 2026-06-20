use crate::model::AppConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

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
    #[serde(default)]
    pub skill_dirs: Vec<String>,
    #[serde(default)]
    pub launch_command: Option<String>,
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TakeoverProfileView {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub protocol: String,
    pub configured: bool,
    pub settings_detected: bool,
    pub skills_detected: bool,
    pub env_preview: BTreeMap<String, String>,
    pub builtin: bool,
}

pub fn builtin_profiles() -> Vec<TakeoverProfile> {
    vec![
        profile(
            "claude-code",
            "Claude Code",
            "Anthropic",
            "官方环境变量：ANTHROPIC_BASE_URL + ANTHROPIC_AUTH_TOKEN（勿设 API_KEY）",
            "anthropic",
            btreemap(&[
                ("ANTHROPIC_BASE_URL", "{anthropic_base}"),
                ("ANTHROPIC_AUTH_TOKEN", "{client_key}"),
            ]),
            vec!["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"],
            vec![".claude"],
            vec![".claude/skills"],
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
            vec![".codex"],
            vec![],
            Some("codex"),
        ),
        profile(
            "opencode",
            "OpenCode",
            "OpenAI Compatible",
            "开源终端 Agent，走 OpenAI 兼容端点",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            vec![],
            vec![".config/opencode"],
            vec![".opencode/skills"],
            Some("opencode"),
        ),
        profile(
            "gemini-cli",
            "Gemini CLI",
            "Google",
            "通过 OpenAI 兼容层接入时需 OPENAI 变量；原生 Gemini 请直连 Google",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            vec![],
            vec![".gemini"],
            vec![],
            Some("gemini"),
        ),
        profile(
            "aider",
            "Aider",
            "OpenAI Compatible",
            "AIDER 常用 OPENAI_API_BASE / OPENAI_API_KEY",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_API_BASE", "{openai_base}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            vec![],
            vec![".aider"],
            vec![],
            Some("aider"),
        ),
        profile(
            "openai-agents",
            "OpenAI Agents SDK",
            "OpenAI",
            "Python/TS Agents SDK 读取 OPENAI_API_KEY + OPENAI_BASE_URL",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            vec![],
            vec![],
            vec![],
            None,
        ),
        profile(
            "cline",
            "Cline / Roo Code",
            "OpenAI Compatible",
            "VS Code 扩展类 Agent，OpenAI 兼容 Base URL",
            "openai",
            btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            vec![],
            vec![".cline"],
            vec![],
            None,
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

pub fn load_custom_profiles(config_dir: &Path) -> Vec<TakeoverProfile> {
    let path = config_dir.join(CUSTOM_FILE);
    if !path.exists() {
        return Vec::new();
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str::<Vec<TakeoverProfile>>(&raw).unwrap_or_default()
}

pub fn save_custom_profiles(config_dir: &Path, profiles: &[TakeoverProfile]) -> Result<()> {
    let path = config_dir.join(CUSTOM_FILE);
    let json = serde_json::to_string_pretty(profiles)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn all_profiles(config_dir: &Path) -> Vec<TakeoverProfile> {
    let mut all = builtin_profiles();
    all.extend(load_custom_profiles(config_dir));
    all
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

pub fn expand_home(path: &str) -> Option<std::path::PathBuf> {
    if path.starts_with("~/") || path.starts_with("~\\") {
        directories::UserDirs::new().map(|u| u.home_dir().join(path.trim_start_matches('~').trim_start_matches('/').trim_start_matches('\\')))
    } else if path == "~" {
        directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
    } else {
        Some(Path::new(path).to_path_buf())
    }
}

pub fn detect_settings(profile: &TakeoverProfile) -> bool {
    profile.settings_dirs.iter().any(|dir| {
        expand_home(dir).is_some_and(|p| p.is_dir())
    })
}

pub fn detect_skills(profile: &TakeoverProfile) -> bool {
    profile.skill_dirs.iter().any(|dir| {
        expand_home(dir).is_some_and(|p| p.is_dir())
    })
}

pub fn is_configured(profile: &TakeoverProfile, config: &AppConfig) -> bool {
    let expected = resolve_env(config, profile);
    expected.iter().all(|(name, value)| {
        std::env::var(name).map(|v| v == *value).unwrap_or(false)
    })
}

pub fn profile_view(config_dir: &Path, config: &AppConfig, profile: &TakeoverProfile) -> TakeoverProfileView {
    TakeoverProfileView {
        id: profile.id.clone(),
        name: profile.name.clone(),
        vendor: profile.vendor.clone(),
        description: profile.description.clone(),
        protocol: profile.protocol.clone(),
        configured: is_configured(profile, config),
        settings_detected: detect_settings(profile),
        skills_detected: detect_skills(profile),
        env_preview: resolve_env(config, profile),
        builtin: profile.builtin,
    }
}

pub fn list_profile_views(config_dir: &Path, config: &AppConfig) -> Vec<TakeoverProfileView> {
    all_profiles(config_dir)
        .iter()
        .map(|p| profile_view(config_dir, config, p))
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
    Ok(profile_view(config_dir, config, &profile))
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
        assert!(ids.len() >= 7);
    }
}
