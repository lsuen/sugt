//! 本机 Agent 能力矩阵与元数据的唯一真相源。
//!
//! 检测指纹、配置能力、技能目录只在这里声明；`agent_discover.rs`、
//! `takeover_profiles.rs`、`store/paths.rs`、`sugt_skill.rs` 全部从这里派生，
//! 避免同一路径在四处硬编码后互相打架。
//! 作者：sun / 孙文龙

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// SUGT 当前实际使用的配置写入方式。
///
/// 这里描述的是「SUGT 会写什么」，不是「该 Agent 官方支持哪些配置项」；
/// 探测到配置文件但不写入的客户端仍归为 `UserEnv`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    /// 用户级环境变量（平台层写 shell rc / 注册表）
    UserEnv,
    /// Codex 专用：`~/.codex/config.toml` + `auth.json`
    CodexToml,
    /// 暂无写入能力
    None,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigCapability {
    pub kind: ConfigKind,
    /// 相对用户主目录的配置文件路径，仅声明 SUGT 可写的目标
    pub config_path: Option<&'static str>,
    /// SUGT 接管时实际会写入的字段名（用于接入影响预览，不含值）
    pub writable_fields: &'static [&'static str],
    /// 接管前必须清除的冲突变量
    pub clear_vars: &'static [&'static str],
    /// 释放接管时能否完整还原到接管前状态
    pub revertable: bool,
    /// 面向用户的说明，会展示在接入预览里
    pub note: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct AgentAdapter {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub protocol: &'static str,
    /// PATH 上的可执行文件名
    pub commands: &'static [&'static str],
    /// 配置目录候选（用于发现，不代表会写入）
    pub settings_dirs: &'static [&'static str],
    /// 个人级技能目录候选，按顺序优先；第一个为 canonical
    pub skill_dirs: &'static [&'static str],
    /// 项目级技能目录（相对仓库根，仅用于展示与说明）
    pub project_skill_dirs: &'static [&'static str],
    pub config: ConfigCapability,
}

const ENV_CAPABILITY: ConfigCapability = ConfigCapability {
    kind: ConfigKind::UserEnv,
    config_path: None,
    writable_fields: &["OPENAI_API_KEY", "OPENAI_BASE_URL"],
    clear_vars: &[],
    revertable: true,
    note: "仅写用户环境变量，不改客户端配置文件；释放时清除变量即可还原",
};

/// 尚未确认官方 skill 目录的客户端：统一声明为「仅写环境变量 + 通用目录兜底」。
/// 候选目录里始终包含跨 Agent 事实标准 `~/.agents/skills`，专有目录探测不到时不会无处挂载。
const GENERIC_CAPABILITY: ConfigCapability = ConfigCapability {
    note: "未确认官方 skill 目录，按候选探测并回落到跨 Agent 通用目录；仅写用户环境变量",
    ..ENV_CAPABILITY
};

/// 未在表内的自定义客户端：声明为「能力未知」，绝不虚报可写配置。
const UNKNOWN_CAPABILITY: ConfigCapability = ConfigCapability {
    kind: ConfigKind::UserEnv,
    config_path: None,
    writable_fields: &[],
    clear_vars: &[],
    revertable: false,
    note: "自定义客户端：能力未声明，请核对环境变量与技能目录后再接入",
};

