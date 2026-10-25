//! GitHub 可达性：直连 → 用户代理 → 上次成功源 → 内置/远端镜像。

use crate::store::settings::apply_github_proxy;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BUILTIN_MIRRORS: &[&str] = &[
    "https://ghfast.top",
    "https://gh-proxy.com",
    "https://mirror.ghproxy.com",
];

const REMOTE_MIRRORS_URL: &str =
    "https://raw.githubusercontent.com/lsuen/sugt/master/update-mirrors.json";

const CHANNEL_STATE_FILE: &str = "update_channel.toml";
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const FETCH_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelState {
    /// 上次成功访问 GitHub 时使用的代理前缀；空字符串表示直连。
    #[serde(default)]
    pub last_prefix: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelAttempt {
    pub label: String,
    pub prefix: String,
}

#[derive(Debug, Deserialize)]
struct RemoteMirrors {
    #[serde(default)]
    mirrors: Vec<String>,
}

pub fn state_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CHANNEL_STATE_FILE)
}

pub fn load_state(config_dir: &Path) -> ChannelState {
    let path = state_path(config_dir);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return ChannelState::default();
    };
    toml::from_str(&raw).unwrap_or_default()
}

pub fn save_state(config_dir: &Path, state: &ChannelState) -> Result<()> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("无法创建 {}", config_dir.display()))?;
    let raw = toml::to_string_pretty(state).context("序列化更新通道状态失败")?;
    std::fs::write(state_path(config_dir), raw)
        .with_context(|| format!("无法写入 {}", state_path(config_dir).display()))?;
    Ok(())
}

fn normalize_prefix(prefix: &str) -> String {
    prefix.trim().trim_end_matches('/').to_string()
}

/// 构建探测/请求顺序（已去重）。
pub fn build_attempt_order(user_prefix: &str, last_prefix: &str, extra_mirrors: &[String]) -> Vec<ChannelAttempt> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let push = |out: &mut Vec<ChannelAttempt>, seen: &mut std::collections::HashSet<String>, label: &str, prefix: &str| {
        let key = normalize_prefix(prefix);
        if !seen.insert(key.clone()) {
            return;
        }
        out.push(ChannelAttempt {
            label: label.to_string(),
            prefix: key,
        });
    };

    // 1) 上次成功源优先（加快国内二次检查）
    if !last_prefix.trim().is_empty() {
        push(&mut out, &mut seen, "last", last_prefix);
    }
    // 2) 直连
    push(&mut out, &mut seen, "direct", "");
    // 3) 用户配置
    if !user_prefix.trim().is_empty() {
        push(&mut out, &mut seen, "user", user_prefix);
    }
    // 4) 远端名单 + 内置
    for m in extra_mirrors {
        if !m.trim().is_empty() {
            push(&mut out, &mut seen, "mirror", m);
        }
    }
    for m in BUILTIN_MIRRORS {
        push(&mut out, &mut seen, "builtin", m);
    }
    out
}

