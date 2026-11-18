//! 请求改写规则（PRD v1.1.11 R6）
//!
//! 把 Codex 的 max_tokens / tool 重试等硬编码改为可配置规则：
//! 规则按作用域匹配客户端，动作按作用域顺序（Default → Codex → Agent → Global）
//! 依次应用，后者覆盖前者的同名 key。
//!
//! 持久化文件 `config_dir/request-rules.json`，结构 `{ "version": 1, "rules": [...] }`，
//! 与前端统一 camelCase。文件缺失或解析失败时回退到 [`builtin_rules`]，绝不 panic。

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 规则文件名（位于 config_dir 下）。
const RULES_FILE: &str = "request-rules.json";
const RULES_VERSION: u32 = 1;

/// 内置规则 id，前端据此识别「内置规则」，删除时改为禁用而不是移除。
pub const BUILTIN_CODEX_MAX_TOKENS_ID: &str = "builtin-codex-max-tokens";
pub const BUILTIN_CODEX_TOOL_RETRY_ID: &str = "builtin-codex-tool-retry";

/// 一条规则的作用域。序列化为 `{ "kind": "default" }` / `{ "kind": "agent", "label": "claude-cli" }`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleScope {
    Default,
    Codex,
    Agent { label: String },
    Global,
}

/// 规则动作。序列化形如 `{"type":"setDefaultMaxTokens","value":32768}`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RuleAction {
    SetDefaultMaxTokens { value: u32 },
    SetHeader { key: String, value: String },
    SetBodyField { key: String, value: String },
    ToolTextRetry { extra: u32 },
}

/// 一条请求改写规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestRule {
    pub id: String,
    pub name: String,
    pub scope: RuleScope,
    pub enabled: bool,
    pub actions: Vec<RuleAction>,
}

/// 规则文件顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestRulesFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    rules: Vec<RequestRule>,
}

fn default_version() -> u32 {
    RULES_VERSION
}

fn rules_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join(RULES_FILE)
}

/// 内置默认规则：Codex 默认 max_tokens 开启，tool 重试默认关闭
/// （provider 级 `tool_text_retry_enabled/extra` 仍是主开关，开启本规则会覆盖 extra 次数）。
pub fn builtin_rules() -> Vec<RequestRule> {
    vec![
        RequestRule {
            id: BUILTIN_CODEX_MAX_TOKENS_ID.to_string(),
            name: "Codex 默认 max_tokens".to_string(),
            scope: RuleScope::Codex,
            enabled: true,
            actions: vec![RuleAction::SetDefaultMaxTokens { value: 32768 }],
        },
        RequestRule {
            id: BUILTIN_CODEX_TOOL_RETRY_ID.to_string(),
            name: "Codex 未调用工具时再要".to_string(),
            scope: RuleScope::Codex,
            enabled: false,
            actions: vec![RuleAction::ToolTextRetry { extra: 2 }],
        },
    ]
}

/// 是否为内置规则 id。
pub fn is_builtin_rule_id(id: &str) -> bool {
    id == BUILTIN_CODEX_MAX_TOKENS_ID || id == BUILTIN_CODEX_TOOL_RETRY_ID
}

/// 读取规则。文件不存在或内容损坏都回退到内置默认并告警，调用方无需处理 IO 错误。
pub fn load_rules(config_dir: &Path) -> Vec<RequestRule> {
    let path = rules_path(config_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return builtin_rules(),
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "读取请求改写规则失败，按内置默认处理");
            return builtin_rules();
        }
    };
    match serde_json::from_str::<RequestRulesFile>(&raw) {
        Ok(file) => file.rules,
        Err(err) => {
            tracing::warn!(error = %err, path = %path.display(), "请求改写规则不是合法 JSON，按内置默认处理");
            builtin_rules()
        }
    }
}

/// 持久化规则（原子写：pid 临时文件 → 原文件备份 → rename → 失败回滚）。
pub fn save_rules(config_dir: &Path, rules: &[RequestRule]) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建配置目录 {}", config_dir.display()))?;
    let file = RequestRulesFile {
        version: RULES_VERSION,
        rules: rules.to_vec(),
    };
    let content = serde_json::to_string_pretty(&file).context("序列化请求改写规则失败")?;
    atomic_write(&rules_path(config_dir), &content)
}

