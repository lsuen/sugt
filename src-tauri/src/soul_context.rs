//! SOUL 只读编排层：把运行时事实注入 `system_prompt`，模型产出意图建议卡片。
//! 作者：sun / 孙文龙
//!
//! 管理类意图（接管 / 装技能 / 改客户端配置）仍走界面按钮 + 后果预览。
//! 日常本机小事由小su 自己 bash 完成——编排层不要把「智能助手」训成「动不动说不行」。
//! 本模块全部只读；任何读取失败都降级为「暂无数据」，绝不阻断对话。

use crate::store::paths::StorePaths;
use std::path::Path;

const MAX_NAMES: usize = 12;
const MAX_SKILL_HINTS: usize = 3;
const SKILL_DESC_CHARS: usize = 120;

/// 建议卡片标记。前端解析后渲染成按钮；未知标记剥离，不当指令执行。
pub const ROUTE_TOKEN_PREFIX: &str = "[[SUGT_ROUTE]]";
pub const ROUTE_TOKEN_SUFFIX: &str = "[[/SUGT_ROUTE]]";

fn brief_list(values: &[String], limit: usize, empty: &str) -> String {
    if values.is_empty() {
        return empty.to_string();
    }
    let mut out = values
        .iter()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .join(" / ");
    if values.len() > limit {
        out.push_str(&format!(" … +{}", values.len() - limit));
    }
    out
}

/// 汇总能力矩阵 / 已接管客户端 / 已装技能 / 仓库目录，生成注入提示词的事实摘要。
pub fn soul_brief(
    zh: bool,
    config_dir: &Path,
    store_paths: &StorePaths,
    active_takeover_ids: &[String],
) -> String {
    let capabilities = crate::takeover_preview::list_capabilities();
    let cap_names: Vec<String> = capabilities.iter().map(|c| c.name.clone()).collect();
    let skills: Vec<String> =
        crate::store::hub::list_local_skills(store_paths, config_dir)
            .map(|list| list.into_iter().map(|s| s.name).collect())
            .unwrap_or_default();
    let repos: Vec<String> = crate::store::repos::load_repos(store_paths)
        .map(|list| list.into_iter().map(|r| r.label()).collect())
        .unwrap_or_default();
    let active: Vec<String> = active_takeover_ids.to_vec();

    if zh {
        format!(
            "【本机事实（程序提供，不要凭记忆猜）】\n\
             - 已知客户端 {cap_count} 个：{caps}\n\
             - 已接管：{active}\n\
             - 本地技能 {skill_count} 个：{skills}\n\
             - 技能仓库 {repo_count} 个：{repos}\n\
             【管理类意图路由】\n\
             用户想管理客户端、发现 Agent、接管或释放 → 回复里放一行 [[SUGT_ROUTE]]clients[[/SUGT_ROUTE]]；\n\
             用户想管理技能、从 GitHub 装技能、挂载、看触发词 → [[SUGT_ROUTE]]skills[[/SUGT_ROUTE]]；\n\
             用户想看接入影响 / 确认接管后果 → [[SUGT_ROUTE]]preview[[/SUGT_ROUTE]]；\n\
             用户想看日志、关于、更新、外观或网络设置 → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]]；\n\
             用户想开/关桌面精灵、调大小或互动 → [[SUGT_ROUTE]]pet[[/SUGT_ROUTE]]。\n\
             标记前用一两句人话说明；标记只渲染建议卡片。\n\
             这些「改客户端官方配置 / 装技能 / 接管」场景不要自己 bash 改 ~/.claude、~/.codex 等；交给界面确认。\n\
             其余本机小事（写项目文件、查目录、联网查询并汇总、做 demo）照常 bash，不要因为有路由规则就推诿；问了就答完，不要假装记住。\n\
             若下方出现「命中技能」，可 cat 其 SKILL.md 按步骤做；装/挂载技能仍走界面路由，不要 bash 改客户端技能目录。\n\
             没有匹配到管理意图时正常聊天，不要输出任何标记。",
            cap_count = capabilities.len(),
            caps = brief_list(&cap_names, MAX_NAMES, "暂无"),
            active = brief_list(&active, MAX_NAMES, "未接管任何客户端"),
            skill_count = skills.len(),
            skills = brief_list(&skills, MAX_NAMES, "暂无"),
            repo_count = repos.len(),
            repos = brief_list(&repos, 5, "暂无"),
        )
    } else {
        format!(
            "[Local facts (provided by the app, do not guess)]\n\
             - Known clients ({cap_count}): {caps}\n\
             - Taken over: {active}\n\
             - Local skills ({skill_count}): {skills}\n\
             - Skill repos ({repo_count}): {repos}\n\
             [Management intent routing]\n\
             Managing clients / agents / takeover → emit one line [[SUGT_ROUTE]]clients[[/SUGT_ROUTE]];\n\
             Skills / GitHub install / mount / triggers → [[SUGT_ROUTE]]skills[[/SUGT_ROUTE]];\n\
             Takeover impact preview → [[SUGT_ROUTE]]preview[[/SUGT_ROUTE]];\n\
             Logs, About, updates, appearance or network → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]];\n\
             Desktop companion on/off, size or interaction → [[SUGT_ROUTE]]pet[[/SUGT_ROUTE]].\n\
             Briefly explain first; the token only renders a suggestion card.\n\
             For client-config / skill-install / takeover, do not bash ~/.claude or ~/.codex yourself — use the UI.\n\
             For ordinary local work (project files, network lookup with a real answer, demos), keep using bash; do not refuse just because routing exists; finish the ask, do not pretend you remembered it.\n\
             If a matched skill appears below, you may cat its SKILL.md and follow it; install/mount still goes through the UI — do not bash client skill dirs.\n\
             For normal conversation, emit no token at all.",
            cap_count = capabilities.len(),
            caps = brief_list(&cap_names, MAX_NAMES, "none"),
            active = brief_list(&active, MAX_NAMES, "none"),
            skill_count = skills.len(),
            skills = brief_list(&skills, MAX_NAMES, "none"),
            repo_count = repos.len(),
            repos = brief_list(&repos, 5, "none"),
        )
    }
}