static ADAPTERS: &[AgentAdapter] = &[
    AgentAdapter {
        id: "claude-code",
        name: "Claude Code",
        vendor: "Anthropic",
        protocol: "anthropic",
        commands: &["claude"],
        settings_dirs: &["~/.claude"],
        skill_dirs: &["~/.claude/skills"],
        project_skill_dirs: &[".claude/skills"],
        config: ConfigCapability {
            kind: ConfigKind::UserEnv,
            config_path: None,
            writable_fields: &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"],
            clear_vars: &["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"],
            revertable: true,
            note: "仅写用户环境变量，不改 Claude Code 配置文件；同时清除会冲突的 API Key 变量",
        },
    },
    AgentAdapter {
        id: "codex",
        name: "OpenAI Codex CLI",
        vendor: "OpenAI",
        protocol: "openai",
        commands: &["codex"],
        settings_dirs: &["~/.codex"],
        // 公开资料对 ~/.codex/skills 与 ~/.agents/skills 说法不一致，
        // 两者都作为候选按存在性探测；canonical 保持现有的 ~/.agents/skills。
        skill_dirs: &["~/.agents/skills", "~/.codex/skills"],
        project_skill_dirs: &[".agents/skills"],
        config: ConfigCapability {
            kind: ConfigKind::CodexToml,
            config_path: Some("~/.codex/config.toml"),
            writable_fields: &[
                "model_provider",
                "model",
                "openai_base_url",
                "preferred_auth_method",
                "cli_auth_credentials_store",
                "web_search",
                "model_providers.sugt.name",
                "model_providers.sugt.base_url",
                "model_providers.sugt.wire_api",
                "model_providers.sugt.requires_openai_auth",
                "model_providers.sugt.capabilities.remote_compaction",
                "model_providers.sugt.capabilities.external_web_access",
                "auth.json.auth_mode",
                "auth.json.OPENAI_API_KEY",
            ],
            clear_vars: &[],
            revertable: true,
            note: "写 config.toml 与 auth.json（保留原有登录态字段）；model 使用 sugt- 前缀别名，可用 SUGT_CODEX_MODEL_ALIAS=off 关闭；声明第三方能力与本地压缩；不改 mcp_servers，MCP server 由客户端自行配置；接管前备份并快照，释放时完整还原",
        },
    },
    AgentAdapter {
        id: "opencode",
        name: "OpenCode",
        vendor: "OpenCode",
        protocol: "openai",
        commands: &["opencode"],
        settings_dirs: &["~/.opencode", "~/.config/opencode"],
        skill_dirs: &[
            "~/.config/opencode/skills",
            "~/.opencode/skills",
            "~/.dev-agents/skills",
            "~/.claude/skills",
            "~/.agents/skills",
        ],
        project_skill_dirs: &[".opencode/skills", ".agents/skills"],
        config: ConfigCapability { ..ENV_CAPABILITY },
    },
    AgentAdapter {
        id: "qwen-code",
        name: "Qwen Code",
        vendor: "Alibaba",
        protocol: "openai",
        commands: &["qwen", "qwencode"],
        settings_dirs: &["~/.qwen", "~/.qwencode"],
        skill_dirs: &["~/.agents/skills"],
        project_skill_dirs: &[".agents/skills"],
        config: ConfigCapability {
            note: "未确认原生 skill 目录，按通用 .agents/skills 挂载；仅写用户环境变量",
            ..ENV_CAPABILITY
        },
    },
    AgentAdapter {
        id: "aider",
        name: "Aider",
        vendor: "Aider",
        protocol: "openai",
        commands: &["aider"],
        settings_dirs: &["~/.aider"],
        skill_dirs: &["~/.agents/skills"],
        project_skill_dirs: &[".agents/skills"],
        config: ConfigCapability {
            note: "未确认原生 skill 目录，按通用 .agents/skills 挂载；仅写用户环境变量",
            ..ENV_CAPABILITY
        },
    },
    AgentAdapter {
        id: "gemini-cli",
        name: "Gemini CLI",
        vendor: "Google",
        protocol: "openai",
        commands: &["gemini"],
        settings_dirs: &["~/.gemini"],
        skill_dirs: &["~/.gemini/skills", "~/.agents/skills"],
        project_skill_dirs: &[".agents/skills"],
        config: ConfigCapability { ..ENV_CAPABILITY },
    },
    AgentAdapter {
        id: "cursor",
        name: "Cursor",
        vendor: "Cursor",
        protocol: "openai",
        commands: &["cursor"],
        settings_dirs: &["~/.cursor"],
        skill_dirs: &["~/.cursor/skills", "~/.agents/skills", "~/.claude/skills"],
        project_skill_dirs: &[".cursor/skills", ".agents/skills"],
        config: ConfigCapability {
            note: "纯编辑器：需补配置/技能目录；仅写用户环境变量",
            ..ENV_CAPABILITY
        },
    },
    AgentAdapter {
        id: "vscode",
        name: "VS Code",
        vendor: "Microsoft",
        protocol: "openai",
        commands: &["code"],
        settings_dirs: &[],
        skill_dirs: &["~/.agents/skills", "~/.claude/skills"],
        project_skill_dirs: &[".github/skills", ".agents/skills"],
        config: ConfigCapability {
            note: "纯编辑器：需补配置/技能目录；仅写用户环境变量",
            ..ENV_CAPABILITY
        },
    },
    // 国内常见 Agent / IDE 客户端（2025-2026）。
    // 未确认官方 skill 目录的一律走 GENERIC_CAPABILITY，专有目录只是探测候选。
    AgentAdapter {
        id: "codebuddy",
        name: "CodeBuddy",
        vendor: "Tencent",
        protocol: "openai",
        commands: &["codebuddy"],
        settings_dirs: &["~/.codebuddy", "~/.config/CodeBuddy"],
        skill_dirs: &["~/.codebuddy/skills", "~/.agents/skills"],
        project_skill_dirs: &[".codebuddy/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "trae",
        name: "Trae",
        vendor: "ByteDance",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.trae", "~/.config/Trae"],
        skill_dirs: &["~/.trae/skills", "~/.agents/skills"],
        project_skill_dirs: &[".trae/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "lingma",
        name: "通义灵码 Lingma",
        vendor: "Alibaba",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.lingma", "~/.config/Lingma"],
        skill_dirs: &["~/.lingma/skills", "~/.agents/skills"],
        project_skill_dirs: &[".lingma/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "iflow",
        name: "iFlow CLI",
        vendor: "iFlow",
        protocol: "openai",
        commands: &["iflow"],
        settings_dirs: &["~/.iflow"],
        skill_dirs: &["~/.iflow/skills", "~/.agents/skills"],
        project_skill_dirs: &[".iflow/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "kimi",
        name: "Kimi CLI",
        vendor: "Moonshot AI",
        protocol: "openai",
        commands: &["kimi"],
        settings_dirs: &["~/.kimi"],
        skill_dirs: &["~/.kimi/skills", "~/.agents/skills"],
        project_skill_dirs: &[".kimi/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "qoder",
        name: "Qoder",
        vendor: "Alibaba",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.qoder", "~/.config/Qoder"],
        skill_dirs: &["~/.qoder/skills", "~/.agents/skills"],
        project_skill_dirs: &[".qoder/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "comate",
        name: "百度 Comate",
        vendor: "Baidu",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.comate"],
        skill_dirs: &["~/.comate/skills", "~/.agents/skills"],
        project_skill_dirs: &[".comate/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "codegeex",
        name: "CodeGeeX",
        vendor: "Zhipu AI",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.codegeex"],
        skill_dirs: &["~/.codegeex/skills", "~/.agents/skills"],
        project_skill_dirs: &[".codegeex/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    // 国外常见 Agent / IDE 客户端。
    AgentAdapter {
        id: "windsurf",
        name: "Windsurf",
        vendor: "Codeium",
        protocol: "openai",
        commands: &[],
        settings_dirs: &["~/.windsurf", "~/.codeium"],
        skill_dirs: &["~/.windsurf/skills", "~/.agents/skills"],
        project_skill_dirs: &[".windsurf/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "cline",
        name: "Cline",
        vendor: "Cline",
        protocol: "openai",
        commands: &["cline"],
        settings_dirs: &["~/.cline"],
        skill_dirs: &["~/.cline/skills", "~/.agents/skills"],
        project_skill_dirs: &[".cline/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "copilot-cli",
        name: "GitHub Copilot CLI",
        vendor: "GitHub",
        protocol: "openai",
        commands: &["copilot"],
        settings_dirs: &["~/.copilot"],
        skill_dirs: &["~/.copilot/skills", "~/.github/skills", "~/.agents/skills"],
        project_skill_dirs: &[".github/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "kiro",
        name: "Kiro",
        vendor: "AWS",
        protocol: "openai",
        commands: &["kiro"],
        settings_dirs: &["~/.kiro"],
        skill_dirs: &["~/.kiro/skills", "~/.agents/skills"],
        project_skill_dirs: &[".kiro/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "goose",
        name: "Goose",
        vendor: "Block",
        protocol: "openai",
        commands: &["goose"],
        settings_dirs: &["~/.config/goose"],
        skill_dirs: &["~/.config/goose/skills", "~/.agents/skills"],
        project_skill_dirs: &[".goose/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
    AgentAdapter {
        id: "droid",
        name: "Factory Droid",
        vendor: "Factory",
        protocol: "openai",
        commands: &["droid"],
        settings_dirs: &["~/.factory"],
        skill_dirs: &["~/.factory/skills", "~/.agents/skills"],
        project_skill_dirs: &[".factory/skills", ".agents/skills"],
        config: ConfigCapability { ..GENERIC_CAPABILITY },
    },
];

pub fn all() -> &'static [AgentAdapter] {
    ADAPTERS
}

pub fn by_id(id: &str) -> Option<&'static AgentAdapter> {
    let key = id.trim().to_ascii_lowercase();
    ADAPTERS.iter().find(|a| a.id.eq_ignore_ascii_case(&key))
}

pub fn by_command(command: &str) -> Option<&'static AgentAdapter> {
    let key = command.trim().to_ascii_lowercase();
    ADAPTERS
        .iter()
        .find(|a| a.commands.iter().any(|c| c.eq_ignore_ascii_case(&key)))
}

/// 未知客户端返回「能力未声明」，而不是伪装成已知 Agent。
pub fn config_capability(id: &str) -> ConfigCapability {
    by_id(id).map(|a| a.config).unwrap_or(UNKNOWN_CAPABILITY)
}

/// 用户主目录解析的唯一实现（~/、~、相对点目录、绝对路径）。
pub fn expand_home(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.starts_with("~/") || path.starts_with("~\\") {
        return home_dir().map(|h| {
            h.join(
                path.trim_start_matches('~')
                    .trim_start_matches('/')
                    .trim_start_matches('\\'),
            )
        });
    }
    if path == "~" {
        return home_dir();
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        Some(candidate.to_path_buf())
    } else {
        home_dir().map(|h| h.join(path))
    }
}

fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

/// 按声明顺序展开该 Agent 的个人级技能目录候选（不检查是否存在）。
pub fn skill_dir_candidates(id: &str) -> Vec<PathBuf> {
    match by_id(id) {
        Some(adapter) => adapter
            .skill_dirs
            .iter()
            .filter_map(|raw| expand_home(raw))
            .collect(),
        // 跨 Agent 通用目录：自定义客户端至少有一个明确落点
        None => expand_home("~/.agents/skills").into_iter().collect(),
    }
}

/// 该 Agent 的技能注入主目录。未知客户端回落到跨 Agent 通用目录。
pub fn canonical_skill_dir(id: &str) -> Result<PathBuf> {
    skill_dir_candidates(id)
        .into_iter()
        .next()
        .context("无法解析 Agent 技能目录")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_ids_are_unique() {
        let mut ids: Vec<&str> = all().iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "AgentAdapter id 重复");
    }

    #[test]
    fn every_adapter_has_a_skill_dir_and_a_capability() {
        for adapter in all() {
            assert!(
                !adapter.skill_dirs.is_empty(),
                "{} 未声明技能目录",
                adapter.id
            );
            assert_ne!(
                adapter.config.kind,
                ConfigKind::None,
                "{} 未声明配置能力",
                adapter.id
            );
            assert!(
                !adapter.config.note.is_empty(),
                "{} 缺少面向用户的说明",
                adapter.id
            );
        }
    }

    #[test]
    fn codex_is_the_only_toml_writer() {
        let toml_writers: Vec<&str> = all()
            .iter()
            .filter(|a| a.config.kind == ConfigKind::CodexToml)
            .map(|a| a.id)
            .collect();
        assert_eq!(toml_writers, vec!["codex"]);
        assert_eq!(
            config_capability("codex").config_path,
            Some("~/.codex/config.toml")
        );
    }

    #[test]
    fn unknown_agent_does_not_claim_capabilities() {
        let capability = config_capability("discovered-my-agent");
        assert!(capability.writable_fields.is_empty());
        assert!(!capability.revertable);
        assert_eq!(capability.kind, ConfigKind::UserEnv);
    }

    #[test]
    fn canonical_dirs_match_legacy_store_paths() {
        let home = home_dir().unwrap();
        assert_eq!(
            canonical_skill_dir("claude-code").unwrap(),
            home.join(".claude").join("skills")
        );
        assert_eq!(
            canonical_skill_dir("codex").unwrap(),
            home.join(".agents").join("skills")
        );
        assert_eq!(
            canonical_skill_dir("unknown").unwrap(),
            home.join(".agents").join("skills")
        );
    }

    #[test]
    fn catalog_covers_domestic_and_global_clients() {
        let ids: Vec<&str> = all().iter().map(|a| a.id).collect();
        for id in [
            "codebuddy", "trae", "lingma", "iflow", "kimi", "qoder", "comate", "codegeex",
            "windsurf", "cline", "copilot-cli", "kiro", "goose", "droid",
        ] {
            assert!(ids.contains(&id), "能力矩阵缺少 {id}");
        }
    }

    #[test]
    fn unconfirmed_skill_dirs_always_have_a_shared_fallback() {
        for adapter in all()
            .iter()
            .filter(|a| a.config.note.contains("未确认官方 skill 目录"))
        {
            assert!(
                adapter
                    .skill_dirs
                    .iter()
                    .any(|dir| *dir == "~/.agents/skills"),
                "{} 缺少跨 Agent 通用兜底目录",
                adapter.id
            );
            assert_eq!(
                adapter.config.kind,
                ConfigKind::UserEnv,
                "{} 未确认能力，不能声称写配置文件",
                adapter.id
            );
        }
    }

    #[test]
    fn expand_home_handles_all_path_forms() {
        let home = home_dir().unwrap();
        assert_eq!(expand_home("~/.codex"), Some(home.join(".codex")));
        assert_eq!(expand_home("~"), Some(home.clone()));
        assert_eq!(expand_home(".claude"), Some(home.join(".claude")));
        assert_eq!(expand_home(""), None);
        // Windows 上 "/tmp/x" 没有盘符，会被当成相对路径回落到主目录；
        // 因此绝对路径用例必须用当前平台的临时目录构造，不能写 Unix 字面量。
        let absolute = std::env::temp_dir().join("sugt-absolute-x");
        let absolute_text = absolute.to_string_lossy().to_string();
        assert_eq!(expand_home(&absolute_text), Some(absolute));
    }
}