/// 校验单条规则；`existing` 用于重名检查（同一 id 自身不算重名）。
pub fn validate(rule: &RequestRule, existing: &[RequestRule]) -> Result<(), String> {
    let name = rule.name.trim();
    if name.is_empty() {
        return Err("规则名称不能为空".to_string());
    }
    if rule.actions.is_empty() {
        return Err("规则至少需要一个动作".to_string());
    }
    for action in &rule.actions {
        match action {
            RuleAction::SetDefaultMaxTokens { value } => {
                if !(1..=200_000).contains(value) {
                    return Err(format!("默认 max_tokens 必须在 1..=200000 之间，当前为 {value}"));
                }
            }
            RuleAction::SetHeader { key, .. } => {
                let key = key.trim();
                if key.is_empty() {
                    return Err("请求头名称不能为空".to_string());
                }
                if !key
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                {
                    return Err(format!("请求头名称只能包含字母、数字和连字符: {key}"));
                }
            }
            RuleAction::SetBodyField { key, .. } => {
                if key.trim().is_empty() {
                    return Err("请求体字段名不能为空".to_string());
                }
            }
            RuleAction::ToolTextRetry { extra } => {
                if *extra > 5 {
                    return Err(format!("工具文本重试次数不能超过 5，当前为 {extra}"));
                }
            }
        }
    }
    if let RuleScope::Agent { label } = &rule.scope {
        if label.trim().is_empty() {
            return Err("客户端标签不能为空".to_string());
        }
    }
    if existing
        .iter()
        .any(|other| other.id != rule.id && other.name.trim() == name)
    {
        return Err(format!("规则名称已存在: {name}"));
    }
    Ok(())
}

/// 插入或替换：id 为空则生成 uuid，id 已存在则整体替换。
pub fn upsert(rules: &mut Vec<RequestRule>, mut spec: RequestRule) -> RequestRule {
    spec.id = spec.id.trim().to_string();
    if spec.id.is_empty() {
        spec.id = uuid::Uuid::new_v4().to_string();
    }
    spec.name = spec.name.trim().to_string();
    if let Some(existing) = rules.iter_mut().find(|rule| rule.id == spec.id) {
        *existing = spec.clone();
    } else {
        rules.push(spec.clone());
    }
    spec
}

/// 作用域匹配所需的请求上下文。
pub struct RuleContext<'a> {
    /// `gateway_stats::client_label_from_headers` 的结果。
    pub client_label: &'a str,
    /// 与 Codex 路径一致（label == "codex" 或请求走 /v1/responses 的 Chat 桥）。
    pub is_codex: bool,
}

/// 按 Default → Codex → Agent(匹配 label) → Global 的顺序返回生效规则（后者可覆盖前者的同名动作）。
pub fn matching_rules<'a>(
    rules: &'a [RequestRule],
    ctx: &RuleContext<'_>,
) -> Vec<&'a RequestRule> {
    let label = ctx.client_label.trim();
    let mut matched: Vec<&RequestRule> = rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter(|rule| match &rule.scope {
            RuleScope::Default => true,
            RuleScope::Codex => ctx.is_codex,
            RuleScope::Agent { label: rule_label } => {
                !label.is_empty() && rule_label.trim().eq_ignore_ascii_case(label)
            }
            RuleScope::Global => true,
        })
        .collect();
    // 稳定排序：同作用域内保持文件顺序。
    matched.sort_by_key(|rule| scope_order(&rule.scope));
    matched
}

fn scope_order(scope: &RuleScope) -> u8 {
    match scope {
        RuleScope::Default => 0,
        RuleScope::Codex => 1,
        RuleScope::Agent { .. } => 2,
        RuleScope::Global => 3,
    }
}

/// 合并后的动作效果，纯数据，由调用方决定如何应用到请求。
#[derive(Debug, Default, Clone)]
pub struct RuleEffects {
    /// None = 没有规则给出，由调用方沿用既有默认。
    pub max_tokens: Option<u32>,
    /// Codex 的 max_tokens 规则被显式关闭时为 true（区别于「没配置」）。
    pub max_tokens_suppressed: bool,
    pub tool_retry_extra: Option<u32>,
    pub headers: Vec<(String, String)>,
    pub body_fields: Vec<(String, String)>,
}

