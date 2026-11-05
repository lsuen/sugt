//! 接入影响预览与 Agent 能力清单。
//!
//! 全部只读：不写环境变量、不改配置文件、不创建目录。前端在用户点「接入」之前
//! 用这里的结果把影响范围摊开，SOUL 也复用同一份能力摘要。
//! 作者：sun / 孙文龙

use crate::agent_adapters::{self, ConfigKind};
use crate::model::AppConfig;
use crate::takeover_profiles;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct AgentCapabilityView {
    pub id: String,
    pub name: String,
    pub config_kind: String,
    pub config_path: Option<String>,
    pub writable_fields: Vec<String>,
    pub skill_dirs: Vec<String>,
    pub note: String,
    pub revertable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigWritePreview {
    pub path: String,
    pub field: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TakeoverPreview {
    pub profile_id: String,
    pub profile_name: String,
    pub config_kind: String,
    /// 将写入的环境变量名（不含值，避免密钥出现在预览里）
    pub env_names: Vec<String>,
    /// 接管前会清除的冲突变量
    pub env_cleared: Vec<String>,
    /// 会修改的配置文件字段
    pub config_writes: Vec<ConfigWritePreview>,
    /// 会注入 / 挂载技能的目录
    pub skill_dirs: Vec<String>,
    pub warnings: Vec<String>,
    pub note: String,
    pub revertable: bool,
}

fn kind_label(kind: ConfigKind) -> &'static str {
    match kind {
        ConfigKind::UserEnv => "user_env",
        ConfigKind::CodexToml => "codex_toml",
        ConfigKind::None => "none",
    }
}

/// 供设置页与 SOUL 使用的能力清单：只声明 SUGT 真正会写的东西。
pub fn list_capabilities() -> Vec<AgentCapabilityView> {
    agent_adapters::all()
        .iter()
        .map(|adapter| AgentCapabilityView {
            id: adapter.id.to_string(),
            name: adapter.name.to_string(),
            config_kind: kind_label(adapter.config.kind).to_string(),
            config_path: adapter.config.config_path.map(|s| s.to_string()),
            writable_fields: adapter
                .config
                .writable_fields
                .iter()
                .map(|s| s.to_string())
                .collect(),
            skill_dirs: adapter.skill_dirs.iter().map(|s| s.to_string()).collect(),
            note: adapter.config.note.to_string(),
            revertable: adapter.config.revertable,
        })
        .collect()
}

pub fn build_preview(config_dir: &Path, config: &AppConfig, id: &str) -> Result<TakeoverPreview> {
    let profile = takeover_profiles::find_profile(config_dir, id)
        .ok_or_else(|| anyhow!("未找到接管模板：{}", id))?;
    let capability = agent_adapters::config_capability(&profile.id);
    let env = takeover_profiles::resolve_env(config, &profile);
    let env_names: Vec<String> = env.keys().cloned().collect();

    let mut cleared: BTreeSet<String> = profile.clear_vars.iter().cloned().collect();
    cleared.extend(capability.clear_vars.iter().map(|s| s.to_string()));
    for name in &env_names {
        cleared.remove(name);
    }

    let mut config_writes = Vec::new();
    if capability.kind == ConfigKind::CodexToml {
        for field in capability.writable_fields {
            let path = if field.starts_with("auth.json.") {
                "~/.codex/auth.json"
            } else {
                capability.config_path.unwrap_or("~/.codex/config.toml")
            };
            config_writes.push(ConfigWritePreview {
                path: path.to_string(),
                field: field.trim_start_matches("auth.json.").to_string(),
            });
        }
    }

    let mut warnings = Vec::new();
    if agent_adapters::by_id(&profile.id).is_none() {
        warnings.push("自定义客户端未声明能力，请核对环境变量与技能目录后再接入".to_string());
    }
    if !capability.revertable {
        warnings.push("该客户端暂不支持自动回滚，请自行记录原有配置".to_string());
    }
    if capability.kind == ConfigKind::CodexToml {
        warnings.push(
            "MCP server 由客户端自己的 [mcp_servers] 配置管理，SUGT 不代管；read_mcp_resource 等工具能否使用取决于客户端侧是否已启动 server"
                .to_string(),
        );
        warnings.push(
            "客户端看到的模型名会加 sugt- 前缀，避免继承官方模型的 code_mode_only / use_responses_lite 工具协议；若这版 Codex 对未知 slug 表现异常，设环境变量 SUGT_CODEX_MODEL_ALIAS=off 后重新接入即可回退"
                .to_string(),
        );
        if let Some(raw) = capability.config_path.and_then(agent_adapters::expand_home) {
            if raw.is_file() {
                if let Ok(content) = std::fs::read_to_string(&raw) {
                    if toml::from_str::<toml::Table>(&content).is_err() {
                        warnings.push(format!(
                            "{} 不是合法 TOML，接入会被拒绝；请先修复该文件",
                            raw.display()
                        ));
                    }
                }
            }
        }
    }

    let mut skill_dirs: Vec<String> = takeover_profiles::collect_skill_dirs(&profile)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if skill_dirs.is_empty() {
        if let Ok(dir) = agent_adapters::canonical_skill_dir(&profile.id) {
            warnings.push(format!("技能目录尚不存在，接入时会创建 {}", dir.display()));
            skill_dirs.push(dir.display().to_string());
        }
    }

    // 与其它已接管客户端抢同一个环境变量时提前提示。
    for other_id in &config.active_takeover_ids {
        if other_id.eq_ignore_ascii_case(id) {
            continue;
        }
        let Some(other) = takeover_profiles::find_profile(config_dir, other_id) else {
            continue;
        };
        for (name, value) in takeover_profiles::resolve_env(config, &other) {
            if env.get(&name).is_some_and(|mine| *mine != value) {
                warnings.push(format!(
                    "与已接管的「{}」在环境变量 {} 上取值冲突",
                    other.name, name
                ));
            }
        }
    }

    Ok(TakeoverPreview {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        config_kind: kind_label(capability.kind).to_string(),
        env_names,
        env_cleared: cleared.into_iter().collect(),
        config_writes,
        skill_dirs,
        warnings,
        note: capability.note.to_string(),
        revertable: capability.revertable,
    })
}

/// 注入 SOUL system_prompt 的能力摘要：让模型掌握事实，而不是靠 bash 猜路径。
pub fn capability_brief(zh: bool) -> String {
    if zh {
        concat!(
            "本机 Agent 接入能力（程序已声明，不要凭记忆猜路径）：\n",
            "- Claude Code：只写环境变量 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN；技能目录 ~/.claude/skills\n",
            "- Codex：写 ~/.codex/config.toml（model_providers.sugt、wire_api=responses、requires_openai_auth=false、本地压缩、web_search=disabled、模型别名 sugt- 前缀）与 auth.json（保留原登录态）；别名避免客户端把上游模型当官方模型套用 code_mode_only 工具协议，可用环境变量 SUGT_CODEX_MODEL_ALIAS=off 关闭；不改 mcp_servers，MCP server 由客户端自管；技能目录 ~/.agents/skills；接管前有备份，取消可还原\n",
            "- OpenCode / Qwen Code / Aider / Gemini CLI / Cursor / VS Code：只写 OPENAI_* 环境变量，技能目录按各自候选探测\n",
            "接入与取消接管必须由用户在「客户端」页点按钮完成；你只解释影响并给建议，绝不自己改这些客户端配置文件。"
        )
        .to_string()
    } else {
        concat!(
            "Local agent takeover capabilities (declared by the app; do not guess paths):\n",
            "- Claude Code: user env ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN only; skills at ~/.claude/skills\n",
            "- Codex: writes ~/.codex/config.toml (model_providers.sugt, wire_api=responses, requires_openai_auth=false, local compaction, web_search=disabled, sugt- model alias) and auth.json while preserving existing login fields; the alias prevents the client from inheriting the official code_mode_only tool protocol (disable with SUGT_CODEX_MODEL_ALIAS=off); never touches mcp_servers, MCP servers are managed by the client; skills at ~/.agents/skills; backed up before takeover and revertable\n",
            "- OpenCode / Qwen Code / Aider / Gemini CLI / Cursor / VS Code: OPENAI_* env vars only; skill directories are probed per client\n",
            "Takeover and release are performed only when the user clicks the buttons on the Clients tab. Explain the impact and advise, but never edit those client config files yourself."
        )
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_list_has_no_unknown_or_empty_entries() {
        let list = list_capabilities();
        assert_eq!(list.len(), agent_adapters::all().len());
        for item in list {
            assert!(!item.id.is_empty());
            assert!(!item.skill_dirs.is_empty());
            assert!(!item.note.is_empty());
        }
    }

    #[test]
    fn codex_capability_exposes_toml_writes() {
        let codex = list_capabilities()
            .into_iter()
            .find(|c| c.id == "codex")
            .unwrap();
        assert_eq!(codex.config_kind, "codex_toml");
        assert_eq!(codex.config_path.as_deref(), Some("~/.codex/config.toml"));
        assert!(codex.writable_fields.iter().any(|f| f.contains("wire_api")));
        assert!(codex.revertable);
    }

    #[test]
    fn brief_is_bilingual_and_forbids_self_editing() {
        assert!(capability_brief(true).contains("绝不自己改"));
        assert!(capability_brief(false).contains("never edit"));
    }

    #[test]
    fn preview_rejects_unknown_profile() {
        let dir = std::env::temp_dir().join(format!("sugt-preview-{}", std::process::id()));
        let config = AppConfig::default();
        let err = build_preview(&dir, &config, "no-such-profile").unwrap_err();
        assert!(err.to_string().contains("未找到接管模板"));
    }

    #[test]
    fn builtin_claude_preview_never_leaks_key_values() {
        let dir = std::env::temp_dir().join(format!("sugt-preview-claude-{}", std::process::id()));
        let config = AppConfig::default();
        let preview = build_preview(&dir, &config, "claude-code").unwrap();
        assert_eq!(preview.config_kind, "user_env");
        assert!(preview
            .env_names
            .contains(&"ANTHROPIC_AUTH_TOKEN".to_string()));
        assert!(preview.config_writes.is_empty());
        assert!(preview.revertable);
        // 预览只含变量名，绝不能出现密钥值。
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(!serialized.contains("sugt-local-key"));
        assert!(!serialized.contains(&config.gateway_client_api_key));
    }
}
