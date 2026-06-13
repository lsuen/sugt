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

pub fn list_plugins() -> Result<Vec<PluginItemView>> {
    let plugins_dir = StorePaths::claude_plugins_dir()?;
    if !plugins_dir.exists() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    scan_plugins(&plugins_dir, &mut items)?;
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
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
