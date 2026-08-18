//! 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗，实时展示 token 交互。
//! 位置策略：未定位（x/y < 0）时按主屏右上角创建；启动恢复时若坐标不在任何
//! monitor 范围内（如副屏已拔），自动回主屏右上角并写回配置。

use crate::commands::AppRuntime;
use crate::model::OverlayConfig;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "overlay";
const WIDTH: f64 = 260.0;
const HEIGHT: f64 = 76.0;
const MARGIN: f64 = 16.0;

/// 主屏右上角逻辑坐标（取不到主屏时兜左上）
fn top_right(app: &AppHandle) -> (i32, i32) {
    let Some(m) = app.primary_monitor().ok().flatten() else {
        return (MARGIN as i32, MARGIN as i32);
    };
    let scale = m.scale_factor().max(1.0);
    let s = m.size();
    let x = ((s.width as f64 / scale) - WIDTH - MARGIN).max(MARGIN) as i32;
    (x, MARGIN as i32)
}

/// 若 cfg.x/y 不在任一 monitor 范围内 → 修正为主屏右上角；坐标已被改写
async fn ensure_on_screen(app: &AppHandle, cfg: &mut OverlayConfig) -> Result<(), String> {
    let on = app
        .available_monitors()
        .map_err(|e| format!("读取显示器列表失败: {e}"))?
        .iter()
        .any(|m| {
            let p = m.position();
            let s = m.size();
            cfg.x >= p.x && cfg.x < p.x + s.width as i32
                && cfg.y >= p.y && cfg.y < p.y + s.height as i32
        });
    if on {
        return Ok(());
    }
    let (x, y) = top_right(app);
    cfg.x = x;
    cfg.y = y;
    Ok(())
}

/// 把 cfg 写回 runtime.config（apply 调整位置后调用）
async fn write_back(app: &AppHandle, cfg: &OverlayConfig) {
    if let Some(rt) = app.try_state::<AppRuntime>() {
        let mut guard = rt.config.write().await;
        guard.overlay = cfg.clone();
    }
}

/// 依据配置创建/更新/销毁悬浮窗；可能改写 cfg.x/y（首次右上角、副屏回退），调用方负责持久化。
pub async fn apply(app: &AppHandle, cfg: &mut OverlayConfig) -> Result<(), String> {
    match app.get_webview_window(LABEL) {
        Some(w) if !cfg.enabled => {
            w.close().map_err(|e| format!("关闭悬浮窗失败: {e}"))?;
        }
        Some(w) => {
            w.set_ignore_cursor_events(!cfg.edit)
                .map_err(|e| format!("设置鼠标穿透失败: {e}"))?;
            w.set_always_on_top(true)
                .map_err(|e| format!("设置置顶失败: {e}"))?;
            let prev = (cfg.x, cfg.y);
            ensure_on_screen(app, cfg).await?;
            if (cfg.x, cfg.y) != prev {
                w.set_position(PhysicalPosition::new(cfg.x, cfg.y))
                    .map_err(|e| format!("移动悬浮窗失败: {e}"))?;
                write_back(app, cfg).await;
            }
        }
        None if cfg.enabled => {
            let (x, y) = top_right(app);
            cfg.x = x;
            cfg.y = y;
            let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
                .title("SUGT 流量")
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .inner_size(WIDTH, HEIGHT)
                .position(x as f64, y as f64)
                .build()
                .map_err(|e| format!("创建悬浮窗失败: {e}"))?;
            w.set_ignore_cursor_events(!cfg.edit)
                .map_err(|e| format!("设置鼠标穿透失败: {e}"))?;
            w.set_always_on_top(true)
                .map_err(|e| format!("设置置顶失败: {e}"))?;
            write_back(app, cfg).await;
        }
        None => {}
    }
    Ok(())
}