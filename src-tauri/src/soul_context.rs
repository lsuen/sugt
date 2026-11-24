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
             用户想切换模型、添加或修改上游、看模型配置 → [[SUGT_ROUTE]]models[[/SUGT_ROUTE]]；\n\
             用户想翻看完整日志 → [[SUGT_ROUTE]]logs[[/SUGT_ROUTE]]；\n\
             用户想看关于、更新、外观或网络设置 → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]]；\n\
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
             Switch model, add or edit an upstream, model config → [[SUGT_ROUTE]]models[[/SUGT_ROUTE]];\n\
             Browse the full log → [[SUGT_ROUTE]]logs[[/SUGT_ROUTE]];\n\
             About, updates, appearance or network → [[SUGT_ROUTE]]settings[[/SUGT_ROUTE]];\n\
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

/// 网关运行事实：全部来自当前配置与内存流量快照；统计只覆盖本次网关运行，重启清零。
#[derive(Debug, Clone, Default)]
pub struct GatewayFacts {
    pub listen_url: String,
    pub active_model: String,
    pub active_provider: String,
    pub enabled_providers: Vec<String>,
    pub total_providers: usize,
    pub total_requests: u64,
    pub today_requests: u64,
    pub fail_rate_percent: f32,
    pub avg_latency_ms: u64,
    pub active_requests: u64,
    /// `名称 失败/总数`，只列有失败的渠道。
    pub failing_channels: Vec<String>,
    pub log_file: String,
}

impl GatewayFacts {
    pub fn collect(
        config: &crate::model::AppConfig,
        active: Option<&crate::model::ProviderConfig>,
        stats: &crate::gateway_stats::TrafficStatsView,
        listen_url: &str,
        log_file: &Path,
    ) -> Self {
        let enabled_providers = config
            .providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.name.clone())
            .collect();
        let failing_channels = stats
            .recent_channels
            .iter()
            .filter(|c| c.failed_count > 0)
            .map(|c| format!("{} {}/{}", c.provider_name, c.failed_count, c.request_count))
            .collect();
        Self {
            listen_url: listen_url.to_string(),
            active_model: active.map(|p| p.public_model_id()).unwrap_or_default(),
            active_provider: active.map(|p| p.name.clone()).unwrap_or_default(),
            enabled_providers,
            total_providers: config.providers.len(),
            total_requests: stats.total_requests,
            today_requests: stats.today_requests,
            fail_rate_percent: stats.fail_rate_percent,
            avg_latency_ms: stats.avg_latency_ms,
            active_requests: stats.active_requests,
            failing_channels,
            log_file: log_file.display().to_string(),
        }
    }
}

