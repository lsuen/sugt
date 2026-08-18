//! 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗，实时展示 token 交互。
//! 窗口随配置创建/更新/销毁，位置与属性全部来自 AppConfig.overlay。
//! 透明度由页面背景 alpha 实现（窗口 transparent + 前端按配置着色）。

use crate::model::OverlayConfig;
use tauri::{
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

const LABEL: &str = "overlay";
const WIDTH: f64 = 260.0;
const HEIGHT: f64 = 76.0;

/// 依据配置创建/更新/销毁悬浮窗；返回 Result 由命令层统一转 String 错误。
pub fn apply(app: &AppHandle, cfg: &OverlayConfig) -> Result<(), String> {
    match app.get_webview_window(LABEL) {
        Some(window) if !cfg.enabled => {
            window.close().map_err(|e| format!("关闭悬浮窗失败: {e}"))?;
        }
        Some(window) => {
            apply_common(&window, cfg)?;
        }
        None if cfg.enabled => {
            let window = WebviewWindowBuilder::new(
                app,
                LABEL,
                WebviewUrl::App("overlay.html".into()),
            )
            .title("SUGT 流量")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .inner_size(WIDTH, HEIGHT)
            .position(cfg.x as f64, cfg.y as f64)
            .build()
            .map_err(|e| format!("创建悬浮窗失败: {e}"))?;
            apply_common(&window, cfg)?;
        }
        None => {}
    }
    Ok(())
}

/// 置顶与鼠标穿透：edit 模式临时关闭穿透，便于拖动窗口。
fn apply_common(
    window: &tauri::WebviewWindow,
    cfg: &OverlayConfig,
) -> Result<(), String> {
    window
        .set_ignore_cursor_events(!cfg.edit)
        .map_err(|e| format!("设置鼠标穿透失败: {e}"))?;
    window
        .set_always_on_top(true)
        .map_err(|e| format!("设置置顶失败: {e}"))?;
    window
        .set_position(PhysicalPosition::new(cfg.x, cfg.y))
        .map_err(|e| format!("移动悬浮窗失败: {e}"))?;
    Ok(())
}
