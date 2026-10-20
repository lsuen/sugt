//! 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗，实时展示 token 交互。
//! 位置策略：未定位（x/y < 0）时按主屏右上角创建；启动恢复时若坐标不在任何
//! monitor 范围内（如副屏已拔），自动回主屏右上角并写回配置。
//! 尺寸/布局方向由配置驱动（width/height/layout），前端按 layout 渲染横竖排。

use crate::commands::AppRuntime;
use crate::model::OverlayConfig;
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "overlay";
const MARGIN: f64 = 16.0;

/// 主屏右上角物理坐标。前端 `outerPosition()` 和 `set_position(PhysicalPosition)` 都是物理像素；
/// Retina 上若把逻辑坐标当物理坐标用，悬浮窗会偏到屏幕内侧。
fn top_right(app: &AppHandle, width: f64) -> (i32, i32) {
    let Some(m) = app.primary_monitor().ok().flatten() else {
        return (MARGIN as i32, MARGIN as i32);
    };
    let origin = m.position();
    top_right_physical(origin.x, origin.y, m.size().width, m.scale_factor(), width)
}

fn top_right_physical(
    origin_x: i32,
    origin_y: i32,
    screen_width: u32,
    scale: f64,
    window_width: f64,
) -> (i32, i32) {
    let scale = scale.max(1.0);
    let margin_px = MARGIN * scale;
    let width_px = window_width * scale;
    let x = origin_x + ((screen_width as f64) - width_px - margin_px).max(margin_px) as i32;
    let y = origin_y + margin_px as i32;
    (x, y)
}

fn point_inside(x: i32, y: i32, origin_x: i32, origin_y: i32, width: u32, height: u32) -> bool {
    x >= origin_x
        && x < origin_x.saturating_add_unsigned(width)
        && y >= origin_y
        && y < origin_y.saturating_add_unsigned(height)
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
            point_inside(cfg.x, cfg.y, p.x, p.y, s.width, s.height)
        });
    if on {
        return Ok(());
    }
    let (x, y) = top_right(app, cfg.width);
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
            w.set_size(LogicalSize::new(cfg.width, cfg.height))
                .map_err(|e| format!("调整悬浮窗尺寸失败: {e}"))?;
            let prev = (cfg.x, cfg.y);
            ensure_on_screen(app, cfg).await?;
            if (cfg.x, cfg.y) != prev {
                w.set_position(PhysicalPosition::new(cfg.x, cfg.y))
                    .map_err(|e| format!("移动悬浮窗失败: {e}"))?;
                write_back(app, cfg).await;
            }
        }
        None if cfg.enabled => {
            ensure_on_screen(app, cfg).await?;
            let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
                .title("SUGT 流量")
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .inner_size(cfg.width, cfg.height)
                .build()
                .map_err(|e| format!("创建悬浮窗失败: {e}"))?;
            w.set_position(PhysicalPosition::new(cfg.x, cfg.y))
                .map_err(|e| format!("移动悬浮窗失败: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::{point_inside, top_right_physical};

    #[test]
    fn retina_top_right_stays_in_physical_pixels() {
        // 1512 逻辑点 × 2 = 3024 物理像素，窗口宽 280 逻辑点
        let (x, y) = top_right_physical(0, 0, 3024, 2.0, 280.0);
        assert_eq!(y, 32);
        assert_eq!(x, 3024 - 560 - 32);
    }

    #[test]
    fn scale_one_matches_logical_layout() {
        let (x, y) = top_right_physical(0, 0, 1920, 1.0, 280.0);
        assert_eq!((x, y), (1920 - 280 - 16, 16));
    }

    #[test]
    fn offscreen_marker_is_not_inside_primary() {
        assert!(!point_inside(-1, -1, 0, 0, 3024, 1964));
        assert!(point_inside(2400, 32, 0, 0, 3024, 1964));
    }
}
