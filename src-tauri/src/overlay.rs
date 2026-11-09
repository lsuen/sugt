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

/// 窗口是否完整落在某个显示器内（全部为物理像素）。
/// 只看左上角会在拔屏 / 改分辨率后留下「只剩一角可见」的悬浮窗。
fn window_fully_inside(
    x: i32,
    y: i32,
    window_w: i32,
    window_h: i32,
    origin_x: i32,
    origin_y: i32,
    screen_width: u32,
    screen_height: u32,
) -> bool {
    let right = x.saturating_add(window_w);
    let bottom = y.saturating_add(window_h);
    x >= origin_x
        && y >= origin_y
        && right <= origin_x.saturating_add_unsigned(screen_width)
        && bottom <= origin_y.saturating_add_unsigned(screen_height)
}

/// 未定位或窗口不完整落在任一 monitor 内 → 按当前模式重新定位；坐标已被改写。
/// x/y < 0 是显式哨兵（多屏 origin 可能为负，不能只靠坐标范围判断）。
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
                let scale = m.scale_factor().max(1.0);
                let window_w = (cfg.width * scale) as i32;
                let window_h = (cfg.height * scale) as i32;
                window_fully_inside(
                    cfg.x, cfg.y, window_w, window_h,
                    p.x, p.y, s.width, s.height,
                )
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
        // 用窗口中心而不是左上角选显示器：跨屏拖动时左上角可能仍落在原屏幕，
        // 按左上角吸附会把精灵拽回错误的屏幕边缘。
        let center_x = x.saturating_add(size.width as i32 / 2);
        let center_y = y.saturating_add(size.height as i32 / 2);
        let monitors = app
            .available_monitors()
            .map_err(|e| format!("读取显示器列表失败: {e}"))?;
        if let Some(m) = monitors
            .iter()
            .find(|m| {
                let p = m.position();
                let s = m.size();
                point_inside(center_x, center_y, p.x, p.y, s.width, s.height)
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

/// 自主走动：把前端插值出的目标点夹在当前显示器可用区内（物理像素）后移动窗口。
/// 前端负责「整屏随机落点 + 缓动」，后端负责「永不越屏」硬边界；本命令不写配置，
/// 走动结束后由 `pet_finish_drag` 统一持久化最终坐标。
pub fn roam_to(app: &AppHandle, x: i32, y: i32) -> Result<[i32; 2], String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let size = window
        .outer_size()
        .map_err(|e| format!("读取窗口尺寸失败: {e}"))?;
    let current = window
        .outer_position()
        .map_err(|e| format!("读取窗口位置失败: {e}"))?;
    // 按窗口中心选屏，与 snap_position 保持一致，避免跨屏时被夹回错误显示器
    let center_x = current.x.saturating_add(size.width as i32 / 2);
    let center_y = current.y.saturating_add(size.height as i32 / 2);
    let monitors = app
        .available_monitors()
        .map_err(|e| format!("读取显示器列表失败: {e}"))?;
    let picked = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            point_inside(center_x, center_y, p.x, p.y, s.width, s.height)
        })
        .map(|m| {
            let p = m.position();
            let s = m.size();
            (p.x, p.y, s.width as i32, s.height as i32, m.scale_factor())
        });
    let Some((ox, oy, sw, sh, scale)) = picked.or_else(|| {
        app.primary_monitor().ok().flatten().map(|m| {
            let p = m.position();
            let s = m.size();
            (p.x, p.y, s.width as i32, s.height as i32, m.scale_factor())
        })
    }) else {
        return Ok([current.x, current.y]);
    };
    let margin = (MARGIN * scale.max(1.0)) as i32;
    let (cx, cy) = clamp_roam_position(
        x, y, size.width as i32, size.height as i32, ox, oy, sw, sh, margin,
    );
    if (cx, cy) != (current.x, current.y) {
        window
            .set_position(PhysicalPosition::new(cx, cy))
            .map_err(|e| format!("移动悬浮窗失败: {e}"))?;
    }
    Ok([cx, cy])
}