pub fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(format!("SUGT-Updater/{}", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("创建 HTTP 客户端失败")
}

async fn fetch_remote_mirrors(client: &reqwest::Client, user_prefix: &str) -> Vec<String> {
    // 轻量尝试：直连 + 用户代理 + 内置镜像各试一次拿名单
    let probes = [
        "".to_string(),
        normalize_prefix(user_prefix),
        BUILTIN_MIRRORS[0].to_string(),
    ];
    for prefix in probes {
        let url = apply_github_proxy(&prefix, REMOTE_MIRRORS_URL);
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Ok(body) = resp.json::<RemoteMirrors>().await {
            return body
                .mirrors
                .into_iter()
                .map(|m| normalize_prefix(&m))
                .filter(|m| !m.is_empty())
                .collect();
        }
    }
    Vec::new()
}

/// 按通道顺序 GET JSON；成功则记住通道。
pub async fn get_json_with_fallback<T: for<'de> Deserialize<'de>>(
    config_dir: &Path,
    user_prefix: &str,
    target_url: &str,
) -> Result<(T, ChannelAttempt)> {
    let client = http_client(PROBE_TIMEOUT)?;
    let state = load_state(config_dir);
    let remote = fetch_remote_mirrors(&client, user_prefix).await;
    let attempts = build_attempt_order(user_prefix, &state.last_prefix, &remote);

    let mut errors = Vec::new();
    for attempt in attempts {
        let url = apply_github_proxy(&attempt.prefix, target_url);
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<T>().await {
                Ok(value) => {
                    let _ = save_state(
                        config_dir,
                        &ChannelState {
                            last_prefix: attempt.prefix.clone(),
                        },
                    );
                    return Ok((value, attempt));
                }
                Err(err) => errors.push(format!("{}: 解析失败 ({err})", attempt.label)),
            },
            Ok(resp) => errors.push(format!("{}: HTTP {}", attempt.label, resp.status())),
            Err(err) => errors.push(format!("{}: {err}", attempt.label)),
        }
    }
    Err(anyhow!(
        "无法访问 GitHub 检查更新。已尝试直连/代理/镜像。\n{}",
        errors.join("\n")
    ))
}

/// 按通道顺序下载二进制；优先使用 check 时已成功的 prefix。
pub async fn download_bytes_with_fallback(
    config_dir: &Path,
    user_prefix: &str,
    preferred_prefix: Option<&str>,
    target_url: &str,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<(Vec<u8>, ChannelAttempt)> {
    let client = http_client(FETCH_TIMEOUT)?;
    let state = load_state(config_dir);
    let mut preferred = preferred_prefix.unwrap_or("").to_string();
    if preferred.is_empty() {
        preferred = state.last_prefix.clone();
    }
    let remote = fetch_remote_mirrors(&client, user_prefix).await;
    let mut attempts = build_attempt_order(user_prefix, &preferred, &remote);
    // 把 preferred 挪到最前
    if !preferred.is_empty() {
        attempts.sort_by_key(|a| if a.prefix == normalize_prefix(&preferred) { 0 } else { 1 });
    }

    let mut errors = Vec::new();
    for attempt in attempts {
        let url = apply_github_proxy(&attempt.prefix, target_url);
        match download_one(&client, &url, &on_progress).await {
            Ok(bytes) => {
                let _ = save_state(
                    config_dir,
                    &ChannelState {
                        last_prefix: attempt.prefix.clone(),
                    },
                );
                return Ok((bytes, attempt));
            }
            Err(err) => errors.push(format!("{}: {err}", attempt.label)),
        }
    }
    Err(anyhow!(
        "下载更新失败。已尝试直连/代理/镜像。\n{}",
        errors.join("\n")
    ))
}

async fn download_one(
    client: &reqwest::Client,
    url: &str,
    on_progress: &impl Fn(u64, Option<u64>),
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;

    let resp = client.get(url).send().await.context("请求失败")?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let total = resp.content_length();
    let mut out = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut stream = resp.bytes_stream();
    let mut read = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("下载中断")?;
        read += chunk.len() as u64;
        out.extend_from_slice(&chunk);
        on_progress(read, total);
    }
    if out.is_empty() {
        anyhow::bail!("空响应");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_order_dedups_and_prioritizes() {
        let order = build_attempt_order(
            "https://my-proxy.example/",
            "https://ghfast.top",
            &["https://gh-proxy.com".into(), "https://ghfast.top".into()],
        );
        assert_eq!(order[0].label, "last");
        assert_eq!(order[0].prefix, "https://ghfast.top");
        assert_eq!(order[1].label, "direct");
        assert_eq!(order[2].label, "user");
        assert_eq!(order[2].prefix, "https://my-proxy.example");
        let prefixes: Vec<_> = order.iter().map(|a| a.prefix.as_str()).collect();
        assert_eq!(
            prefixes.iter().filter(|p| **p == "https://ghfast.top").count(),
            1
        );
    }
}
