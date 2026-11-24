//! WorkBuddy `~/.workbuddy/models.json` 归属合并写入。
//! 只增删 `sugtOwner == "sugt-v1"` 的条目，用户手写项永不覆盖。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

pub const OWNER: &str = "sugt-v1";
pub const OWNER_FIELD: &str = "sugtOwner";

#[derive(Debug, Clone)]
pub struct SugtModelSpec {
    pub id: String,
    pub chat_only: bool,
}

pub fn default_models_file() -> Option<PathBuf> {
    let home = dirs_home()?;
    if let Ok(custom) = std::env::var("WORKBUDDY_CONFIG_DIR") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("models.json"));
        }
    }
    Some(home.join(".workbuddy").join("models.json"))
}

fn dirs_home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

fn is_owned(entry: &Value) -> bool {
    entry
        .get(OWNER_FIELD)
        .and_then(|v| v.as_str())
        .map(|s| s == OWNER)
        .unwrap_or(false)
}

fn merge_models_owned(
    mut document: Value,
    models: &[SugtModelSpec],
    chat_completions_url: &str,
    api_key: &str,
) -> Result<Value> {
    let is_array = document.is_array();
    let list = if is_array {
        document.as_array_mut().unwrap()
    } else {
        let models_val = document
            .get_mut("models")
            .ok_or_else(|| anyhow::anyhow!("WorkBuddy models.json 缺少 models 数组"))?;
        models_val
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("WorkBuddy models 不是数组"))?
    };

    let mut kept: Vec<Value> = list.iter().filter(|e| !is_owned(e)).cloned().collect();
    let conflicts: std::collections::HashSet<String> = kept
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    for model in models {
        if conflicts.contains(&model.id) {
            continue;
        }
        kept.push(json!({
            "id": model.id,
            "name": model.id,
            "vendor": "Custom",
            "url": chat_completions_url,
            "apiKey": api_key,
            "supportsToolCall": !model.chat_only,
            "supportsImages": false,
            OWNER_FIELD: OWNER,
        }));
    }

    if is_array {
        Ok(Value::Array(kept))
    } else {
        if let Some(obj) = document.as_object_mut() {
            obj.insert("models".into(), Value::Array(kept));
        }
        Ok(document)
    }
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("无法创建 {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.sugt.tmp");
    fs::write(&tmp, text).with_context(|| format!("无法写临时文件 {}", tmp.display()))?;
    if path.exists() {
        let backup = path.with_extension(format!(
            "json.sugt-{}.bak",
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = fs::copy(path, &backup);
        prune_backups(path, 5);
    }
    fs::rename(&tmp, path).with_context(|| format!("无法替换 {}", path.display()))?;
    Ok(())
}

fn prune_backups(file: &Path, keep: usize) {
    let Some(dir) = file.parent() else { return };
    let prefix = format!("{}.sugt-", file.file_name().and_then(|s| s.to_str()).unwrap_or("models.json"));
    let mut names: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.ends_with(".bak") {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    let drop_n = names.len().saturating_sub(keep);
    for path in names.into_iter().take(drop_n) {
        let _ = fs::remove_file(path);
    }
}

/// 写入（或创建）models.json；models 为空时仍摘掉旧自有条目。
pub fn sync_models(
    file: &Path,
    models: &[SugtModelSpec],
    chat_completions_url: &str,
    api_key: &str,
) -> Result<usize> {
    let raw = fs::read_to_string(file).unwrap_or_else(|_| "[]\n".to_string());
    let document: Value = serde_json::from_str(&raw).unwrap_or(Value::Array(vec![]));
    let merged = merge_models_owned(document, models, chat_completions_url, api_key)?;
    let count = match &merged {
        Value::Array(list) => list.iter().filter(|e| is_owned(e)).count(),
        Value::Object(map) => map
            .get("models")
            .and_then(|v| v.as_array())
            .map(|list| list.iter().filter(|e| is_owned(e)).count())
            .unwrap_or(0),
        _ => 0,
    };
    let text = serde_json::to_string_pretty(&merged)? + "\n";
    atomic_write(file, &text)?;
    Ok(count)
}

pub fn clear_owned(file: &Path) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }
    sync_models(file, &[], "", "")?;
    Ok(())
}

/// 探测漂移：文件里自有条目的 id/apiKey/url 是否仍匹配期望。
pub fn probe_drift(
    file: &Path,
    expected_models: &[SugtModelSpec],
    chat_completions_url: &str,
    api_key: &str,
) -> Result<(bool, String)> {
    if !file.exists() {
        return Ok((true, "models.json 不存在".into()));
    }
    let raw = fs::read_to_string(file)?;
    let document: Value = serde_json::from_str(&raw)?;
    let list = match &document {
        Value::Array(list) => list.clone(),
        Value::Object(map) => map
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => vec![],
    };
    let owned: Vec<&Value> = list.iter().filter(|e| is_owned(e)).collect();
    let expected_ids: std::collections::HashSet<&str> =
        expected_models.iter().map(|m| m.id.as_str()).collect();
    let mut issues = Vec::new();
    for entry in &owned {
        let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !expected_ids.contains(id) {
            issues.push(format!("多余模型 {id}"));
        }
        let key = entry.get("apiKey").and_then(|v| v.as_str()).unwrap_or("");
        if key != api_key {
            issues.push(format!("{id} 的 API Key 已改动"));
        }
        let url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url != chat_completions_url {
            issues.push(format!("{id} 的 URL 已改动"));
        }
    }
    for model in expected_models {
        if !owned
            .iter()
            .any(|e| e.get("id").and_then(|v| v.as_str()) == Some(model.id.as_str()))
        {
            issues.push(format!("缺少模型 {}", model.id));
        }
    }
    if issues.is_empty() {
        Ok((false, String::new()))
    } else {
        Ok((true, issues.join("；")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sugt-wb-{tag}-{stamp}.json"))
    }

    #[test]
    fn merge_keeps_user_entries_and_marks_owner() {
        let doc = json!([
            { "id": "mine", "name": "mine", "url": "https://x", "apiKey": "u" }
        ]);
        let models = vec![SugtModelSpec {
            id: "sutai".into(),
            chat_only: false,
        }];
        let merged = merge_models_owned(
            doc,
            &models,
            "http://127.0.0.1:9876/v1/chat/completions",
            "sugt-local-key",
        )
        .unwrap();
        let list = merged.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "mine");
        assert_eq!(list[1]["sugtOwner"], OWNER);
        assert_eq!(list[1]["supportsToolCall"], true);
    }

    #[test]
    fn sync_and_drift_roundtrip() {
        let path = temp_file("sync");
        let models = vec![SugtModelSpec {
            id: "sutai".into(),
            chat_only: true,
        }];
        let url = "http://127.0.0.1:9/v1/chat/completions";
        let key = "k1";
        sync_models(&path, &models, url, key).unwrap();
        let (drifted, _) = probe_drift(&path, &models, url, key).unwrap();
        assert!(!drifted);
        let (drifted, detail) = probe_drift(&path, &models, url, "other").unwrap();
        assert!(drifted);
        assert!(detail.contains("API Key"));
        clear_owned(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let doc: Value = serde_json::from_str(&raw).unwrap();
        assert!(doc.as_array().unwrap().iter().all(|e| !is_owned(e)));
        let _ = fs::remove_file(path);
    }
}