/// 夹取纯函数：窗口四角必须留在同一显示器内，并留出 MARGIN 安全边距。
/// 抽成纯函数便于跨平台单测（无需 GUI 会话）。
fn clamp_roam_position(
    x: i32,
    y: i32,
    window_w: i32,
    window_h: i32,
    origin_x: i32,
    origin_y: i32,
    screen_w: i32,
    screen_h: i32,
    margin: i32,
) -> (i32, i32) {
    let min_x = origin_x + margin;
    let min_y = origin_y + margin;
    let max_x = (origin_x + screen_w - window_w - margin).max(min_x);
    let max_y = (origin_y + screen_h - window_h - margin).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
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
    // HUD：仅编辑态解除穿透。
    // 精灵：创建即穿透；之后的更新不要再写 ignore_cursor_events（交给前端按立绘/
    // 菜单命中框动态 pet_set_clickable）。配置热更若整窗解穿透，空白侧栏会变透明墙。
    let hud_interactive = cfg.edit && !cfg.pet_enabled;
    match app.get_webview_window(LABEL) {
        Some(w) if !cfg.enabled => {
            w.close().map_err(|e| format!("关闭悬浮窗失败: {e}"))?;
        }
        Some(w) => {
            if !cfg.pet_enabled {
                w.set_ignore_cursor_events(!hud_interactive)
                    .map_err(|e| format!("设置鼠标穿透失败: {e}"))?;
            } else if cfg.edit {
                // 精灵编辑态需要点保存/拖柄；退出编辑后由前端重新按命中框切回穿透
                w.set_ignore_cursor_events(false)
                    .map_err(|e| format!("设置鼠标穿透失败: {e}"))?;
            }
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
            // 新建窗口：精灵默认穿透；HUD 按编辑态
            let ignore = if cfg.pet_enabled {
                !cfg.edit
            } else {
                !hud_interactive
            };
            w.set_ignore_cursor_events(ignore)
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
    use super::{
        bottom_right_physical, clamp_roam_position, point_inside, top_right_physical,
        window_fully_inside,
    };

    #[test]
    fn pet_default_size_docks_on_retina() {
        // v0.9 默认紧凑窗 148×168
        let (x, y) = bottom_right_physical(0, 0, 3024, 1964, 2.0, 148.0, 168.0);
        assert_eq!(x, 3024 - 296 - 32);
        assert_eq!(y, 1964 - 336 - 32);
    }

    #[test]
    fn window_visibility_requires_all_corners_inside() {
        assert!(window_fully_inside(
            3024 - 400 - 32, 1964 - 320 - 32, 400, 320, 0, 0, 3024, 1964,
        ));
        // 分辨率变小后左上角仍在屏内、右下角越界：必须重定位
        assert!(!window_fully_inside(1500, 900, 400, 320, 0, 0, 1920, 1080));
        // 副屏 origin 为负时按物理坐标判定
        assert!(window_fully_inside(-1820, 200, 400, 320, -1920, 0, 1920, 1080));
    }

    #[test]
    fn monitor_lookup_uses_window_center() {
        // 主屏 0..1920、副屏 -1920..0：窗口左上角在副屏，中心在主屏
        let left = -80;
        let center_x = left + 400 / 2;
        assert!(!point_inside(left, 100, 0, 0, 1920, 1080), "左上角不在主屏");
        assert!(point_inside(center_x, 100, 0, 0, 1920, 1080), "中心点在主屏，吸附应跟随主屏");
    }

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

    #[test]
    fn roam_never_leaves_screen_edges() {
        // 左上越界 → 夹到安全边距
        assert_eq!(
            clamp_roam_position(-500, -500, 148, 168, 0, 0, 1920, 1080, 16),
            (16, 16)
        );
        // 右下越界 → 夹到「屏幕 - 窗口 - 边距」，四角仍完整可见
        assert_eq!(
            clamp_roam_position(9999, 9999, 148, 168, 0, 0, 1920, 1080, 16),
            (1920 - 148 - 16, 1080 - 168 - 16)
        );
    }

    #[test]
    fn roam_keeps_in_bounds_target_untouched() {
        assert_eq!(
            clamp_roam_position(800, 500, 148, 168, 0, 0, 1920, 1080, 16),
            (800, 500)
        );
    }

    #[test]
    fn roam_respects_negative_secondary_monitor_origin() {
        // 副屏 origin.x = -1920：不能被当成负坐标直接夹成 0
        assert_eq!(
            clamp_roam_position(-3000, 400, 148, 168, -1920, 200, 1920, 1080, 16),
            (-1904, 400)
        );
    }
}
