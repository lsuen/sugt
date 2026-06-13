use crate::store::paths::StorePaths;
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct PluginItemView {
    pub name: String,
    pub path: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInstallGuide {
    pub client: String,
    pub title: String,
    pub summary: String,
    pub commands: Vec<String>,
    pub docs_url: Option<String>,
    pub skills_path: String,
    pub plugins_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPanelView {
    pub items: Vec<PluginItemView>,
    pub guide: PluginInstallGuide,
}

pub fn plugin_panel(client: &str) -> Result<PluginPanelView> {
    let client_key = client.trim().to_lowercase();
    let guide = install_guide(&client_key);
    let items = if client_key == "codex" {
        Vec::new()
    } else {
        list_claude_plugins()?
    };
    Ok(PluginPanelView { items, guide })
}

pub fn list_claude_plugins() -> Result<Vec<PluginItemView>> {
    let plugins_dir = StorePaths::claude_plugins_dir()?;
    if !plugins_dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    scan_plugins(&plugins_dir, &mut items)?;
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

fn install_guide(client: &str) -> PluginInstallGuide {
    if client == "codex" {
        let skills_path = StorePaths::codex_skills_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/.agents/skills".to_string());
        PluginInstallGuide {
            client: "codex".to_string(),
            title: "Codex 技能与扩展".to_string(),
            summary: "Codex 与 Claude Code 共用 SKILL.md 格式，但发现路径不同。SUGT 商店可将同一技能挂到 Codex 用户目录；插件体系与 Claude Code Marketplace 不同，请使用 Codex 官方方式管理扩展。".to_string(),
            commands: vec![
                "在 Codex CLI 中输入 /skills 浏览已安装技能".to_string(),
                "在 Codex CLI 中输入 $skill-name 显式调用技能".to_string(),
                "在终端执行：codex（进入交互后使用上述命令）".to_string(),
            ],
            docs_url: Some("https://developers.openai.com/codex/skills".to_string()),
            skills_path,
            plugins_path: None,
        }
    } else {
        let skills_path = StorePaths::claude_skills_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/.claude/skills".to_string());
        let plugins_path = StorePaths::claude_plugins_dir()
            .map(|p| p.display().to_string())
            .ok();
        PluginInstallGuide {
            client: "claude".to_string(),
            title: "Claude Code 插件安装".to_string(),
            summary: "SUGT 暂不提供插件在线安装。请在终端或 Claude Code 内使用官方命令安装插件；技能与插件独立，技能请在本页「技能」或「发现技能」管理。".to_string(),
            commands: vec![
                "在 Claude Code 交互界面输入：/plugin".to_string(),
                "在终端执行：claude plugin install <插件名或仓库>".to_string(),
                "示例：claude plugin install anthropics/claude-code".to_string(),
            ],
            docs_url: Some("https://docs.anthropic.com/en/docs/claude-code/plugins".to_string()),
            skills_path,
            plugins_path,
        }
    }
}

fn scan_plugins(dir: &Path, out: &mut Vec<PluginItemView>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let source = read_plugin_source(&path);
        out.push(PluginItemView {
            name,
            path: path.display().to_string(),
            source,
        });
    }
    Ok(())
}

fn read_plugin_source(path: &Path) -> Option<String> {
    let manifest = path.join("package.json");
    if manifest.exists() {
        if let Ok(raw) = std::fs::read_to_string(&manifest) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}
