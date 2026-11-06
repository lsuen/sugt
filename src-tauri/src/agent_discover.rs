//! 从本机环境（进程/用户/系统环境变量 + PATH）发现 Agent 候选。

use crate::takeover_profiles::{self, expand_home, TakeoverProfile};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredAgent {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub protocol: String,
    /// high | low
    pub confidence: String,
    pub already_added: bool,
    pub cli_path: Option<String>,
    pub settings_path: Option<String>,
    pub skill_dirs: Vec<String>,
    pub launch_command: Option<String>,
    pub hit_reasons: Vec<String>,
    pub env_vars: BTreeMap<String, String>,
    pub clear_vars: Vec<String>,
    pub warning: Option<String>,
    /// 是否像纯 IDE（需用户补目录才能添加）
    pub requires_path: bool,
    /// 该 Agent 会写入的字段名（不含值），来自能力矩阵
    pub writable_fields: Vec<String>,
    /// 配置方式说明
    pub config_note: String,
}

struct Fingerprint {
    id: &'static str,
    name: &'static str,
    vendor: &'static str,
    protocol: &'static str,
    keywords: &'static [&'static str],
    commands: &'static [&'static str],
    env_hints: &'static [&'static str],
    /// 纯 IDE / 编辑器，默认低置信
    ide_like: bool,
    env_vars: &'static [(&'static str, &'static str)],
    clear_vars: &'static [&'static str],
}

fn fingerprints() -> Vec<Fingerprint> {
    vec![
        Fingerprint {
            id: "claude-code",
            name: "Claude Code",
            vendor: "Anthropic",
            protocol: "anthropic",
            keywords: &["claude", "anthropic"],
            commands: &["claude"],
            env_hints: &["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"],
            ide_like: false,
            env_vars: &[
                ("ANTHROPIC_BASE_URL", "{anthropic_base}"),
                ("ANTHROPIC_AUTH_TOKEN", "{client_key}"),
            ],
            clear_vars: &["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"],
        },
        Fingerprint {
            id: "codex",
            name: "OpenAI Codex CLI",
            vendor: "OpenAI",
            protocol: "openai",
            keywords: &["codex"],
            commands: &["codex"],
            env_hints: &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY"],
            ide_like: false,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
                ("OPENAI_API_BASE", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "opencode",
            name: "OpenCode",
            vendor: "OpenCode",
            protocol: "openai",
            keywords: &["opencode", "open-code"],
            commands: &["opencode"],
            env_hints: &["OPENCODE_API_KEY", "OPENAI_API_KEY"],
            ide_like: false,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "qwen-code",
            name: "Qwen Code",
            vendor: "Alibaba",
            protocol: "openai",
            keywords: &["qwen", "qwencode"],
            commands: &["qwen", "qwencode"],
            env_hints: &["DASHSCOPE_API_KEY", "QWEN_API_KEY"],
            ide_like: false,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "aider",
            name: "Aider",
            vendor: "Aider",
            protocol: "openai",
            keywords: &["aider"],
            commands: &["aider"],
            env_hints: &["OPENAI_API_KEY", "AIDER_"],
            ide_like: false,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_API_BASE", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "gemini-cli",
            name: "Gemini CLI",
            vendor: "Google",
            protocol: "openai",
            keywords: &["gemini"],
            commands: &["gemini"],
            env_hints: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            ide_like: false,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "cursor",
            name: "Cursor",
            vendor: "Cursor",
            protocol: "openai",
            keywords: &["cursor"],
            commands: &["cursor"],
            env_hints: &["CURSOR_"],
            ide_like: true,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ],
            clear_vars: &[],
        },
        Fingerprint {
            id: "vscode",
            name: "VS Code",
            vendor: "Microsoft",
            protocol: "openai",
            keywords: &["vscode", "code"],
            commands: &["code"],
            env_hints: &[],
            ide_like: true,
            env_vars: &[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ],
            clear_vars: &[],
        },
    ]
}