/// 按用户话命中本地技能触发词（或名称），注入路径级提示——下限能力：看得见、读得了；不是技能运行时。
pub fn skill_hints_for_ask(
    zh: bool,
    ask: &str,
    store_paths: &StorePaths,
    config_dir: &Path,
) -> String {
    let ask = ask.trim();
    if ask.is_empty() {
        return String::new();
    }
    let Ok(skills) = crate::store::hub::list_local_skills(store_paths, config_dir) else {
        return String::new();
    };
    let ask_lower = ask.to_ascii_lowercase();
    let mut hits: Vec<(String, String, Vec<String>, std::path::PathBuf)> = Vec::new();
    for skill in &skills {
        let mut matched = Vec::new();
        for trigger in &skill.triggers {
            let t = trigger.trim();
            if t.is_empty() {
                continue;
            }
            if ask.contains(t) || ask_lower.contains(&t.to_ascii_lowercase()) {
                matched.push(t.to_string());
            }
        }
        if matched.is_empty() {
            let name = skill.name.trim();
            if name.chars().count() >= 2
                && (ask.contains(name) || ask_lower.contains(&name.to_ascii_lowercase()))
            {
                matched.push(name.to_string());
            }
        }
        if matched.is_empty() {
            continue;
        }
        let path = store_paths.staged_skill_dir(&skill.id).join("SKILL.md");
        let desc = skill
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(SKILL_DESC_CHARS)
            .collect::<String>();
        hits.push((skill.name.clone(), desc, matched, path));
        if hits.len() >= MAX_SKILL_HINTS {
            break;
        }
    }
    if hits.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    if zh {
        lines.push("【命中技能（只读提示；可 cat SKILL.md 执行步骤）】".to_string());
    } else {
        lines.push("[Matched skills (read-only hint; cat SKILL.md to follow steps)]".to_string());
    }
    for (name, desc, matched, path) in hits {
        let triggers = matched.join(" / ");
        if zh {
            lines.push(format!(
                "- {name}｜触发：{triggers}｜{desc}｜{path}",
                path = path.display()
            ));
        } else {
            lines.push(format!(
                "- {name} | triggers: {triggers} | {desc} | {path}",
                path = path.display()
            ));
        }
    }
    format!("\n{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> (std::path::PathBuf, StorePaths) {
        let config_dir = std::env::temp_dir().join(format!("sugt-soul-{}", std::process::id()));
        let store_paths = StorePaths {
            root: config_dir.join("store"),
            skills_dir: config_dir.join("store/skills"),
            cache_dir: config_dir.join("store/cache"),
            repos_file: config_dir.join("store/repos.toml"),
            settings_file: config_dir.join("store/settings.toml"),
            staged_meta: config_dir.join("store/staged.json"),
            mounted_meta: config_dir.join("store/mounted.json"),
        };
        (config_dir, store_paths)
    }

    #[test]
    fn brief_is_bilingual_and_keeps_client_writes_on_ui() {
        let (dir, paths) = test_paths();
        let zh = soul_brief(true, &dir, &paths, &[]);
        let en = soul_brief(false, &dir, &paths, &[]);
        assert!(zh.contains("不要自己 bash 改"));
        assert!(en.contains("do not bash"));
        assert!(!zh.contains("```bash"));
        assert!(!en.contains("```bash"));
    }

    #[test]
    fn brief_exposes_runtime_facts_without_panicking_on_missing_files() {
        let (dir, paths) = test_paths();
        let brief = soul_brief(true, &dir, &paths, &["codex".to_string()]);
        assert!(brief.contains("已知客户端"));
        assert!(brief.contains("已接管：codex"));
        assert!(brief.contains("本地技能"));
        assert!(brief.contains("技能仓库"));
    }

    #[test]
    fn brief_contains_all_route_tokens() {
        let (dir, paths) = test_paths();
        let brief = soul_brief(true, &dir, &paths, &[]);
        for route in ["clients", "skills", "preview", "settings", "pet"] {
            assert!(
                brief.contains(&format!("{ROUTE_TOKEN_PREFIX}{route}{ROUTE_TOKEN_SUFFIX}")),
                "缺少路由 {route}"
            );
        }
    }

    #[test]
    fn skill_hints_match_triggers_and_expose_path() {
        let (dir, paths) = test_paths();
        let _ = std::fs::create_dir_all(&paths.skills_dir);
        let skill_id = "local__weekly";
        let skill_dir = paths.staged_skill_dir(skill_id);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: 周报助手\ndescription: 根据本周提交生成一份简洁周报，适合周五收工时使用。\ntriggers: 生成周报, 写周报\n---\n\n## 步骤\n\n1. 收集提交\n",
        )
        .unwrap();
        if let Some(parent) = paths.staged_meta.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(
            &paths.staged_meta,
            format!(
                r#"{{"skills":{{"{skill_id}":{{"name":"周报助手","repo_id":"local","relative_path":"{skill_id}","installed_at":"2026-01-01T00:00:00Z"}}}}}}"#
            ),
        )
        .unwrap();
        let hint = skill_hints_for_ask(true, "帮我生成周报", &paths, &dir);
        assert!(hint.contains("命中技能"), "{hint}");
        assert!(hint.contains("周报助手"), "{hint}");
        assert!(hint.contains("SKILL.md"), "{hint}");
        assert!(skill_hints_for_ask(true, "今天天气怎么样", &paths, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
