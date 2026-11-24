//! Tauri 2 的自定义命令只注册 invoke_handler 不够，还要在 permissions 里 allow
//! 并挂到 capability，否则桌面端调用时报 `Command xxx not allowed by ACL`。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

fn quoted_words(s: &str) -> Vec<String> {
    s.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn registered_commands(root: &Path) -> BTreeSet<String> {
    let app = fs::read_to_string(root.join("src/app.rs")).expect("read app.rs");
    let start = app.find("generate_handler![").expect("generate_handler! in app.rs");
    let body = &app[start + "generate_handler![".len()..];
    let body = &body[..body.find(']').expect("closing ] of generate_handler!")];
    body.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !s.starts_with("//"))
        .map(|s| s.rsplit("::").next().unwrap().trim().to_string())
        .collect()
}

fn permission_commands(root: &Path) -> HashMap<String, Vec<String>> {
    let toml = fs::read_to_string(root.join("permissions/app-commands.toml"))
        .expect("read app-commands.toml");
    let mut map = HashMap::new();
    for block in toml.split("[[permission]]").skip(1) {
        let id_line = block
            .lines()
            .find(|l| l.trim_start().starts_with("identifier"))
            .expect("permission identifier");
        let id = quoted_words(id_line).remove(0);
        let allow = block
            .find("commands.allow")
            .map(|i| quoted_words(&block[i..]))
            .unwrap_or_default();
        map.insert(id, allow);
    }
    map
}

fn capability_permissions(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    for entry in fs::read_dir(root.join("capabilities")).expect("read capabilities") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for p in json["permissions"].as_array().into_iter().flatten() {
            if let Some(s) = p.as_str() {
                out.insert(s.to_string());
            }
        }
    }
    out
}

#[test]
fn every_registered_command_is_allowed_by_a_capability() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let perms = permission_commands(root);
    let caps = capability_permissions(root);
    let allowed: HashSet<&String> = perms
        .iter()
        .filter(|(id, _)| caps.contains(*id))
        .flat_map(|(_, cmds)| cmds)
        .collect();
    let missing: Vec<_> = registered_commands(root)
        .into_iter()
        .filter(|c| !allowed.contains(c))
        .collect();
    assert!(
        missing.is_empty(),
        "以下命令已注册但未在 permissions/app-commands.toml + capabilities 放行：{missing:?}"
    );
}
