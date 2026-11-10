//! 主窗口尺寸持久化：补偿底部状态栏占用的内容区净高，并记住用户手动调整后的尺寸。
//! 作者：sun / 孙文龙
//!
//! 背景：`tauri.conf.json` 的初始高度从最早版本起一直是 680，状态栏加入后
//! `.app-shell` 改成「内容 1fr + 状态栏 auto」两行，内容区净高被吃掉 34px。
//! 因此先补到 714（680+34），再按真机视觉微调为默认 730；最小高度 674。
//! 用户手动调整后的尺寸落盘，避免每次启动都回到默认值（也避免小屏用户被迫接受 730）。
//! 只读/写一份可重建的小 JSON，任何失败都降级为配置默认值，绝不阻断启动。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{LogicalSize, WebviewWindow};

/// 与 `tauri.conf.json` 的窗口约束保持一致，改配置时两处必须同步。
const MIN_WIDTH: f64 = 960.0;
const MAX_WIDTH: f64 = 1200.0;
const MIN_HEIGHT: f64 = 674.0;
const MAX_HEIGHT: f64 = 960.0;
/// 状态栏高度，与 `styles.css` 的 `--status-bar-h` 对齐。历史补偿：680 + 34 → 714，现默认高 730。
pub const STATUS_BAR_HEIGHT: f64 = 34.0;
/// 与 `tauri.conf.json` 初始宽高一致（改配置时同步）。
pub const DEFAULT_WIDTH: f64 = 1040.0;
pub const DEFAULT_HEIGHT: f64 = 730.0;
/// 换到更小的显示器时，给系统菜单栏 / Dock 留出的余量。
const SCREEN_MARGIN: f64 = 48.0;
const STATE_FILE: &str = "window-state.json";
const SAVE_DEBOUNCE: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct WindowSize {
    width: f64,
    height: f64,
}

fn state_file(config_dir: &Path) -> PathBuf {
    config_dir.join(STATE_FILE)
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if !value.is_finite() {
        return min;
    }
    value.clamp(min, max)
}

/// 读取上次尺寸。文件缺失、损坏或字段异常一律降级为「不恢复」。
fn read_saved(config_dir: &Path) -> Option<WindowSize> {
    let raw = std::fs::read_to_string(state_file(config_dir)).ok()?;
    match serde_json::from_str::<WindowSize>(&raw) {
        Ok(size) => Some(size),
        Err(err) => {
            tracing::warn!(error = %err, "窗口尺寸记录损坏，沿用默认尺寸");
            None
        }
    }
}

/// 启动时恢复尺寸。在任何异常路径下都只影响窗口大小，不影响应用可用性。
pub fn restore(window: &WebviewWindow, config_dir: &Path) {
    let Some(saved) = read_saved(config_dir) else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0).max(0.1);
    let mut width = clamp(saved.width, MIN_WIDTH, MAX_WIDTH);
    let mut height = clamp(saved.height, MIN_HEIGHT, MAX_HEIGHT);

    // 换到更小的显示器时，避免把窗口恢复到屏幕外（菜单栏 / Dock 会占掉一部分）。
    if let Ok(Some(monitor)) = window.current_monitor() {
        let physical = monitor.size();
        let usable_w = physical.width as f64 / scale;
        let usable_h = physical.height as f64 / scale - SCREEN_MARGIN;
        if usable_w > 0.0 {
            width = width.min(usable_w.max(MIN_WIDTH));
        }
        if usable_h > 0.0 {
            height = height.min(usable_h.max(MIN_HEIGHT));
        }
    }

    match window.set_size(LogicalSize::new(width, height)) {
        Ok(()) => mark_restored(width, height),
        Err(err) => tracing::warn!(error = %err, "恢复窗口尺寸失败，沿用默认尺寸"),
    }
}

/// 落盘一份尺寸。文件可随时重建，因此不引入备份链；rename 失败时直接覆盖写。
fn persist(width: f64, height: f64, config_dir: &Path) {
    let size = WindowSize {
        width: clamp(width, MIN_WIDTH, MAX_WIDTH),
        height: clamp(height, MIN_HEIGHT, MAX_HEIGHT),
    };
    let json = match serde_json::to_string(&size) {
        Ok(json) => json,
        Err(_) => return,
    };
    let path = state_file(config_dir);
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json.as_bytes()).is_err() {
        return;
    }
    if std::fs::rename(&tmp, &path).is_err() {
        // Windows 的 rename 不能覆盖已存在文件；这份记录丢了也只影响下次恢复。
        let _ = std::fs::write(&path, json.as_bytes());
        let _ = std::fs::remove_file(&tmp);
    }
}

