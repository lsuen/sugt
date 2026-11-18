use anyhow::Result;
use chrono::{NaiveDate, Utc};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// 按天滚动：实际落盘为 `sugt.log.YYYY-MM-DD`，旧版本留下的单文件 `sugt.log` 由
/// [`latest_log_file`] 兼容读取。
pub fn init(log_file: &Path) -> Result<()> {
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let file_appender = tracing_appender::rolling::daily(
        log_file.parent().unwrap_or_else(|| Path::new(".")),
        log_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sugt.log"),
    );

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("sugt=info,tower_http=info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr).compact())
        .with(fmt::layer().with_writer(file_appender).json())
        .try_init();

    Ok(())
}

/// 找到当前最新的日志文件：同目录下以 `log_file` 文件名为前缀的文件（含原路径本身），
/// 按文件名倒序取最新。按天滚动后 `sugt.log.2026-11-18` 大于 `sugt.log`，日期后缀自然胜出。
pub fn latest_log_file(log_file: &Path) -> Option<PathBuf> {
    let dir = log_file.parent()?;
    let name = log_file.file_name()?.to_str()?;
    let mut candidates: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !file_name.starts_with(name) || !path.is_file() {
                return None;
            }
            Some((file_name, path))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.into_iter().next().map(|(_, path)| path)
}

pub fn tail(log_file: &Path, lines: usize) -> Result<Vec<String>> {
    let Some(path) = latest_log_file(log_file) else {
        return Ok(Vec::new());
    };

    let mut file = fs::File::open(path)?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    let mut result: Vec<String> = content
        .lines()
        .rev()
        .take(lines)
        .map(ToOwned::to_owned)
        .collect();
    result.reverse();
    Ok(result)
}

/// 删除超过保留期的日志文件，返回删除个数。`retention_days == 0` 表示不自动清理。
///
/// 文件日期优先取文件名后缀 `YYYY-MM-DD`；解析不出（如旧版单文件 `sugt.log`）
/// 则退回文件修改时间。单文件删除失败只告警，不中断整轮清理。
pub fn prune_logs(log_file: &Path, retention_days: u32) -> Result<usize> {
    if retention_days == 0 {
        return Ok(0);
    }
    let Some(dir) = log_file.parent() else {
        return Ok(0);
    };
    let Some(name) = log_file.file_name().and_then(|name| name.to_str()) else {
        return Ok(0);
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(0);
    };

    let cutoff = Utc::now().date_naive() - chrono::Duration::days(retention_days as i64);
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(suffix) = file_name.strip_prefix(name) else {
            continue;
        };
        // 只处理 `sugt.log` 本身与 `sugt.log.YYYY-MM-DD`，别的前缀不碰。
        let parsed = if suffix.is_empty() {
            None
        } else if let Some(rest) = suffix.strip_prefix('.') {
            NaiveDate::parse_from_str(rest, "%Y-%m-%d").ok()
        } else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_date = parsed.or_else(|| {
            entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .map(|time| chrono::DateTime::<Utc>::from(time).date_naive())
        });
        let Some(file_date) = file_date else {
            continue;
        };
        if file_date < cutoff {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(err) => {
                    tracing::warn!(error = %err, path = %path.display(), "删除过期日志失败");
                }
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sugt-logging-{}-{tag}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn today_name() -> String {
        Utc::now().date_naive().format("%Y-%m-%d").to_string()
    }

    #[test]
    fn prune_removes_only_expired_daily_files() {
        let dir = temp_dir("prune");
        let log_file = dir.join("sugt.log");
        let old = dir.join("sugt.log.2020-01-01");
        let fresh = dir.join(format!("sugt.log.{}", today_name()));
        fs::write(&old, "old").unwrap();
        fs::write(&fresh, "new").unwrap();

        assert_eq!(prune_logs(&log_file, 7).unwrap(), 1);
        assert!(!old.exists(), "超过保留期的日志应被删除");
        assert!(fresh.exists(), "保留期内的日志必须保留");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_with_zero_retention_keeps_everything() {
        let dir = temp_dir("prune-zero");
        let log_file = dir.join("sugt.log");
        let old = dir.join("sugt.log.2020-01-01");
        fs::write(&old, "old").unwrap();

        assert_eq!(prune_logs(&log_file, 0).unwrap(), 0);
        assert!(old.exists(), "0 表示不自动清理");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_log_file_picks_newest_dated_file() {
        let dir = temp_dir("latest");
        let log_file = dir.join("sugt.log");
        let old = dir.join("sugt.log.2020-01-01");
        let fresh = dir.join(format!("sugt.log.{}", today_name()));
        fs::write(&old, "old").unwrap();
        fs::write(&fresh, "new").unwrap();
        fs::write(dir.join("other.log"), "noise").unwrap();

        assert_eq!(latest_log_file(&log_file), Some(fresh.clone()));
        assert_eq!(tail(&log_file, 10).unwrap(), vec!["new".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }
}