/// 把网关事实渲染成提示词段落；用户问「为什么报错 / 连不上」时，模型先看这里和日志，再下结论。
pub fn gateway_facts_block(zh: bool, facts: &GatewayFacts) -> String {
    let none = if zh { "暂无" } else { "none" };
    let model = if facts.active_model.is_empty() {
        none.to_string()
    } else if facts.active_provider.is_empty() {
        facts.active_model.clone()
    } else {
        format!("{}（{}）", facts.active_model, facts.active_provider)
    };
    let enabled = brief_list(&facts.enabled_providers, MAX_NAMES, none);
    let failing = brief_list(&facts.failing_channels, 6, none);
    let log = if facts.log_file.is_empty() { none } else { facts.log_file.as_str() };
    if zh {
        let traffic = if facts.total_requests == 0 {
            "本次运行还没有请求".to_string()
        } else {
            format!(
                "本次运行累计 {} 次（今日 {}），失败率 {:.1}%，平均延迟 {} ms，在途 {}",
                facts.total_requests,
                facts.today_requests,
                facts.fail_rate_percent,
                facts.avg_latency_ms,
                facts.active_requests
            )
        };
        format!(
            "\n【网关运行（程序实时提供）】\n\
             - 监听：{listen}\n\
             - 当前模型：{model}\n\
             - 已启用上游 {enabled_count}/{total}：{enabled}\n\
             - 流量：{traffic}\n\
             - 有失败的渠道：{failing}\n\
             - 日志文件：{log}\n\
             用户问报错、连不上、变慢时，先对照上面的数字，再用 tail -n 80 读日志文件里的真实错误，按 request_id 归因后回答；不要凭印象猜原因。\n",
            listen = if facts.listen_url.is_empty() { none } else { facts.listen_url.as_str() },
            enabled_count = facts.enabled_providers.len(),
            total = facts.total_providers,
        )
    } else {
        let traffic = if facts.total_requests == 0 {
            "no requests in this run yet".to_string()
        } else {
            format!(
                "{} requests this run ({} today), {:.1}% failed, avg {} ms, {} in flight",
                facts.total_requests,
                facts.today_requests,
                facts.fail_rate_percent,
                facts.avg_latency_ms,
                facts.active_requests
            )
        };
        format!(
            "\n[Gateway runtime (live, from the app)]\n\
             - Listening: {listen}\n\
             - Current model: {model}\n\
             - Enabled upstreams {enabled_count}/{total}: {enabled}\n\
             - Traffic: {traffic}\n\
             - Channels with failures: {failing}\n\
             - Log file: {log}\n\
             When asked about errors, connection problems or slowness, check these numbers first, then tail -n 80 the log file for the real error and attribute it by request_id; do not guess.\n",
            listen = if facts.listen_url.is_empty() { none } else { facts.listen_url.as_str() },
            enabled_count = facts.enabled_providers.len(),
            total = facts.total_providers,
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
        let en = soul_brief(false, &dir, &paths, &[]);
        for route in ["clients", "skills", "preview", "models", "logs", "settings", "pet"] {
            let token = format!("{ROUTE_TOKEN_PREFIX}{route}{ROUTE_TOKEN_SUFFIX}");
            assert!(brief.contains(&token), "缺少路由 {route}");
            assert!(en.contains(&token), "missing route {route}");
        }
    }

    #[test]
    fn gateway_block_reports_real_numbers_and_log_path() {
        let facts = GatewayFacts {
            listen_url: "http://127.0.0.1:8787".into(),
            active_model: "sutai".into(),
            active_provider: "DeepSeek".into(),
            enabled_providers: vec!["DeepSeek".into(), "Kimi".into()],
            total_providers: 3,
            total_requests: 40,
            today_requests: 12,
            fail_rate_percent: 7.5,
            avg_latency_ms: 830,
            active_requests: 1,
            failing_channels: vec!["Kimi 3/9".into()],
            log_file: "/tmp/sugt.log".into(),
        };
        let zh = gateway_facts_block(true, &facts);
        assert!(zh.contains("http://127.0.0.1:8787"), "{zh}");
        assert!(zh.contains("sutai（DeepSeek）"), "{zh}");
        assert!(zh.contains("已启用上游 2/3"), "{zh}");
        assert!(zh.contains("累计 40 次（今日 12）"), "{zh}");
        assert!(zh.contains("7.5%"), "{zh}");
        assert!(zh.contains("Kimi 3/9"), "{zh}");
        assert!(zh.contains("/tmp/sugt.log"), "{zh}");
        let en = gateway_facts_block(false, &facts);
        assert!(en.contains("40 requests this run (12 today)"), "{en}");
        assert!(en.contains("tail -n 80"), "{en}");
    }

    #[test]
    fn gateway_block_without_traffic_does_not_invent_zero_rates() {
        let facts = GatewayFacts::default();
        let zh = gateway_facts_block(true, &facts);
        assert!(zh.contains("本次运行还没有请求"), "{zh}");
        assert!(!zh.contains("失败率"), "{zh}");
        assert!(zh.contains("当前模型：暂无"), "{zh}");
        let en = gateway_facts_block(false, &facts);
        assert!(en.contains("no requests in this run yet"), "{en}");
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