static LATEST: OnceLock<Mutex<Option<(f64, f64, PathBuf)>>> = OnceLock::new();
static SAVER_RUNNING: AtomicBool = AtomicBool::new(false);
/// 恢复尺寸会触发一次 Resized 事件。换到小屏时被显示器上限压小的尺寸不能反写覆盖
/// 用户在大屏保存的记录，因此短暂识别并忽略这次「回声」。
static RESTORED: OnceLock<Mutex<Option<(f64, f64, Instant)>>> = OnceLock::new();
const RESTORE_ECHO_WINDOW: Duration = Duration::from_secs(2);

fn mark_restored(width: f64, height: f64) {
    let slot = RESTORED.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut guard) => *guard = Some((width, height, Instant::now())),
        Err(poisoned) => *poisoned.into_inner() = Some((width, height, Instant::now())),
    }
}

fn is_restore_echo(width: f64, height: f64) -> bool {
    let Some(slot) = RESTORED.get() else {
        return false;
    };
    let Ok(guard) = slot.lock() else {
        return false;
    };
    match guard.as_ref() {
        Some((w, h, at)) => {
            at.elapsed() < RESTORE_ECHO_WINDOW
                && (width - w).abs() < 0.5
                && (height - h).abs() < 0.5
        }
        None => false,
    }
}

/// 由 `WindowEvent::Resized` 调用。拖动窗口会连续触发，这里只保留最后一次尺寸，
/// 由单个后台线程延时落盘，避免每帧写文件。
pub fn schedule_save(width: f64, height: f64, config_dir: PathBuf) {
    if is_restore_echo(width, height) {
        return;
    }
    let slot = LATEST.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut guard) => *guard = Some((width, height, config_dir)),
        Err(poisoned) => *poisoned.into_inner() = Some((width, height, config_dir)),
    }
    if SAVER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(SAVE_DEBOUNCE);
        let pending = LATEST
            .get()
            .and_then(|slot| slot.lock().ok())
            .and_then(|mut guard| guard.take());
        match pending {
            Some((w, h, dir)) => persist(w, h, &dir),
            None => {
                SAVER_RUNNING.store(false, Ordering::SeqCst);
                // 关闭「读到空」与「新事件写入」之间的竞态：有新值就继续跑一轮。
                let has_new = LATEST
                    .get()
                    .and_then(|slot| slot.lock().ok())
                    .is_some_and(|guard| guard.is_some());
                if has_new && !SAVER_RUNNING.swap(true, Ordering::SeqCst) {
                    continue;
                }
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sugt-window-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn clamp_rejects_non_finite_and_out_of_range() {
        assert!(!f64::NAN.is_finite());
        assert_eq!(clamp(f64::NAN, MIN_WIDTH, MAX_WIDTH), MIN_WIDTH);
        assert_eq!(clamp(10.0, MIN_WIDTH, MAX_WIDTH), MIN_WIDTH);
        assert_eq!(clamp(9999.0, MIN_WIDTH, MAX_WIDTH), MAX_WIDTH);
        assert_eq!(clamp(1040.0, MIN_WIDTH, MAX_WIDTH), 1040.0);
        assert_eq!(STATUS_BAR_HEIGHT, 34.0);
    }

    #[test]
    fn saved_size_round_trips_with_clamping() {
        let dir = temp_dir("roundtrip");
        assert!(read_saved(&dir).is_none(), "无记录时不应恢复");
        persist(1400.0, 300.0, &dir);
        let saved = read_saved(&dir).expect("应能读回记录");
        assert_eq!(saved.width, MAX_WIDTH);
        assert_eq!(saved.height, MIN_HEIGHT);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_echo_is_ignored_only_for_the_same_size() {
        mark_restored(DEFAULT_WIDTH, DEFAULT_HEIGHT);
        assert!(is_restore_echo(DEFAULT_WIDTH, DEFAULT_HEIGHT), "恢复产生的回声不应写盘");
        assert!(!is_restore_echo(1100.0, DEFAULT_HEIGHT), "用户手动改宽度必须能保存");
        assert!(!is_restore_echo(DEFAULT_WIDTH, 800.0), "用户手动改高度必须能保存");
    }

    #[test]
    fn corrupt_record_degrades_to_default() {
        let dir = temp_dir("corrupt");
        std::fs::write(state_file(&dir), "{\"width\":").unwrap();
        assert!(read_saved(&dir).is_none(), "损坏记录必须降级");
        std::fs::remove_dir_all(&dir).ok();
    }
}