/// 收集生效规则的动作效果；headers / body_fields 按 key 去重，保留最后一次的值。
pub fn collect_effects(rules: &[RequestRule], ctx: &RuleContext<'_>) -> RuleEffects {
    let mut effects = RuleEffects::default();
    for rule in matching_rules(rules, ctx) {
        for action in &rule.actions {
            match action {
                RuleAction::SetDefaultMaxTokens { value } => effects.max_tokens = Some(*value),
                RuleAction::SetHeader { key, value } => {
                    upsert_pair(&mut effects.headers, key, value)
                }
                RuleAction::SetBodyField { key, value } => {
                    upsert_pair(&mut effects.body_fields, key, value)
                }
                RuleAction::ToolTextRetry { extra } => effects.tool_retry_extra = Some(*extra),
            }
        }
    }
    // 只有「全部 codex 作用域的 max_tokens 规则都被显式关闭」才算 suppressed；
    // 否则关掉一条自定义规则会顺带把仍启用的内置默认值也关掉。
    let codex_max_rules: Vec<&RequestRule> = rules
        .iter()
        .filter(|rule| {
            rule.scope == RuleScope::Codex
                && rule
                    .actions
                    .iter()
                    .any(|action| matches!(action, RuleAction::SetDefaultMaxTokens { .. }))
        })
        .collect();
    effects.max_tokens_suppressed =
        !codex_max_rules.is_empty() && !codex_max_rules.iter().any(|rule| rule.enabled);
    effects
}

fn upsert_pair(pairs: &mut Vec<(String, String)>, key: &str, value: &str) {
    let key = key.trim();
    if let Some(existing) = pairs.iter_mut().find(|(existing_key, _)| existing_key == key) {
        existing.1 = value.to_string();
        return;
    }
    pairs.push((key.to_string(), value.to_string()));
}

