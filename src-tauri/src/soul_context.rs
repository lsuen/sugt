//! SOUL 只读编排层：把运行时事实注入 `system_prompt`，模型只产出意图与建议卡片标记。
//! 作者：sun / 孙文龙
//!
//! 红线（见 AGENTS.md）：SOUL 只解释、只路由，不写客户端配置、不自己下载安装技能、
//! 不扩大 shell 权限。接管 / 安装 / 挂载一律由用户点按钮走既有确定性命令 + 后果预览 + 确认。
//! 本模块全部只读；任何读取失败都降级为「暂无数据」，绝不阻断对话。

use crate::store::paths::StorePaths;
use std::path::Path;

const MAX_NAMES: usize = 12;

/// 建议卡片标记。前端解析后渲染成按钮；未知标记按普通文本处理，不影响渲染。
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
             【管理类意图路由（只读，绝不写文件）】\n\
             用户想管理客户端、发现 Agent、接管或释放接管 → 回复里放一行 [[SUGT_ROUTE]]clients[[/SUGT_ROUTE]]；\n\
             用户想管理技能、从 GitHub 地址装技能、挂载、看触发词或优化建议 → [[SUGT_ROUTE]]skills[[/SUGT_ROUTE]]；\n\
             用户想看接入影响、确认接管后果 → [[SUGT_ROUTE]]preview[[/SUGT_ROUTE]]；\n\
             用户想看日志、关于、检查更新、改外观或网络设置 → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]]。\n\
             标记前先用一两句人话说明建议，标记只用于渲染建议卡片；\n\
             这些场景**绝不能出现 bash 代码块**，也不要自己写配置、下载仓库或改文件。\n\
             接管、安装、挂载一律由用户点界面按钮完成，程序会先给后果预览再执行。\n\
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
             [Management intent routing (read-only, never write files)]\n\
             Managing clients, discovering agents, taking over or releasing → emit one line [[SUGT_ROUTE]]clients[[/SUGT_ROUTE]];\n\
             Managing skills, installing from a GitHub URL, mounting, triggers or suggestions → [[SUGT_ROUTE]]skills[[/SUGT_ROUTE]];\n\
             Reviewing takeover impact → [[SUGT_ROUTE]]preview[[/SUGT_ROUTE]];\n\
             Logs, About, updates, appearance or network settings → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]].\n\
             Briefly explain first; the token only renders a suggestion card.\n\
             In these cases **never emit a bash fence**, never write config, clone repos or edit files yourself.\n\
             Takeover, install and mount are performed only when the user clicks UI buttons, with an impact preview first.\n\
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
    fn brief_is_bilingual_and_never_requests_a_write() {
        let (dir, paths) = test_paths();
        let zh = soul_brief(true, &dir, &paths, &[]);
        let en = soul_brief(false, &dir, &paths, &[]);
        assert!(zh.contains("绝不写文件"));
        assert!(en.contains("never write files"));
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
    fn brief_contains_all_four_route_tokens() {
        let (dir, paths) = test_paths();
        let brief = soul_brief(true, &dir, &paths, &[]);
        for route in ["clients", "skills", "preview", "settings"] {
            assert!(
                brief.contains(&format!("{ROUTE_TOKEN_PREFIX}{route}{ROUTE_TOKEN_SUFFIX}")),
                "缺少路由 {route}"
            );
        }
    }
}
