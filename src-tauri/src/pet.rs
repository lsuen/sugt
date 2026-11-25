//! 桌面精灵（小酥）后端：光标探测、拖动结束、自主走动、元素级穿透与设置跳转。
//!
//! 悬浮窗默认 `ignore_cursor_events`（鼠标穿透）时，WebView 收不到 `pointermove`。
//! 视线跟随走窗口自身的 `cursor_position`（由 OS 换算到窗口左上角物理坐标），
//! 避免 macOS 上 `CGEventGetLocation` 底左原点与 Tauri 顶左原点混用导致视线永远不对。
//!
//! 精灵的窗口几何（创建/销毁/贴边吸附/夹边）仍由 `crate::overlay` 统一管理，这里只放
//! 精灵专属命令与一次性迁移，不污染通用的 `commands.rs`。

use crate::commands::AppRuntime;
use crate::model::OverlayConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{
    window::Color, AppHandle, Emitter, Manager, State, WebviewUrl, WindowEvent,
    WebviewWindowBuilder,
};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

/// 旧的宽扁流量窗。现行立绘三档和用户自定义尺寸不是这种形状。
pub(crate) fn is_legacy_hud_frame(width: f64, height: f64) -> bool {
    height <= 120.0 || (near(width, 200.0) && near(height, 160.0))
}

/// 只把旧 HUD 收成现行立绘默认尺寸。已有落点保留。
pub(crate) fn migrate_legacy_hud_frame(overlay: &mut OverlayConfig) -> bool {
    if !is_legacy_hud_frame(overlay.width, overlay.height) {
        return false;
    }
    let defaults = OverlayConfig::default();
    overlay.width = defaults.width;
    overlay.height = defaults.height;
    if overlay.x < 0 || overlay.y < 0 {
        overlay.x = -1;
        overlay.y = -1;
    }
    true
}

/// 光标相对窗口左上角的物理像素。
/// Tauri 的 `cursor_position` 是桌面坐标，不减窗口原点就会落在命中框外面，人永远点不中。
pub(crate) fn cursor_local(cursor: (f64, f64), origin: (f64, f64)) -> (f64, f64) {
    (cursor.0 - origin.0, cursor.1 - origin.1)
}

/// 相对 overlay 窗口左上角的光标位置（物理像素）。
/// 前端再除以 `scaleFactor` 得到 CSS 像素，即可驱动视线和身体命中。
pub fn cursor_in_overlay(app: &AppHandle) -> Result<(f64, f64), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let cursor = window
        .cursor_position()
        .map_err(|e| format!("读取光标失败: {e}"))?;
    let origin = window
        .outer_position()
        .map_err(|e| format!("读取窗口位置失败: {e}"))?;
    Ok(cursor_local(
        (cursor.x, cursor.y),
        (f64::from(origin.x), f64::from(origin.y)),
    ))
}

/// 元素级鼠标穿透：只有立绘/菜单拦点击时才解除穿透，空白区照常透给桌面。
/// `enabled` 为 true 时窗口可接收鼠标事件，false 时整窗穿透。
#[tauri::command]
pub fn pet_set_clickable(app: AppHandle, enabled: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    window
        .set_ignore_cursor_events(!enabled)
        .map_err(|e| format!("设置鼠标穿透失败: {e}"))
}

/// 桌面精灵只读交互：把主窗口带到对应设置子页。不修改任何配置、不执行写操作。
#[tauri::command]
pub fn open_settings_tab(app: AppHandle, tab: String) -> Result<(), String> {
    app.emit("sugt://navigate-settings", &tab)
        .map_err(|e| format!("跳转设置失败: {e}"))
}

/// 桌面精灵拖动结束：贴边吸附 + 持久化新位置 + 通知悬浮窗刷新配置。
/// 前端只需监听窗口 onMoved 防抖后调用一次，不需要再走一遍 set_overlay_config。
#[tauri::command]
pub async fn pet_finish_drag(
    runtime: State<'_, AppRuntime>,
    app: AppHandle,
    snap: bool,
) -> Result<[i32; 2], String> {
    let (x, y) = crate::overlay::snap_position(&app, snap)?;
    let cfg = {
        let mut guard = runtime.config.write().await;
        guard.overlay.x = x;
        guard.overlay.y = y;
        guard.overlay.clone()
    };
    let _ = app.emit("sugt://overlay-config-changed", &cfg);
    runtime.persist().await.map_err(|err| err.to_string())?;
    Ok([x, y])
}