/// 能力矩阵里登记的全部已知客户端（空查询时的目录视图）。
/// 完全从 `agent_adapters::all()` 派生，不再维护第二份客户端清单。
pub fn known_agents(config_dir: &Path) -> Vec<DiscoveredAgent> {
    let existing: HashSet<String> = takeover_profiles::all_profiles(config_dir)
        .into_iter()
        .map(|p| p.id.to_ascii_lowercase())
        .collect();
    let path_bins = list_path_binaries(None);
    let mut results: Vec<DiscoveredAgent> = crate::agent_adapters::all()
        .iter()
        .map(|adapter| catalog_item(adapter, &existing, &path_bins))
        .collect();
    results.sort_by(|a, b| {
        confidence_rank(&a.confidence)
            .cmp(&confidence_rank(&b.confidence))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    results
}

fn adapter_matches_query(adapter: &crate::agent_adapters::AgentAdapter, q: &str) -> bool {
    adapter.name.to_ascii_lowercase().contains(q)
        || adapter.vendor.to_ascii_lowercase().contains(q)
        || adapter.id.to_ascii_lowercase().contains(q)
        || adapter.commands.iter().any(|c| c.contains(q))
}

/// 从单一真相源构造一条候选：本机痕迹决定置信度，但目录条目始终可选、可主动接管。
fn catalog_item(
    adapter: &crate::agent_adapters::AgentAdapter,
    existing: &HashSet<String>,
    path_bins: &[PathBuf],
) -> DiscoveredAgent {
    let mut reasons = Vec::new();
    let mut cli_path = None;
    for cmd in adapter.commands {
        if let Some(path) = find_bin(cmd, path_bins) {
            reasons.push(format!("CLI: {}", path.display()));
            cli_path = Some(path.display().to_string());
            break;
        }
    }

    let mut settings_path = None;
    for dir in adapter.settings_dirs {
        if let Some(path) = expand_home(dir).filter(|p| p.is_dir()) {
            reasons.push(format!("配置目录: {}", path.display()));
            settings_path = Some(path.display().to_string());
            break;
        }
    }

    let mut skill_dirs = Vec::new();
    for dir in adapter.skill_dirs {
        if let Some(path) = expand_home(dir) {
            if path.is_dir() {
                reasons.push(format!("技能目录: {}", path.display()));
            }
            skill_dirs.push(path.display().to_string());
        }
    }

    let detected = cli_path.is_some()
        || settings_path.is_some()
        || adapter
            .skill_dirs
            .iter()
            .any(|dir| expand_home(dir).is_some_and(|p| p.is_dir()));
    reasons.push(if detected {
        "已知客户端目录".into()
    } else {
        "已知客户端目录（本机未检出，可主动接管）".into()
    });

    // 旧指纹表里的环境变量模板优先；新客户端按协议给通用模板，绝不虚报官方配置格式。
    let env_vars = fingerprints()
        .into_iter()
        .find(|fp| fp.id.eq_ignore_ascii_case(adapter.id))
        .map(|fp| btreemap(fp.env_vars))
        .unwrap_or_else(|| {
            if adapter.protocol == "anthropic" {
                btreemap(&[
                    ("ANTHROPIC_BASE_URL", "{anthropic_base}"),
                    ("ANTHROPIC_AUTH_TOKEN", "{client_key}"),
                ])
            } else {
                btreemap(&[
                    ("OPENAI_API_KEY", "{client_key}"),
                    ("OPENAI_BASE_URL", "{openai_base}"),
                ])
            }
        });

    DiscoveredAgent {
        id: adapter.id.to_string(),
        name: adapter.name.to_string(),
        vendor: adapter.vendor.to_string(),
        protocol: adapter.protocol.to_string(),
        confidence: if detected { "high" } else { "low" }.into(),
        already_added: existing.contains(&adapter.id.to_ascii_lowercase()),
        cli_path,
        settings_path,
        skill_dirs,
        launch_command: adapter.commands.first().map(|s| s.to_string()),
        hit_reasons: reasons,
        env_vars,
        clear_vars: adapter
            .config
            .clear_vars
            .iter()
            .map(|s| s.to_string())
            .collect(),
        warning: if detected {
            None
        } else {
            Some("本机未检出该客户端，接管前请核对技能目录".into())
        },
        // 目录条目由用户主动选择，不强制补路径；接管前仍会走影响预览。
        requires_path: false,
        writable_fields: adapter
            .config
            .writable_fields
            .iter()
            .map(|s| s.to_string())
            .collect(),
        config_note: adapter.config.note.to_string(),
    }
}

pub fn discover_agents(config_dir: &Path, query: &str) -> Vec<DiscoveredAgent> {
    let q = query.trim().to_ascii_lowercase();
    // 空查询 / 单字符不再返回空列表，而是列出全部已知客户端，避免「发现不了就没法接管」。
    if q.len() < 2 {
        return known_agents(config_dir);
    }

    let existing: HashSet<String> = takeover_profiles::all_profiles(config_dir)
        .into_iter()
        .map(|p| p.id.to_ascii_lowercase())
        .collect();

    let env_map = collect_env_map();
    let path_bins = list_path_binaries(Some(&q));

    let mut results = Vec::new();
    let mut seen_ids = HashSet::new();

    for fp in fingerprints() {
        if !fingerprint_matches_query(&fp, &q) {
            continue;
        }
        if let Some(item) = probe_fingerprint(&fp, &env_map, &path_bins, &existing) {
            seen_ids.insert(item.id.to_ascii_lowercase());
            results.push(item);
        }
    }

    // PATH 里命中查询词、但未落入指纹的可执行文件 → 低置信候选
    for bin in &path_bins {
        let stem = bin
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if stem.is_empty() || !stem.contains(&q) {
            continue;
        }
        // 已由指纹覆盖
        if fingerprints().iter().any(|fp| {
            fp.commands
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&stem))
        }) {
            continue;
        }
        let id = format!("discovered-{}", stem.replace(' ', "-"));
        if seen_ids.contains(&id.to_ascii_lowercase()) {
            continue;
        }
        seen_ids.insert(id.to_ascii_lowercase());
        results.push(DiscoveredAgent {
            id,
            name: stem.clone(),
            vendor: "Detected".into(),
            protocol: "openai".into(),
            confidence: "low".into(),
            already_added: existing.contains(&format!("discovered-{}", stem)),
            cli_path: Some(bin.display().to_string()),
            settings_path: None,
            skill_dirs: vec![],
            launch_command: Some(stem),
            hit_reasons: vec![format!("PATH: {}", bin.display())],
            env_vars: btreemap(&[
                ("OPENAI_API_KEY", "{client_key}"),
                ("OPENAI_BASE_URL", "{openai_base}"),
            ]),
            clear_vars: vec![],
            warning: Some("未识别到 Agent 特征，请补充目录".into()),
            requires_path: true,
            writable_fields: vec![],
            config_note: "未识别客户端：能力未声明，请补充目录后核对".into(),
        });
    }

    // 能力矩阵新增的客户端不在旧指纹表里：按名称 / 厂商 / 命令补入，保证搜得到国内外知名 Agent。
    let extra: Vec<&crate::agent_adapters::AgentAdapter> = crate::agent_adapters::all()
        .iter()
        .filter(|adapter| {
            !seen_ids.contains(&adapter.id.to_ascii_lowercase())
                && adapter_matches_query(adapter, &q)
        })
        .collect();
    if !extra.is_empty() {
        let all_bins = list_path_binaries(None);
        for adapter in extra {
            seen_ids.insert(adapter.id.to_ascii_lowercase());
            results.push(catalog_item(adapter, &existing, &all_bins));
        }
    }

    results.sort_by(|a, b| {
        confidence_rank(&a.confidence)
            .cmp(&confidence_rank(&b.confidence))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    results
}

fn confidence_rank(c: &str) -> u8 {
    if c == "high" {
        0
    } else {
        1
    }
}

fn fingerprint_matches_query(fp: &Fingerprint, q: &str) -> bool {
    let name = fp.name.to_ascii_lowercase();
    let vendor = fp.vendor.to_ascii_lowercase();
    fp.keywords
        .iter()
        .any(|k| *k == q || k.starts_with(q) || (k.len() >= 3 && q.starts_with(k)))
        || fp.commands
            .iter()
            .any(|c| *c == q || c.starts_with(q) || (c.len() >= 3 && q.starts_with(c)))
        || fp.id == q
        || fp.id.starts_with(&format!("{q}-"))
        || name == q
        || vendor == q
        || (q.len() >= 5 && name.contains(q))
}

fn probe_fingerprint(
    fp: &Fingerprint,
    env_map: &BTreeMap<String, String>,
    path_bins: &[PathBuf],
    existing: &HashSet<String>,
) -> Option<DiscoveredAgent> {
    let mut reasons = Vec::new();
    let mut cli_path = None;
    for cmd in fp.commands {
        if let Some(p) = find_bin(cmd, path_bins) {
            reasons.push(format!("CLI: {}", p.display()));
            cli_path = Some(p.display().to_string());
            break;
        }
    }

    // 路径只从 AgentAdapter 取，指纹表里不再维护第二份。
    let adapter = crate::agent_adapters::by_id(fp.id);
    let settings_candidates: &'static [&'static str] =
        adapter.map(|a| a.settings_dirs).unwrap_or(&[]);
    let skill_candidates: &'static [&'static str] =
        adapter.map(|a| a.skill_dirs).unwrap_or(&[]);

    let mut settings_path = None;
    for dir in settings_candidates {
        if let Some(p) = expand_home(dir).filter(|p| p.is_dir()) {
            reasons.push(format!("配置目录: {}", p.display()));
            settings_path = Some(p.display().to_string());
            break;
        }
    }

    let mut skill_dirs = Vec::new();
    for dir in skill_candidates {
        if let Some(p) = expand_home(dir).filter(|p| p.is_dir()) {
            reasons.push(format!("技能目录: {}", p.display()));
            skill_dirs.push(p.display().to_string());
        } else if !dir.is_empty() {
            skill_dirs.push(dir.to_string());
        }
    }

    for hint in fp.env_hints {
        let hit = env_map.keys().any(|k| {
            if hint.ends_with('_') {
                k.to_ascii_uppercase().starts_with(&hint.to_ascii_uppercase())
            } else {
                k.eq_ignore_ascii_case(hint)
            }
        });
        if hit {
            reasons.push(format!("环境变量: {}", hint.trim_end_matches('_')));
        }
    }

    let has_agent_signal = cli_path.is_some() || settings_path.is_some() || !skill_dirs.is_empty()
        || reasons.iter().any(|r| r.starts_with("环境变量:"));

    // 查询命中关键词但本机毫无痕迹：仍返回，便于用户看到并手动补目录（IDE 类）
    // 若完全无痕迹且非 ide，也可返回低置信占位
    let has_agent_dir = settings_path.is_some()
        || skill_dirs
            .iter()
            .any(|s| expand_home(s).is_some_and(|p| p.is_dir()));

    // IDE / 编辑器默认低置信，除非本机已有明确配置/技能目录
    let confidence = if fp.ide_like {
        if has_agent_dir {
            "high"
        } else {
            "low"
        }
    } else if has_agent_signal {
        "high"
    } else {
        "low"
    };

    if reasons.is_empty() {
        reasons.push("关键词匹配（本机未检测到 CLI/目录）".into());
    }

    let requires_path = confidence == "low";
    let warning = if requires_path {
        Some("未识别为 Agent，请补充目录".into())
    } else {
        None
    };

    Some(DiscoveredAgent {
        id: fp.id.to_string(),
        name: fp.name.to_string(),
        vendor: fp.vendor.to_string(),
        protocol: fp.protocol.to_string(),
        confidence: confidence.into(),
        already_added: existing.contains(&fp.id.to_ascii_lowercase()),
        cli_path,
        settings_path,
        skill_dirs,
        launch_command: fp.commands.first().map(|s| s.to_string()),
        hit_reasons: reasons,
        env_vars: btreemap(fp.env_vars),
        clear_vars: fp.clear_vars.iter().map(|s| s.to_string()).collect(),
        warning,
        requires_path,
        writable_fields: adapter
            .map(|a| a.config.writable_fields.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default(),
        config_note: adapter
            .map(|a| a.config.note.to_string())
            .unwrap_or_else(|| "自定义客户端：能力未声明，请核对后再接入".to_string()),
    })
}

fn btreemap(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn find_bin(cmd: &str, path_bins: &[PathBuf]) -> Option<PathBuf> {
    let cmd_l = cmd.to_ascii_lowercase();
    path_bins.iter().find(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case(&cmd_l))
            .unwrap_or(false)
    }).cloned().or_else(|| find_command_on_path(cmd))
}

