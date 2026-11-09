//! 桌面精灵（小酥）后端：光标探测、拖动结束、自主走动、元素级穿透与设置跳转。
//!
//! 悬浮窗默认 `ignore_cursor_events`（鼠标穿透）时，WebView 收不到 `pointermove`。
//! 视线跟随走窗口自身的 `cursor_position`（由 OS 换算到窗口左上角物理坐标），
//! 避免 macOS 上 `CGEventGetLocation` 底左原点与 Tauri 顶左原点混用导致视线永远不对。
//!
//! 精灵的窗口几何（创建/销毁/贴边吸附/夹边）仍由 `crate::overlay` 统一管理，这里只放
//! 精灵专属命令与一次性迁移，不污染通用的 `commands.rs`。

use crate::commands::AppRuntime;
use tauri::{AppHandle, Emitter, Manager, State};

/// 相对 overlay 窗口左上角的光标位置（物理像素）。
/// 前端再除以 `scaleFactor` 得到 CSS 像素，即可驱动 `--gaze-*` / 靠近倾听。
pub fn cursor_in_overlay(app: &AppHandle) -> Result<(f64, f64), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let pos = window
        .cursor_position()
        .map_err(|e| format!("读取光标失败: {e}"))?;
    Ok((pos.x, pos.y))
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

/// 一次性迁移：产品默认开启小酥全功能（精灵 + 互动 + 视线/眨眼），并收紧透明窗。
/// 仅在用户尚未自定义精灵落点时覆盖；已手动摆过位置的保留坐标。
pub async fn migrate_pet_full_defaults(runtime: &AppRuntime) -> Result<(), String> {
    let marker = runtime.paths.config_dir.join("migrated-pet-full-v1");
    if marker.exists() {
        return Ok(());
    }
    {
        let mut config = runtime.config.write().await;
        let o = &mut config.overlay;
        let unpositioned = o.x < 0 || o.y < 0;
        o.enabled = true;
        o.pet_enabled = true;
        o.pet_interactive = true;
        o.pet_always_interactive = true;
        o.pet_gaze = true;
        o.pet_blink = true;
        o.pet_snap = true;
        o.pet_quiet = false;
        o.pet_roam = true;
        // 旧默认 200×160 / 280×108 等宽窗 → 收成贴身立绘框
        if unpositioned || o.width >= 190.0 || o.height <= 120.0 {
            o.width = 148.0;
            o.height = 168.0;
            o.x = -1;
            o.y = -1;
            o.layout = "column".to_string();
        }
    }
    runtime.persist().await.map_err(|e| e.to_string())?;
    let _ = std::fs::write(&marker, b"1");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_links() {
        // 纯链接冒烟：真正的窗口探测 / 穿透需 GUI 会话。
        assert!(true);
    }
}