/// overlay 窗口内光标（物理像素，相对窗口左上角）。
/// 穿透模式下 WebView 收不到 pointermove，精灵视线/靠近倾听靠它。
#[tauri::command]
pub fn pet_global_cursor(app: AppHandle) -> Result<[f64; 2], String> {
    let (x, y) = cursor_in_overlay(&app)?;
    Ok([x, y])
}

/// 桌面精灵自主走动：把前端插值出的目标点夹在屏幕内后移动窗口。
/// 只改窗口坐标、不写配置；走动结束后由 pet_finish_drag 统一持久化。
#[tauri::command]
pub fn pet_roam_to(app: AppHandle, x: i32, y: i32) -> Result<[i32; 2], String> {
    crate::overlay::roam_to(&app, x, y)
}

const MENU_LABEL: &str = "pet-menu";
const MENU_W: f64 = 208.0;
const MENU_H: f64 = 252.0;

/// 菜单窗放在人物窗口正上方，右边对齐。上面放不下由调用方改到脚下。
pub(crate) fn menu_origin(pet_x: f64, pet_y: f64, pet_w: f64) -> (f64, f64) {
    (pet_x + pet_w - MENU_W, pet_y - MENU_H - 6.0)
}

fn dismiss_menu(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MENU_LABEL) {
        // destroy 不走 CloseRequested。主窗口会拦住 close，菜单用 close 会留在屏幕上。
        let _ = window.destroy();
    }
    // 通知 overlay：独立菜单窗已关，穿透/视线逻辑不要再猜 DOM 里有没有 .pet-menu。
    let _ = app.emit("sugt://pet-menu-closed", ());
}

/// 右键菜单是独立小窗，不改精灵透明窗的大小和底色。
/// 已打开则 focus，不再「再点一次关掉」——Windows 上一次右键会连发
/// pointerdown + contextmenu + auxclick，toggle 会刚开就关。
// Windows 上必须走 async command：同步 command 里建 WebView2 窗口会死锁
// （CreateCoreWebView2Controller 完成回调等待主线程），表现为右键无反应。
#[tauri::command]
pub async fn pet_open_menu(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(MENU_LABEL) {
        let _ = existing.set_focus();
        let _ = app.emit("sugt://pet-menu-opened", ());
        return Ok(());
    }
    let overlay = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let origin = overlay
        .outer_position()
        .map_err(|e| format!("读取窗口位置失败: {e}"))?;
    let size = overlay
        .outer_size()
        .map_err(|e| format!("读取窗口尺寸失败: {e}"))?;
    let scale = overlay
        .scale_factor()
        .map_err(|e| format!("读取缩放失败: {e}"))?;
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let pet_x = f64::from(origin.x) / scale;
    let pet_y = f64::from(origin.y) / scale;
    let pet_w = f64::from(size.width) / scale;
    let pet_h = f64::from(size.height) / scale;
    let (mut x, mut y) = menu_origin(pet_x, pet_y, pet_w);
    if let Ok(Some(mon)) = overlay.current_monitor() {
        let ms = mon.scale_factor();
        let ms = if ms > 0.0 { ms } else { scale };
        let left = f64::from(mon.position().x) / ms;
        let top = f64::from(mon.position().y) / ms;
        let right = left + f64::from(mon.size().width) / ms;
        let bottom = top + f64::from(mon.size().height) / ms;
        if y < top {
            y = pet_y + pet_h + 6.0;
        }
        if x < left {
            x = left;
        }
        if x + MENU_W > right {
            x = right - MENU_W;
        }
        if y + MENU_H > bottom {
            y = (bottom - MENU_H).max(top);
        }
    }
    // 透明窗 + 菜单面板自带圆角实底：否则窗口不透明矩形会盖住 CSS border-radius，
    // macOS 上会看起来是方角；Windows 上透明圆角与命中也更干净。
    let window = WebviewWindowBuilder::new(&app, MENU_LABEL, WebviewUrl::App("menu.html".into()))
        .title("小su")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .accept_first_mouse(true)
        .inner_size(MENU_W, MENU_H)
        .position(x, y)
        .background_color(Color(0, 0, 0, 0))
        .build()
        .map_err(|e| format!("打开菜单失败: {e}"))?;
    let app_for_blur = app.clone();
    let opened = Instant::now();
    // Windows 上新建窗常先抛 Focused(false)；必须等真正拿到过焦点后再允许失焦关闭，
    // 否则菜单会在刚弹出时被立刻 destroy，表现为「右键没反应」。
    let saw_focus = Arc::new(AtomicBool::new(false));
    let grace = if cfg!(windows) {
        Duration::from_millis(900)
    } else {
        Duration::from_millis(400)
    };
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => {
            saw_focus.store(true, Ordering::SeqCst);
        }
        WindowEvent::Focused(false) => {
            if !saw_focus.load(Ordering::SeqCst) {
                return;
            }
            if opened.elapsed() < grace {
                return;
            }
            let app = app_for_blur.clone();
            let target = app.clone();
            let _ = app.run_on_main_thread(move || dismiss_menu(&target));
        }
        _ => {}
    });
    let _ = window.set_focus();
    let _ = app.emit("sugt://pet-menu-opened", ());
    Ok(())
}

