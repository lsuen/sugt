use crate::{config::AppPaths, model::TrialStatusView, product};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const STATE_FILE: &str = "trial-state.json";
const CLOCK_ROLLBACK_TOLERANCE_MINUTES: i64 = 10;

#[derive(Debug, Clone)]
pub enum TrialMode {
    Dev,
    SelfUse,
    Trial,
}

#[derive(Debug, Clone)]
pub struct TrialStatus {
    pub mode: TrialMode,
    pub valid: bool,
    pub status: String,
    pub message: String,
    pub build_id: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub days_remaining: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrialState {
    build_id: String,
    machine_hash: String,
    first_seen_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

pub fn status(paths: &AppPaths) -> TrialStatusView {
    match check(paths, false) {
        Ok(status) => TrialStatusView::from(status),
        Err(err) => TrialStatusView {
            edition: "trial".to_string(),
            product_line: product::product_line_id().to_string(),
            product_label: product::product_line_label().to_string(),
            trial_enabled: true,
            valid: false,
            status: "error".to_string(),
            message: err.to_string(),
            build_id: build_id(),
            expires_at: expires_at_string(),
            expires_date: expires_date_string(),
            days_remaining: None,
        },
    }
}

pub fn ensure_allowed(paths: &AppPaths) -> Result<()> {
    let status = check(paths, true)?;
    if status.valid {
        Ok(())
    } else {
        Err(anyhow!(status.message))
    }
}

fn check(paths: &AppPaths, update_state: bool) -> Result<TrialStatus> {
    let build_id = build_id();

    if !trial_enabled() {
        let mode = if has_trial_env() {
            TrialMode::SelfUse
        } else {
            TrialMode::Dev
        };
        let message = match mode {
            TrialMode::Dev => "开发模式，无有效期限制".to_string(),
            TrialMode::SelfUse => "内部自用版本，无有效期限制".to_string(),
            TrialMode::Trial => unreachable!(),
        };
        return Ok(TrialStatus {
            mode,
            valid: true,
            status: "valid".to_string(),
            message,
            build_id,
            expires_at: None,
            days_remaining: None,
        });
    }

    let expires_at = expires_at().ok_or_else(|| anyhow!("试用版缺少有效期配置，请重新打包"))?;
    let now = Utc::now();
    let days_remaining = (expires_at.date_naive() - now.date_naive())
        .num_days()
        .max(0);

    if now > expires_at {
        return Ok(TrialStatus {
            mode: TrialMode::Trial,
            valid: false,
            status: "expired".to_string(),
            message: "当前内部试用版本已到期，请联系作者获取新版。".to_string(),
            build_id,
            expires_at: Some(expires_at),
            days_remaining: Some(0),
        });
    }

    let machine_hash = machine_hash();
    let state_path = paths.config_dir.join(STATE_FILE);
    let mut state = if state_path.exists() {
        read_state(&state_path)?
    } else {
        TrialState {
            build_id: build_id.clone(),
            machine_hash: machine_hash.clone(),
            first_seen_at: now,
            last_seen_at: now,
        }
    };

    if state.build_id != build_id {
        state = TrialState {
            build_id: build_id.clone(),
            machine_hash: machine_hash.clone(),
            first_seen_at: now,
            last_seen_at: now,
        };
    }

    if state.machine_hash != machine_hash {
        return Ok(TrialStatus {
            mode: TrialMode::Trial,
            valid: false,
            status: "machine_mismatch".to_string(),
            message: "当前内部试用版本已绑定其他电脑，请联系作者获取新版。".to_string(),
            build_id,
            expires_at: Some(expires_at),
            days_remaining: Some(days_remaining),
        });
    }

    if now + Duration::minutes(CLOCK_ROLLBACK_TOLERANCE_MINUTES) < state.last_seen_at {
        return Ok(TrialStatus {
            mode: TrialMode::Trial,
            valid: false,
            status: "clock_rollback".to_string(),
            message: "检测到系统时间异常，请校准时间后重试。".to_string(),
            build_id,
            expires_at: Some(expires_at),
            days_remaining: Some(days_remaining),
        });
    }

    if update_state || state_path.exists() {
        state.last_seen_at = now;
        write_state(&state_path, &state)?;
    }

    let message = format!("内部试用版本，有效期至 {}", expires_at.format("%Y-%m-%d"));
    Ok(TrialStatus {
        mode: TrialMode::Trial,
        valid: true,
        status: "valid".to_string(),
        message,
        build_id,
        expires_at: Some(expires_at),
        days_remaining: Some(days_remaining),
    })
}

fn read_state(path: &Path) -> Result<TrialState> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("无法读取试用状态文件 {}", path.display()))?;
    serde_json::from_str(&raw).context("试用状态文件格式错误")
}

fn write_state(path: &Path, state: &TrialState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(path, raw).with_context(|| format!("无法写入试用状态文件 {}", path.display()))
}

fn build_id() -> String {
    option_env!("SUGT_BUILD_ID").unwrap_or("dev").to_string()
}

fn has_trial_env() -> bool {
    option_env!("SUGT_TRIAL_ENABLED").is_some() || option_env!("SUGT_EDITION").is_some()
}

fn trial_enabled() -> bool {
    matches!(
        option_env!("SUGT_TRIAL_ENABLED"),
        Some("true") | Some("1") | Some("yes")
    )
}

fn expires_at() -> Option<DateTime<Utc>> {
    option_env!("SUGT_TRIAL_EXPIRES_AT")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn expires_at_string() -> Option<String> {
    option_env!("SUGT_TRIAL_EXPIRES_AT").map(ToString::to_string)
}

fn expires_date_string() -> Option<String> {
    expires_at().map(|value| value.format("%Y-%m-%d").to_string())
}

fn machine_hash() -> String {
    let mut parts = Vec::new();
    parts.push(std::env::var("COMPUTERNAME").unwrap_or_default());
    parts.push(std::env::var("USERDOMAIN").unwrap_or_default());
    parts.push(std::env::var("USERNAME").unwrap_or_default());
    if let Some(machine_guid) = machine_guid() {
        parts.push(machine_guid);
    }

    let mut hasher = Sha256::new();
    hasher.update(parts.join("|").as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(windows)]
fn machine_guid() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|line| line.contains("MachineGuid"))
        .and_then(|line| line.split_whitespace().last())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(windows))]
fn machine_guid() -> Option<String> {
    None
}

impl From<TrialStatus> for TrialStatusView {
    fn from(value: TrialStatus) -> Self {
        let trial_enabled = matches!(value.mode, TrialMode::Trial);
        let edition = match value.mode {
            TrialMode::Dev => "dev",
            TrialMode::SelfUse => "self",
            TrialMode::Trial => "trial",
        };

        Self {
            edition: edition.to_string(),
            product_line: product::product_line_id().to_string(),
            product_label: product::product_line_label().to_string(),
            trial_enabled,
            valid: value.valid,
            status: value.status,
            message: value.message,
            build_id: value.build_id,
            expires_at: value.expires_at.map(|value| value.to_rfc3339()),
            expires_date: value
                .expires_at
                .map(|value| value.format("%Y-%m-%d").to_string()),
            days_remaining: value.days_remaining,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_use_is_valid_when_trial_disabled() {
        let paths = AppPaths {
            config_dir: std::env::temp_dir().join("sugt-trial-test"),
            config_file: std::env::temp_dir()
                .join("sugt-trial-test")
                .join("config.toml"),
            log_file: std::env::temp_dir()
                .join("sugt-trial-test")
                .join("sugt.log"),
        };
        let status = status(&paths);
        assert!(status.valid);
    }
}
