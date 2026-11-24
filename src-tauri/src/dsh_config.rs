//! DSH `~/.dsh/profiles/*/cordis.patch.yml`：合并 provider=`sugt`，不碰用户其它 provider。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::workbuddy_config::SugtModelSpec;

pub const PROVIDER_ID: &str = "sugt";
const ENTRY_ID: &str = "llm-pi-ai";
const ENTRY_NAME: &str = "@deepseek-ai/dsh-llm-pi-ai";

pub fn locate_dsh_home() -> PathBuf {
    if let Ok(home) = std::env::var("DSH_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    directories::UserDirs::new()
        .map(|u| u.home_dir().join(".dsh"))
        .unwrap_or_else(|| PathBuf::from(".dsh"))
}

/// 只认带 package.json 的 profile 目录。
pub fn list_patch_files(home: &Path) -> Vec<PathBuf> {
    let profiles = home.join("profiles");
    let Ok(entries) = fs::read_dir(&profiles) else {
        return vec![];
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !path.join("package.json").is_file() {
            continue;
        }
        out.push(path.join("cordis.patch.yml"));
    }
    out.sort();
    out
}

fn provider_value(models: &[SugtModelSpec], base_url: &str, api_key: &str) -> Value {
    let model_entries: Vec<Value> = models
        .iter()
        .map(|m| {
            let name = if m.chat_only {
                format!("{}（仅对话）", m.id)
            } else {
                m.id.clone()
            };
            json!({
                "id": m.id,
                "name": name,
                "input": ["text"],
                "contextWindow": 262144,
                "maxTokens": 32768,
            })
        })
        .collect();
    json!({
        "displayName": "SUGT",
        "api": "openai-completions",
        "baseURL": base_url.trim_end_matches('/'),
        "headers": { "Authorization": format!("Bearer {api_key}") },
        "models": model_entries,
    })
}

/// 在 patch 文档（YAML 数组）中合并或删除 sugt provider。
pub fn render_patch(text: &str, provider: Option<Value>) -> Result<String> {
    let source = if text.trim().is_empty() {
        "[]\n"
    } else {
        text
    };
    let mut doc: Value = serde_yaml::from_str(source).unwrap_or(Value::Array(vec![]));
    if !doc.is_array() {
        doc = Value::Array(vec![]);
    }
    let arr = doc.as_array_mut().unwrap();

    let entry_idx = arr.iter().position(|item| {
        item.get("id").and_then(|v| v.as_str()) == Some(ENTRY_ID)
    });

    match provider {
        None => {
            if let Some(idx) = entry_idx {
                if let Some(entry) = arr.get_mut(idx) {
                    if let Some(providers) = entry
                        .pointer_mut("/config/providers")
                        .and_then(|v| v.as_object_mut())
                    {
                        providers.remove(PROVIDER_ID);
                        if providers.is_empty() {
                            if let Some(config) = entry.get_mut("config").and_then(|c| c.as_object_mut()) {
                                config.remove("providers");
                                if config.is_empty() {
                                    entry.as_object_mut().map(|o| o.remove("config"));
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(provider_val) => {
            if let Some(idx) = entry_idx {
                let entry = &mut arr[idx];
                let config = entry
                    .as_object_mut()
                    .map(|o| o.entry("config".to_string()).or_insert_with(|| json!({})))
                    .unwrap();
                let providers = config
                    .as_object_mut()
                    .map(|o| o.entry("providers".to_string()).or_insert_with(|| json!({})))
                    .unwrap();
                if let Some(map) = providers.as_object_mut() {
                    map.insert(PROVIDER_ID.to_string(), provider_val);
                }
            } else {
                arr.push(json!({
                    "id": ENTRY_ID,
                    "name": ENTRY_NAME,
                    "config": {
                        "providers": {
                            PROVIDER_ID: provider_val
                        }
                    }
                }));
            }
        }
    }

    Ok(serde_yaml::to_string(&doc)?)
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("yml.sugt.tmp");
    fs::write(&tmp, text)?;
    if path.exists() {
        let backup = path.with_extension(format!(
            "yml.sugt-{}.bak",
            chrono::Utc::now().timestamp_millis()
        ));
        let _ = fs::copy(path, backup);
    }
    fs::rename(&tmp, path).with_context(|| format!("无法替换 {}", path.display()))?;
    Ok(())
}

pub fn sync_all(home: &Path, models: &[SugtModelSpec], base_url: &str, api_key: &str) -> Result<usize> {
    let files = list_patch_files(home);
    if files.is_empty() {
        // 尚无 DSH profile：不报错，等用户装好后再接管即可。
        return Ok(0);
    }
    let provider = provider_value(models, base_url, api_key);
    let mut n = 0;
    for file in files {
        let raw = fs::read_to_string(&file).unwrap_or_default();
        let next = render_patch(&raw, Some(provider.clone()))?;
        if next != raw {
            atomic_write(&file, &next)?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn clear_all(home: &Path) -> Result<usize> {
    let files = list_patch_files(home);
    let mut n = 0;
    for file in files {
        if !file.exists() {
            continue;
        }
        let raw = fs::read_to_string(&file).unwrap_or_default();
        let next = render_patch(&raw, None)?;
        if next != raw {
            atomic_write(&file, &next)?;
            n += 1;
        }
    }
    Ok(n)
}

pub fn probe_drift(
    home: &Path,
    expected: &[SugtModelSpec],
    base_url: &str,
    api_key: &str,
) -> Result<(bool, String)> {
    let files = list_patch_files(home);
    if files.is_empty() {
        return Ok((true, "未发现 DSH profile（~/.dsh/profiles）".into()));
    }
    let expect_auth = format!("Bearer {api_key}");
    let expect_ids: std::collections::HashSet<&str> =
        expected.iter().map(|m| m.id.as_str()).collect();
    let mut issues = Vec::new();
    for file in files {
        let raw = fs::read_to_string(&file).unwrap_or_default();
        let doc: Value = serde_yaml::from_str(&raw).unwrap_or(Value::Array(vec![]));
        let Some(arr) = doc.as_array() else {
            issues.push(format!("{} 不是数组", file.display()));
            continue;
        };
        let entry = arr.iter().find(|item| {
            item.get("id").and_then(|v| v.as_str()) == Some(ENTRY_ID)
        });
        let Some(provider) = entry.and_then(|e| e.pointer(&format!("/config/providers/{PROVIDER_ID}")))
        else {
            issues.push(format!("{} 缺少 sugt provider", file.display()));
            continue;
        };
        let base = provider.get("baseURL").and_then(|v| v.as_str()).unwrap_or("");
        if base.trim_end_matches('/') != base_url.trim_end_matches('/') {
            issues.push("baseURL 已改动".into());
        }
        let auth = provider
            .pointer("/headers/Authorization")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if auth != expect_auth {
            issues.push("API Key 已改动".into());
        }
        let models = provider
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let have: std::collections::HashSet<&str> = models
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
            .collect();
        for id in &expect_ids {
            if !have.contains(id) {
                issues.push(format!("缺少模型 {id}"));
            }
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

    #[test]
    fn render_adds_and_removes_sugt_provider() {
        let models = vec![SugtModelSpec {
            id: "sutai".into(),
            chat_only: false,
        }];
        let provider = provider_value(&models, "http://127.0.0.1:9/v1", "k");
        let text = render_patch("[]\n", Some(provider)).unwrap();
        assert!(text.contains("sugt"));
        assert!(text.contains("sutai"));
        let cleared = render_patch(&text, None).unwrap();
        assert!(!cleared.contains("sugt:") && !cleared.contains("sugt\n"));
    }

    #[test]
    fn sync_writes_profile_patch() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("sugt-dsh-{stamp}"));
        let profile = home.join("profiles").join("default");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("package.json"), "{}").unwrap();
        let models = vec![SugtModelSpec {
            id: "sutai".into(),
            chat_only: true,
        }];
        let n = sync_all(&home, &models, "http://127.0.0.1:9/v1", "key").unwrap();
        assert_eq!(n, 1);
        let (drifted, _) = probe_drift(&home, &models, "http://127.0.0.1:9/v1", "key").unwrap();
        assert!(!drifted);
        clear_all(&home).unwrap();
        let _ = fs::remove_dir_all(home);
    }
}