#[tauri::command]
pub fn pet_close_menu(app: AppHandle) -> Result<(), String> {
    dismiss_menu(&app);
    Ok(())
}

/// 一次性迁移：产品默认开启小酥全功能（精灵 + 互动 + 视线/眨眼）。
/// 尺寸只收紧旧的宽扁 HUD；现行三档和已经摆好的坐标不动。
pub async fn migrate_pet_full_defaults(runtime: &AppRuntime) -> Result<(), String> {
    let marker = runtime.paths.config_dir.join("migrated-pet-full-v1");
    if marker.exists() {
        return Ok(());
    }
    {
        let mut config = runtime.config.write().await;
        let o = &mut config.overlay;
        o.enabled = true;
        o.pet_interactive = true;
        o.pet_always_interactive = true;
        o.pet_gaze = true;
        o.pet_blink = true;
        o.pet_snap = true;
        o.pet_quiet = false;
        o.pet_roam = true;
        migrate_legacy_hud_frame(o);
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let _ = std::fs::write(&marker, b"1");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_sits_above_the_pet() {
        let (x, y) = menu_origin(800.0, 400.0, 240.0);
        assert_eq!(x, 800.0 + 240.0 - 208.0);
        assert!(y < 400.0);
    }

    #[test]
    fn cursor_local_is_window_relative() {
        let (x, y) = cursor_local((1800.0, 900.0), (1600.0, 700.0));
        assert_eq!((x, y), (200.0, 200.0));
    }

    #[test]
    fn legacy_hud_frame_does_not_touch_current_presets() {
        for (width, height) in [(160.0, 136.0), (198.0, 168.0), (240.0, 204.0), (220.0, 180.0)] {
            let mut overlay = OverlayConfig::default();
            overlay.width = width;
            overlay.height = height;
            overlay.x = 40;
            overlay.y = 80;
            assert!(!migrate_legacy_hud_frame(&mut overlay), "{width}x{height}");
            assert_eq!(overlay.width, width);
            assert_eq!(overlay.height, height);
            assert_eq!((overlay.x, overlay.y), (40, 80));
        }
    }

    #[test]
    fn legacy_hud_frame_tightens_to_current_default_and_keeps_position() {
        let mut placed = OverlayConfig::default();
        placed.width = 280.0;
        placed.height = 108.0;
        placed.x = 12;
        placed.y = 34;
        assert!(migrate_legacy_hud_frame(&mut placed));
        let defaults = OverlayConfig::default();
        assert_eq!(placed.width, defaults.width);
        assert_eq!(placed.height, defaults.height);
        assert_eq!((placed.x, placed.y), (12, 34));

        let mut fresh = OverlayConfig::default();
        fresh.width = 200.0;
        fresh.height = 160.0;
        fresh.x = -1;
        fresh.y = -1;
        assert!(migrate_legacy_hud_frame(&mut fresh));
        assert_eq!(fresh.width, defaults.width);
        assert_eq!(fresh.height, defaults.height);
        assert_eq!((fresh.x, fresh.y), (-1, -1));
    }
}