/// 原子写：先写临时文件，再用「备份 → 替换 → 失败回滚」落地。
/// 与 `model_registry.rs::atomic_write` 同一做法，避免 Windows rename 不能覆盖导致丢文件。
fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("request-rules");
    let tmp = path.with_file_name(format!("{name}.sugt-tmp"));
    std::fs::write(&tmp, content)
        .with_context(|| format!("无法写入临时文件 {}", tmp.display()))?;
    match replace_with_backup(path, &tmp) {
        Ok(()) => Ok(()),
        Err(err) => {
            // 失败绝不留下半成品临时文件。
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn replace_with_backup(path: &Path, tmp: &Path) -> Result<()> {
    if path.is_dir() {
        anyhow::bail!("目标路径是目录，拒绝替换 {}", path.display());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("request-rules");
    let backup = path.with_file_name(format!("{name}.sugt-{}.backup", std::process::id()));
    if path.exists() {
        std::fs::rename(path, &backup)
            .with_context(|| format!("无法备份 {}", path.display()))?;
    }
    if let Err(err) = std::fs::rename(tmp, path) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, path);
        }
        return Err(err).with_context(|| format!("无法替换 {}", path.display()));
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// OS 中立的独立临时目录。
    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sugt-request-rules-{}-{tag}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn rule(
        id: &str,
        name: &str,
        scope: RuleScope,
        enabled: bool,
        actions: Vec<RuleAction>,
    ) -> RequestRule {
        RequestRule {
            id: id.to_string(),
            name: name.to_string(),
            scope,
            enabled,
            actions,
        }
    }

    fn header_rule(id: &str, scope: RuleScope, key: &str, value: &str) -> RequestRule {
        rule(
            id,
            id,
            scope,
            true,
            vec![RuleAction::SetHeader {
                key: key.to_string(),
                value: value.to_string(),
            }],
        )
    }

    fn ctx(label: &str, is_codex: bool) -> RuleContext<'_> {
        RuleContext {
            client_label: label,
            is_codex,
        }
    }

    #[test]
    fn missing_file_returns_builtin_rules() {
        let dir = temp_dir("missing");
        let rules = load_rules(&dir);
        assert_eq!(rules.len(), 2);
        let max_tokens = rules
            .iter()
            .find(|rule| rule.id == BUILTIN_CODEX_MAX_TOKENS_ID)
            .expect("内置 max_tokens 规则");
        assert!(max_tokens.enabled);
        assert_eq!(max_tokens.scope, RuleScope::Codex);
        assert_eq!(
            max_tokens.actions,
            vec![RuleAction::SetDefaultMaxTokens { value: 32768 }]
        );
        let retry = rules
            .iter()
            .find(|rule| rule.id == BUILTIN_CODEX_TOOL_RETRY_ID)
            .expect("内置 tool 重试规则");
        assert!(!retry.enabled);
        assert_eq!(retry.actions, vec![RuleAction::ToolTextRetry { extra: 2 }]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_builtin() {
        let dir = temp_dir("corrupt");
        std::fs::write(rules_path(&dir), "{not valid json").expect("write corrupt file");
        let rules = load_rules(&dir);
        assert_eq!(rules.len(), 2);
        assert!(rules
            .iter()
            .any(|rule| rule.id == BUILTIN_CODEX_MAX_TOKENS_ID));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let saved = vec![rule(
            "custom-1",
            "给 claude 加请求头",
            RuleScope::Agent {
                label: "claude-cli".to_string(),
            },
            true,
            vec![
                RuleAction::SetHeader {
                    key: "X-Sugt-Test".to_string(),
                    value: "1".to_string(),
                },
                RuleAction::SetBodyField {
                    key: "max_completion_tokens".to_string(),
                    value: "1024".to_string(),
                },
            ],
        )];
        save_rules(&dir, &saved).expect("save");
        let loaded = load_rules(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            serde_json::to_value(&loaded[0]).expect("json"),
            serde_json::to_value(&saved[0]).expect("json")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matching_rules_follow_scope_order() {
        let rules = vec![
            header_rule("global-rule", RuleScope::Global, "X-Scope", "global"),
            header_rule(
                "agent-rule",
                RuleScope::Agent {
                    label: "claude-cli".to_string(),
                },
                "X-Scope",
                "agent",
            ),
            header_rule("codex-rule", RuleScope::Codex, "X-Scope", "codex"),
            header_rule("default-rule", RuleScope::Default, "X-Scope", "default"),
        ];
        let matched = matching_rules(&rules, &ctx("claude-cli", true));
        let ids: Vec<&str> = matched.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids, vec!["default-rule", "codex-rule", "agent-rule", "global-rule"]);
    }

    #[test]
    fn agent_rule_only_matches_its_label() {
        let rules = vec![header_rule(
            "agent-rule",
            RuleScope::Agent {
                label: "claude-cli".to_string(),
            },
            "X-Scope",
            "agent",
        )];
        assert_eq!(matching_rules(&rules, &ctx("claude-cli", false)).len(), 1);
        assert!(matching_rules(&rules, &ctx("codex", true)).is_empty());
        assert!(matching_rules(&rules, &ctx("trae", false)).is_empty());
        assert!(matching_rules(&rules, &ctx("", false)).is_empty());
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let rules = vec![
            rule("off", "off", RuleScope::Default, false, vec![RuleAction::SetBodyField {
                key: "a".to_string(),
                value: "b".to_string(),
            }]),
            rule("on", "on", RuleScope::Default, true, vec![RuleAction::SetBodyField {
                key: "c".to_string(),
                value: "d".to_string(),
            }]),
        ];
        let matched = matching_rules(&rules, &ctx("claude-cli", false));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "on");
    }

    #[test]
    fn set_header_later_rule_overrides_same_key() {
        let rules = vec![
            header_rule("default-rule", RuleScope::Default, "X-Test", "default"),
            header_rule("global-rule", RuleScope::Global, "X-Test", "global"),
            header_rule("other-key", RuleScope::Global, "X-Other", "keep"),
        ];
        let effects = collect_effects(&rules, &ctx("trae", false));
        assert_eq!(
            effects.headers,
            vec![
                ("X-Test".to_string(), "global".to_string()),
                ("X-Other".to_string(), "keep".to_string()),
            ]
        );
    }

    #[test]
    fn disabled_codex_max_tokens_suppresses_default() {
        // 默认（未改）时：Codex 规则给出 32768，不算 suppressed。
        let effects = collect_effects(&builtin_rules(), &ctx("codex", true));
        assert_eq!(effects.max_tokens, Some(32768));
        assert!(!effects.max_tokens_suppressed);
        // 内置 tool-retry 默认关闭，不应产生重试次数。
        assert_eq!(effects.tool_retry_extra, None);

        // 用户关掉 codex max_tokens 规则后：suppressed 且不再给出默认值。
        let mut rules = builtin_rules();
        rules
            .iter_mut()
            .find(|rule| rule.id == BUILTIN_CODEX_MAX_TOKENS_ID)
            .expect("内置规则")
            .enabled = false;
        let effects = collect_effects(&rules, &ctx("codex", true));
        assert_eq!(effects.max_tokens, None);
        assert!(effects.max_tokens_suppressed);

        // 只关掉一条自定义 codex max_tokens 规则时，仍启用的内置默认值不受影响。
        let mut rules = builtin_rules();
        rules.push(rule(
            "custom",
            "custom",
            RuleScope::Codex,
            false,
            vec![RuleAction::SetDefaultMaxTokens { value: 4096 }],
        ));
        let effects = collect_effects(&rules, &ctx("codex", true));
        assert_eq!(effects.max_tokens, Some(32768));
        assert!(!effects.max_tokens_suppressed);
    }

    #[test]
    fn set_body_field_merges_and_retry_uses_last_value() {
        let rules = vec![
            rule("a", "a", RuleScope::Default, true, vec![
                RuleAction::SetBodyField {
                    key: "temperature".to_string(),
                    value: "0.2".to_string(),
                },
                RuleAction::ToolTextRetry { extra: 1 },
            ]),
            rule("b", "b", RuleScope::Global, true, vec![
                RuleAction::SetBodyField {
                    key: "temperature".to_string(),
                    value: "0.7".to_string(),
                },
                RuleAction::ToolTextRetry { extra: 3 },
            ]),
        ];
        let effects = collect_effects(&rules, &ctx("trae", false));
        assert_eq!(
            effects.body_fields,
            vec![("temperature".to_string(), "0.7".to_string())]
        );
        assert_eq!(effects.tool_retry_extra, Some(3));
    }

    #[test]
    fn upsert_assigns_uuid_and_replaces_existing() {
        let mut rules = Vec::new();
        let created = upsert(
            &mut rules,
            rule("", "新规则", RuleScope::Default, true, vec![RuleAction::ToolTextRetry { extra: 1 }]),
        );
        assert!(!created.id.is_empty());
        assert_eq!(rules.len(), 1);

        let replaced = upsert(
            &mut rules,
            rule(&created.id, "改名", RuleScope::Global, false, vec![RuleAction::ToolTextRetry { extra: 2 }]),
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(replaced.id, created.id);
        assert_eq!(rules[0].name, "改名");
        assert_eq!(rules[0].scope, RuleScope::Global);
        assert!(!rules[0].enabled);
    }

    #[test]
    fn validate_rejects_invalid_inputs() {
        let existing = vec![header_rule("keep", RuleScope::Default, "X-Keep", "1")];
        let ok_action = RuleAction::SetHeader {
            key: "X-Good-1".to_string(),
            value: "v".to_string(),
        };

        let empty_name = rule("", "  ", RuleScope::Default, true, vec![ok_action.clone()]);
        assert!(validate(&empty_name, &existing).is_err());

        let empty_actions = rule("", "名字", RuleScope::Default, true, vec![]);
        assert!(validate(&empty_actions, &existing).is_err());

        let bad_header = rule("", "名字", RuleScope::Default, true, vec![RuleAction::SetHeader {
            key: "X_Bad Key".to_string(),
            value: "v".to_string(),
        }]);
        assert!(validate(&bad_header, &existing).is_err());

        let empty_body_key = rule("", "名字", RuleScope::Default, true, vec![RuleAction::SetBodyField {
            key: "  ".to_string(),
            value: "v".to_string(),
        }]);
        assert!(validate(&empty_body_key, &existing).is_err());

        for value in [0u32, 200_001] {
            let invalid = rule("", "名字", RuleScope::Default, true, vec![RuleAction::SetDefaultMaxTokens {
                value,
            }]);
            assert!(validate(&invalid, &existing).is_err());
        }
        let valid_max = rule("", "名字", RuleScope::Default, true, vec![RuleAction::SetDefaultMaxTokens {
            value: 200_000,
        }]);
        assert!(validate(&valid_max, &existing).is_ok());

        let too_many_retries = rule("", "名字", RuleScope::Default, true, vec![
            RuleAction::ToolTextRetry { extra: 6 },
        ]);
        assert!(validate(&too_many_retries, &existing).is_err());

        let empty_agent_label = rule("", "名字", RuleScope::Agent {
            label: "  ".to_string(),
        }, true, vec![ok_action.clone()]);
        assert!(validate(&empty_agent_label, &existing).is_err());

        let duplicate = rule("", "keep", RuleScope::Default, true, vec![ok_action.clone()]);
        assert!(validate(&duplicate, &existing).is_err());

        // 同一 id 重新保存自身不算重名。
        let self_save = rule("keep", "keep", RuleScope::Default, true, vec![ok_action]);
        assert!(validate(&self_save, &existing).is_ok());
    }
}