fn find_command_on_path(cmd: &str) -> Option<PathBuf> {
    crate::process_util::find_command_on_path(cmd)
}

fn list_path_binaries(name_contains: Option<&str>) -> Vec<PathBuf> {
    let filter = name_contains.map(|s| s.to_ascii_lowercase());
    let mut out = Vec::new();
    let path_val = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_val) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(ref f) = filter {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !stem.contains(f) {
                    continue;
                }
            }
            if !crate::platform::is_executable(&path) {
                continue;
            }
            out.push(path);
        }
    }
    out
}

fn collect_env_map() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (k, v) in std::env::vars() {
        map.insert(k, v);
    }
    #[cfg(windows)]
    {
        for (k, v) in crate::platform::windows::read_reg_env(r"HKCU\Environment") {
            map.entry(k).or_insert(v);
        }
        for (k, v) in crate::platform::windows::read_reg_env(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ) {
            map.entry(k).or_insert(v);
        }
    }
    map
}

/// 将发现结果转为可保存的模板（可带用户补的路径）。
pub fn discovered_to_profile(item: &DiscoveredAgent, extra_path: Option<&str>) -> TakeoverProfile {
    let mut settings_dirs = Vec::new();
    let mut skill_dirs = item.skill_dirs.clone();
    let mut settings_path = item.settings_path.clone();

    if let Some(raw) = extra_path.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(p) = expand_home(raw) {
            let display = p.display().to_string();
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name == "skills" {
                skill_dirs.push(display.clone());
                if let Some(parent) = p.parent() {
                    settings_dirs.push(parent.display().to_string());
                    settings_path = Some(parent.display().to_string());
                }
            } else {
                settings_dirs.push(display.clone());
                settings_path = Some(display.clone());
                skill_dirs.push(p.join("skills").display().to_string());
            }
        } else {
            settings_dirs.push(raw.to_string());
            settings_path = Some(raw.to_string());
        }
    } else if let Some(ref sp) = item.settings_path {
        settings_dirs.push(sp.clone());
    }

    TakeoverProfile {
        id: item.id.clone(),
        name: item.name.clone(),
        vendor: item.vendor.clone(),
        description: format!("由环境发现添加 · {}", item.hit_reasons.join("；")),
        protocol: item.protocol.clone(),
        env_vars: item.env_vars.clone(),
        clear_vars: item.clear_vars.clone(),
        settings_dirs,
        settings_path,
        skill_dirs,
        launch_command: item.launch_command.clone(),
        builtin: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_matches_claude_keyword() {
        let fp = fingerprints()
            .into_iter()
            .find(|f| f.id == "claude-code")
            .unwrap();
        assert!(fingerprint_matches_query(&fp, "claude"));
        assert!(fingerprint_matches_query(&fp, "anthropic"));
        assert!(!fingerprint_matches_query(&fp, "code"));
        let vscode = fingerprints()
            .into_iter()
            .find(|f| f.id == "vscode")
            .unwrap();
        assert!(fingerprint_matches_query(&vscode, "code"));
    }

    #[test]
    fn empty_query_lists_every_known_agent() {
        let dir = std::env::temp_dir().join(format!("sugt-discover-catalog-{}", std::process::id()));
        let list = discover_agents(&dir, "");
        assert_eq!(list.len(), crate::agent_adapters::all().len());
        for id in ["codex", "claude-code", "codebuddy", "kimi", "windsurf"] {
            assert!(list.iter().any(|a| a.id == id), "目录缺少 {id}");
        }
        assert!(
            list.iter().all(|a| !a.requires_path),
            "目录条目应允许直接选择接管"
        );
        assert!(
            list.iter().all(|a| !a.config_note.is_empty()),
            "目录条目必须带能力说明"
        );
    }

    #[test]
    fn newly_added_domestic_and_global_clients_are_searchable() {
        let dir = std::env::temp_dir().join(format!("sugt-discover-search-{}", std::process::id()));
        for (query, id) in [
            ("codebuddy", "codebuddy"),
            ("kimi", "kimi"),
            ("trae", "trae"),
            ("windsurf", "windsurf"),
            ("灵码", "lingma"),
        ] {
            let list = discover_agents(&dir, query);
            assert!(
                list.iter().any(|a| a.id == id),
                "查询「{query}」未找到 {id}"
            );
        }
    }

    #[test]
    fn catalog_item_uses_a_safe_env_template_for_new_clients() {
        let dir = std::env::temp_dir().join(format!("sugt-discover-env-{}", std::process::id()));
        let list = discover_agents(&dir, "");
        let kimi = list.iter().find(|a| a.id == "kimi").unwrap();
        assert_eq!(
            kimi.env_vars.get("OPENAI_BASE_URL").map(String::as_str),
            Some("{openai_base}")
        );
        assert!(!kimi.env_vars.contains_key("ANTHROPIC_BASE_URL"));
    }
}
