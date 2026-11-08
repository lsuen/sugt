//! 流量悬浮窗：置顶 + 半透明 + 鼠标穿透的小窗，实时展示 token 交互。
//! 位置策略：未定位（x/y < 0）时按模式分派——流量 HUD 在主屏右上角，
//! 桌面精灵在主屏右下角；启动恢复时若坐标不在任何 monitor 范围内（如副屏已拔），
//! 自动按当前模式重新定位并写回配置。
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

/// 主屏右下角物理坐标：桌面精灵默认停靠位，避开顶部菜单栏与右上角 HUD。
fn bottom_right(app: &AppHandle, width: f64, height: f64) -> (i32, i32) {
    let Some(m) = app.primary_monitor().ok().flatten() else {
        return (MARGIN as i32, MARGIN as i32);
    };
    let origin = m.position();
    bottom_right_physical(
        origin.x, origin.y, m.size().width, m.size().height,
        m.scale_factor(), width, height,
    )
}

fn bottom_right_physical(
    origin_x: i32,
    origin_y: i32,
    screen_width: u32,
    screen_height: u32,
    scale: f64,
    window_width: f64,
    window_height: f64,
) -> (i32, i32) {
    let scale = scale.max(1.0);
    let margin_px = (MARGIN * scale) as i32;
    let width_px = (window_width * scale) as i32;
    let height_px = (window_height * scale) as i32;
    let x = origin_x + ((screen_width as i32) - width_px - margin_px).max(margin_px);
    let y = origin_y + ((screen_height as i32) - height_px - margin_px).max(margin_px);
    (x, y)
}

/// 按当前模式选择默认落点：精灵右下角，原流量 HUD 仍保留右上角。
fn placement(app: &AppHandle, cfg: &OverlayConfig) -> (i32, i32) {
    if cfg.pet_enabled {
        bottom_right(app, cfg.width, cfg.height)
    } else {
        top_right(app, cfg.width)
    }
}

fn point_inside(x: i32, y: i32, origin_x: i32, origin_y: i32, width: u32, height: u32) -> bool {
    x >= origin_x
        && x < origin_x.saturating_add_unsigned(width)
        && y >= origin_y
        && y < origin_y.saturating_add_unsigned(height)
}

/// 未定位或坐标不在任一 monitor 内 → 按当前模式重新定位；坐标已被改写。
/// x/y < 0 是显式哨兵（多屏 origin 可能为负，不能只靠 point_inside 判断）。
async fn ensure_on_screen(app: &AppHandle, cfg: &mut OverlayConfig) -> Result<(), String> {
    let unpositioned = cfg.x < 0 || cfg.y < 0;
    let on = if unpositioned {
        false
    } else {
        app.available_monitors()
            .map_err(|e| format!("读取显示器列表失败: {e}"))?
            .iter()
            .any(|m| {
                let p = m.position();
                let s = m.size();
                point_inside(cfg.x, cfg.y, p.x, p.y, s.width, s.height)
            })
    };
    if on {
        return Ok(());
    }
    let (x, y) = placement(app, cfg);
    cfg.x = x;
    cfg.y = y;
    Ok(())
}

/// 拖动结束：读取窗口物理坐标，按需贴边吸附后落到新位置，返回最终坐标。
/// 参考 Codex 桌面宠物的「松手即停靠」行为，前端再据此持久化配置。
pub fn snap_position(app: &AppHandle, snap: bool) -> Result<(i32, i32), String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let pos = window
        .outer_position()
        .map_err(|e| format!("读取窗口位置失败: {e}"))?;
    let mut x = pos.x;
    let mut y = pos.y;
    if snap {
        let size = window
            .outer_size()
            .map_err(|e| format!("读取窗口尺寸失败: {e}"))?;
        let monitors = app
            .available_monitors()
            .map_err(|e| format!("读取显示器列表失败: {e}"))?;
        if let Some(m) = monitors
            .iter()
            .find(|m| {
                let p = m.position();
                let s = m.size();
                point_inside(x, y, p.x, p.y, s.width, s.height)
            })
        {
            let origin = m.position();
            let screen = m.size();
            let scale = m.scale_factor().max(1.0);
            let margin = (MARGIN * scale) as i32;
            let threshold = (32.0 * scale) as i32;
            let screen_w = screen.width as i32;
            let screen_h = screen.height as i32;
            let window_w = size.width as i32;
            let window_h = size.height as i32;
            let rel_x = x - origin.x;
            let rel_y = y - origin.y;
            let right_gap = screen_w - (rel_x + window_w);
            let bottom_gap = screen_h - (rel_y + window_h);
            // 只在离边缘 32 逻辑点内才吸附，避免用户把精灵放在屏幕中间时被强行拽走
            let next_x = if rel_x < threshold && rel_x <= right_gap {
                margin
            } else if right_gap < threshold {
                screen_w - window_w - margin
            } else {
                rel_x
            };
            let next_y = if rel_y < threshold && rel_y <= bottom_gap {
                margin
            } else if bottom_gap < threshold {
                screen_h - window_h - margin
            } else {
                rel_y
            };
            x = origin.x + next_x;
            y = origin.y + next_y;
            window
                .set_position(PhysicalPosition::new(x, y))
                .map_err(|e| format!("贴边吸附失败: {e}"))?;
        }
    }
    Ok((x, y))
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
    // 默认穿透；编辑态或精灵互动模式才接收鼠标事件。互动模式需用户显式开启，
    // 避免常驻桌面精灵遮挡下方 IDE 的点击操作。
    let interactive = cfg.edit || (cfg.pet_enabled && cfg.pet_interactive);
    match app.get_webview_window(LABEL) {
        Some(w) if !cfg.enabled => {
            w.close().map_err(|e| format!("关闭悬浮窗失败: {e}"))?;
        }
        Some(w) => {
            w.set_ignore_cursor_events(!interactive)
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
                .title(if cfg.pet_enabled { "SUGT 小su" } else { "SUGT 流量" })
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
            w.set_ignore_cursor_events(!interactive)
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
    use super::{bottom_right_physical, point_inside, top_right_physical};

    #[test]
    fn pet_default_docks_bottom_right_on_retina() {
        // 3024x1964 物理像素 / 2x，精灵窗口 120x160 逻辑点
        let (x, y) = bottom_right_physical(0, 0, 3024, 1964, 2.0, 120.0, 160.0);
        assert_eq!(x, 3024 - 240 - 32);
        assert_eq!(y, 1964 - 320 - 32);
    }

    #[test]
    fn pet_default_docks_bottom_right_at_scale_one() {
        let (x, y) = bottom_right_physical(0, 0, 1920, 1080, 1.0, 120.0, 160.0);
        assert_eq!((x, y), (1920 - 120 - 16, 1080 - 160 - 16));
    }

    #[test]
    fn pet_dock_keeps_origin_offset_for_secondary_monitor() {
        let (x, y) = bottom_right_physical(-1920, 200, 1920, 1080, 1.0, 120.0, 160.0);
        assert_eq!((x, y), (-1920 + 1920 - 120 - 16, 200 + 1080 - 160 - 16));
    }

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
