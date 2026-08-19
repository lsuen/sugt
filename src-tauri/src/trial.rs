use crate::{config::AppPaths, model::TrialStatusView, product};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const STATE_FILE: &str = "trial-state.json";
const CLOCK_ROLLBACK_TOLERANCE_MINUTES: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrialMode {
    Dev,
    SelfUse,
    Public,
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
            edition: edition_id().to_string(),
            product_line: product::product_line_id().to_string(),
            product_label: product::product_line_label().to_string(),
            trial_enabled: trial_enabled(),
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
    let mode = edition_mode();

    if !trial_enabled() || !windows_public_enforced(&mode) {
        let message = match mode {
            TrialMode::Dev => "开发模式，无有效期限制".to_string(),
            TrialMode::SelfUse => "异常设计自用版，无有效期限制".to_string(),
            // macOS 公开版：无时间限制
            TrialMode::Public => "公开版（macOS 无时间限制）".to_string(),
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

    let expires_at = expires_at().ok_or_else(|| anyhow!("公开版缺少有效期配置，请重新打包"))?;
    let now = Utc::now();
    let days_remaining = (expires_at.date_naive() - now.date_naive())
        .num_days()
        .max(0);

    if now > expires_at {
        return Ok(TrialStatus {
            mode: TrialMode::Public,
            valid: false,
            status: "expired".to_string(),
            message: "当前公开版已到期，请联系作者或前往 GitHub 项目地址更新软件。".to_string(),
            build_id,
            expires_at: Some(expires_at),
            days_remaining: Some(0),
        });
    }

    let machine_hash = machine_hash();
    let state_path = paths.config_dir.join(STATE_FILE);
    let (mut state, reset) = load_trial_state(&state_path, &build_id, &machine_hash, now);

    // 换新包 / 损坏状态：按当前编译期公开版截止日重新起算，不阻塞关于页。
    if reset {
        write_state(&state_path, &state)?;
    } else if state.machine_hash != machine_hash {
        return Ok(TrialStatus {
            mode: TrialMode::Public,
            valid: false,
            status: "machine_mismatch".to_string(),
            message: "当前公开版已绑定其他电脑，请联系作者获取新版。".to_string(),
            build_id,
            expires_at: Some(expires_at),
            days_remaining: Some(days_remaining),
        });
    }

    if !reset && now + Duration::minutes(CLOCK_ROLLBACK_TOLERANCE_MINUTES) < state.last_seen_at
    {
        return Ok(TrialStatus {
            mode: TrialMode::Public,
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

    let message = "公开版".to_string();
    Ok(TrialStatus {
        mode: TrialMode::Public,
        valid: true,
        status: "valid".to_string(),
        message,
        build_id,
        expires_at: Some(expires_at),
        days_remaining: Some(days_remaining),
    })
}

/// 按编译期环境变量判定授权模式。
/// SUGT_EDITION=public → 公开版；SUGT_EDITION=self → 自用版；否则为开发模式。
fn edition_mode() -> TrialMode {
    match option_env!("SUGT_EDITION") {
        Some("public") | Some("trial") => TrialMode::Public,
        Some("self") => TrialMode::SelfUse,
        _ if has_trial_env() => TrialMode::SelfUse,
        _ => TrialMode::Dev,
    }
}

fn edition_id() -> &'static str {
    match edition_mode() {
        TrialMode::Dev => "dev",
        TrialMode::SelfUse => "self",
        TrialMode::Public => "public",
    }
}

/// 公开版仅在 Windows（exe）上强制半年有效期；macOS 公开版无时间限制。
fn windows_public_enforced(mode: &TrialMode) -> bool {
    cfg!(windows) && mode == &TrialMode::Public
}

fn fresh_state(build_id: &str, machine_hash: &str, now: DateTime<Utc>) -> TrialState {
    TrialState {
        build_id: build_id.to_string(),
        machine_hash: machine_hash.to_string(),
        first_seen_at: now,
        last_seen_at: now,
    }
}

/// 读取试用状态；文件缺失、格式损坏或 build_id 变更时自动重置为当前包。
/// 返回 `(state, reset)`，`reset=true` 表示应按新版本试用期重新起算。
fn load_trial_state(
    path: &Path,
    build_id: &str,
    machine_hash: &str,
    now: DateTime<Utc>,
) -> (TrialState, bool) {
    if !path.exists() {
        return (fresh_state(build_id, machine_hash, now), true);
    }

    match read_state(path) {
        Ok(state) if state.build_id == build_id => (state, false),
        Ok(_) => {
            // 新包覆盖安装：丢弃旧绑定，使用本包 SUGT_TRIAL_EXPIRES_AT
            let _ = archive_corrupt_state(path, "build-mismatch");
            (fresh_state(build_id, machine_hash, now), true)
        }
        Err(_) => {
            let _ = archive_corrupt_state(path, "corrupt");
            (fresh_state(build_id, machine_hash, now), true)
        }
    }
}

fn read_state(path: &Path) -> Result<TrialState> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("无法读取试用状态文件 {}", path.display()))?;
    serde_json::from_str(&raw).context("试用状态文件格式错误")
}

fn archive_corrupt_state(path: &Path, reason: &str) -> Result<()> {
    let backup = path.with_extension(format!("json.bak-{reason}"));
    let _ = fs::remove_file(&backup);
    fs::rename(path, &backup).or_else(|_| {
        fs::remove_file(path)?;
        Ok(())
    })
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
    crate::platform::windows::machine_guid()
}

/// macOS 通过 IOPlatformUUID 获取唯一机器标识
#[cfg(target_os = "macos")]
fn machine_guid() -> Option<String> {
    crate::platform::macos::machine_guid()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn machine_guid() -> Option<String> {
    None
}

impl From<TrialStatus> for TrialStatusView {
    fn from(value: TrialStatus) -> Self {
        let trial_enabled = matches!(value.mode, TrialMode::Public);
        let edition = match value.mode {
            TrialMode::Dev => "dev",
            TrialMode::SelfUse => "self",
            TrialMode::Public => "public",
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

    #[test]
    fn corrupt_state_resets_for_new_build() {
        let dir = std::env::temp_dir().join(format!(
            "sugt-trial-corrupt-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(STATE_FILE);
        fs::write(&path, "{not-json").unwrap();

        let now = Utc::now();
        let (state, reset) = load_trial_state(&path, "build-new", "machine-a", now);
        assert!(reset);
        assert_eq!(state.build_id, "build-new");
        assert_eq!(state.machine_hash, "machine-a");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_id_change_resets_state() {
        let dir = std::env::temp_dir().join(format!(
            "sugt-trial-build-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(1)
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(STATE_FILE);
        let old = TrialState {
            build_id: "old-build".into(),
            machine_hash: "machine-a".into(),
            first_seen_at: Utc::now() - Duration::days(10),
            last_seen_at: Utc::now() - Duration::days(1),
        };
        write_state(&path, &old).unwrap();

        let now = Utc::now();
        let (state, reset) = load_trial_state(&path, "new-build", "machine-a", now);
        assert!(reset);
        assert_eq!(state.build_id, "new-build");
        assert_eq!(state.first_seen_at, now);
        let _ = fs::remove_dir_all(&dir);
    }
}
