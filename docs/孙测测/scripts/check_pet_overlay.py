#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：静态校验桌面精灵链路的关键契约（源码修复点、Tauri ACL、i18n 双语、
         构建产物），把「可判定检查」固化成脚本，供每轮迭代复跑。
适用项目：SUGT（桌面精灵 小su）
创建日期：2026-11-08
复用方式：python3 docs/孙测测/scripts/check_pet_overlay.py [--root 项目根] [--out 留痕路径]；
         在 npm run build 之后执行，dist 断言才有意义。
退出码约定：0 全部通过 / 1 存在不通过项 / 2 读取或解析失败
"""

from __future__ import annotations

import argparse
import glob
import json
import re
import sys
from datetime import datetime
from pathlib import Path

# 每项：(编号, 说明, 相对路径, 必须同时出现的片段列表)
# 只检查「修复是否还在」，不复制业务逻辑断言；行为断言交给 cargo test / npm test。
# v1.1 起精灵 CSS/运行时已从 overlay.* 拆到 src/pet/；后端命令在 pet.rs 并以 pet:: 注册。
SOURCE_CHECKS = [
    ("C01", "精灵 CSS v1.0 分层与互动反馈选择器齐全", "src/pet/pet.css",
     [".pet-side", ".pet-figure", ".pet-motion", ".pet-loops", ".pet-loop", ".pet-poster",
      ".pet-blink", ".pet-gaze", ".pet-ambient", ".pet-status-bar", ".pet-status-dot",
      ".pet-bubble", ".pet-edit-bar", ".pet-thought",
      ".pet-stage.petted .pet-figure::after", ".pet-stage.roaming .pet-loops",
      ".pet-stage.pet-landed .pet-motion"]),
    ("C02", "编辑态显式恢复 pointer-events", "src/pet/pet.css",
     [".pet-stage.interactive .pet-edit-bar { pointer-events: auto; }"]),
    ("C03", "呼吸与状态一次性动效分层", "src/pet/pet.css",
     ["animation: pet-breathe var(--pet-breath-ms)", "[data-state=\"done\"] .pet-loops",
      "[data-state=\"error\"] .pet-loops", "pet-hop 0.62s ease-out 1"]),
    ("C03b", "已移除思考态无限高频摇晃", "src/pet/pet.css", ["@keyframes pet-breathe"]),
    ("C03c", "带 alpha 动画 WebP 五个状态全覆盖", "src/pet/pet.css",
     ["su-idle-loop.webp", "su-typing-loop.webp", "su-done-loop.webp",
      "su-error-loop.webp", "su-sleep-loop.webp"]),
    ("C03d", "静态帧降级与眼睑层存在", "src/pet/pet.css",
     [".pet-stage.still .pet-loop { display: none; }", "pet-stage.blinking .pet-blink"]),
    ("C04", "拖动状态机按本次按下判定", "src/pet/hooks/usePetDrag.ts",
     ["dragActive.current", "movedDuringDrag.current", "pendingDrag.current", "dragRequested.current"]),
    ("C05", "成功气泡等待在途归零", "src/pet/PetStage.tsx",
     ["activeNow.current > 0", "wasActive.current"]),
    ("C06", "网关两个入口把守卫挂到响应体", "src-tauri/src/gateway.rs",
     ["fn hold_guard_through_body", "hold_guard_through_body(response, active_guard)"]),
    ("C07", "网关在途守卫单测", "src-tauri/src/gateway/tests.rs",
     ["active_guard_lives_until_streaming_body_ends", "active_guard_releases_when_body_is_dropped"]),
    ("C08", "吸附按中心选屏 / 回位按完整窗口", "src-tauri/src/overlay.rs",
     ["let center_x", "fn window_fully_inside", "monitor_lookup_uses_window_center", "window_visibility_requires_all_corners_inside"]),
    # v0.9 起精灵设置迁到独立子页，草稿防竞态留在主设置面板，两者都要在
    ("C09", "档位记忆在精灵子页", "src/PetSettingsSection.tsx",
     ["PET_SIZE_STORAGE_KEY", "readPetSize", "localStorage.setItem"]),
    ("C09b", "设置写入防竞态仍在主面板", "src/SettingsPanel.tsx",
     ["overlayDraft", "pendingWrites"]),
    ("C11", "吸附文案 zh/en 同步为阈值口径", "src/i18n.ts",
     ["靠近屏幕边缘", "near a screen edge"]),
    ("C11b", "轻碰回应 zh/en 同步", "src/i18n.ts",
     ["'pet.bubble.poke': '嗯？我在呢～'", "'pet.bubble.poke': 'Hmm? Right here~'"]),
    # v0.9 把说明拆成主hint / 互动 / 视线 / 眨眼四条，逐条覆盖且 zh/en 同步
    ("C11c", "互动说明覆盖转头/眨眼/轻碰", "src/i18n.ts",
     ["'set.petHint': '默认开启小酥全功能", "'set.petGazeHint': '开启后会转头看鼠标",
      "'set.petInteractiveHint': '开启后可拖动", "gaze follow, near listening, and poke replies"]),
    ("C12", "悬浮窗 ACL 含全部精灵命令（含 v1.0 走动与元素穿透）",
     "src-tauri/permissions/app-commands.toml",
     ["get_overlay_config", "set_overlay_config", "overlay_start_drag", "focus_main_window",
      "pet_finish_drag", "pet_global_cursor", "pet_roam_to", "pet_set_clickable", "open_settings_tab"]),
    ("C13", "悬浮窗能力含 core:default", "src-tauri/capabilities/overlay.json", ["core:default"]),
    ("C20b", "后端走动夹边命令与跨平台单测", "src-tauri/src/overlay.rs",
     ["pub fn roam_to", "fn clamp_roam_position", "roam_never_leaves_screen_edges",
      "roam_respects_negative_secondary_monitor_origin"]),
    ("C20c", "走动命令注册与 ACL 放行", "src-tauri/src/app.rs",
     ["pet::pet_roam_to", "pet::pet_global_cursor", "pet::pet_set_clickable"]),
    ("C21", "视线以头部为轴且移动中关掉 3D/滤镜", "src/pet/pet.css",
     ["transform-origin: 55% 32%", "rotateY(calc(var(--head-x) * 9deg))",
      "filter: none", "transform: scaleX(var(--face-x))",
      "@keyframes pet-heart", "@keyframes pet-lift-float", "@keyframes pet-land"]),
    ("C22", "气泡不裁字 / 状态胶囊在脚底 / 思考云与摸头反馈", "src/pet/pet.css",
     ["width: max-content", "overflow-wrap: anywhere",
      "left: calc(50% + var(--pet-side-w) / 2)", ".pet-thought",
      ".pet-stage.petted .pet-figure::after", ".pet-stage.roaming .pet-loops"]),
    ("C23", "v1.0 互动文案 zh/en 同步", "src/i18n.ts",
     ["'pet.bubble.hello': '我来啦～'", "'pet.bubble.hello': 'Here I am~'",
      "'pet.bubble.wake': '嗯…醒啦～'", "'pet.bubble.wake': 'Mm… awake~'",
      "'set.petRoam': '允许小范围自主走动'", "'set.petRoam': 'Roam within the screen'"]),
    ("C24", "走动开关三处默认值与开关接线", "src/pet/config.ts",
     ["pet_roam: boolean", "pet_roam: true"]),
    ("C24b", "设置页走动开关与后端字段", "src/PetSettingsSection.tsx",
     ["'set.petRoam'", "pet_roam: e.target.checked", "cfg.pet_roam !== false"]),
    ("C24c", "后端配置字段与默认值", "src-tauri/src/model.rs",
     ["pub pet_roam: bool", "pet_roam: true"]),
    ("C25", "状态机动作注册表驱动 + 拖拽三段 + 自动节奏器 + 纯 reducer", "src/pet/petState.ts",
     ["export const PET_STATE_TABLE", "export const PET_POSE_TABLE",
      "export const PET_ACTION_TABLE", "export function selectPetAction",
      "export function reducePetState", "export function rhythmDelay",
      "export function shouldWake", "export function pickIdleVariant",
      "waking?: boolean", "dragging?: boolean", "landing?: boolean",
      "disappointed?: boolean", "PET_ACTIONS.map("]),
    # v1.3：拖拽三段必须一路通到渲染层与 CSS，任一环断了就退化成「复用走路姿」。
    ("C37", "拖拽三段接线：lifted / landing 专属动作接入渲染层", "src/pet/PetStage.tsx",
     ["dragPhase", "data-drag-phase={dragPhase}",
      "dragging: dragPhase === 'lifted'", "landing: dragPhase === 'landing'"]),
    ("C37b", "拖拽状态机三段与 manifest 落地时长", "src/pet/hooks/usePetDrag.ts",
     ["setDragPhase('lifted')", "setDragPhase('landing')",
      "setDragPhase('idle')", "AUTO.drag.holdMs"]),
    ("C37c", "拖拽物理感：离地提摆 + 落地缓冲", "src/pet/pet.css",
     ['data-drag-phase="lifted"', "data-drag-phase=\"landing\"",
      "@keyframes pet-lift-dangle"]),
    # v1.3.1：两层安全边 + 水平居中，防止「动作伸出框外 / 高矮胖瘦」回归。
    ("C38", "角色层内缩安全边且立绘水平居中贴底", "src/pet/pet.css",
     ["--pet-inset-x: 8px", "--pet-inset-y: 8px",
      "right: var(--pet-inset-x)", "top: var(--pet-inset-y)",
      "bottom: var(--pet-inset-y)", "background-position: center bottom",
      "overflow: hidden"]),
    ("C38b", "穿透命中框按 center bottom 反算，跟随 CSS 对齐方式", "src/pet/config.ts",
     ["originX = rect.left + (width - drawW) / 2",
      "originY = rect.bottom - drawH"]),
    # v1.3.3：漫游 = 被提起来飞过去 + 落点落地；不得回退成走路帧或抢请求态。
    # 编号从 C46 起：C39/C40 早已被 manifest v2 / 重编码脚本占用，不能复用。
    ("C46", "自主漂移映射 lifted 且只在 idle 生效", "src/pet/petState.ts",
     ["Boolean(input.roaming) && input.state === 'idle'",
      "walk: Boolean(input.walking)"]),
    ("C46b", "漂移落点播一次 landing 且不虚报请求态", "src/pet/hooks/usePetBehaviors.ts",
     ["setRoamLanding(true)", "AUTO.drag.holdMs", "roamLanding"]),
    ("C46c", "落地接线到渲染层与 CSS 缓冲", "src/pet/PetStage.tsx",
     ["roamLanding && petState === 'idle'", "roamLanding ? 'pet-landed' : ''"]),
    ("C46d", "漂移期离地提摆，不再用走路步频动画", "src/pet/pet.css",
     ["translateY(-8px)", "@keyframes pet-lift-float",
      ".pet-stage.roaming .pet-weight"]),
    # v1.3.3：生成侧四边门禁，防止源视频再次把手指/发梢/鞋底裁掉。
    ("C47", "生产线四边触边门禁与 640 宽画布", "docs/孙测测/scripts/comfyui_pet_action_series.py",
     ["def edge_touch_score(", "SUGT_PET_CANVAS\", \"640x768\"",
      "SUGT_PET_MASTER_SCALE", "三段补间均触边，标记待返工"]),
    ("C34", "前端姿势清单与 motion 来自 manifest 生成物", "src/pet/PetStage.tsx",
     ["import { PET_MOTION, PET_POSE_NAMES } from './petManifest.generated';",
      "const PET_LOOPS = PET_POSE_NAMES;", "reducePetState(",
      "data-motion={activeMotion}"]),
    ("C35", "穿透命中区按姿势 alpha 包围盒收紧", "src/pet/hooks/usePetGaze.ts",
     ["figureHitRect(", "PET_HITBOXES", "PET_IMAGE_SIZE",
      "overFigure || overMenu || dragActive.current"]),
    ("C26", "右键菜单 Esc 关闭且不整窗解穿透", "src/pet/PetStage.tsx",
     ["setMenuOpen(false)", "Escape", "onMenuDismiss"]),
    ("C27", "舞台写死零底色", "src/pet/pet.css",
     ["background: transparent !important", "background-color: transparent !important",
      "background-image: none !important"]),
    ("C27b", "入口 html/body/#root 零底色", "overlay.html",
     ["background: transparent !important", "background-color: transparent !important"]),
    ("C29", "姿势硬切且眨眼不叠全身帧", "src/pet/pet.css",
     ["transition: none", ".pet-stage.blinking .pet-loop.is-active",
      "visibility: hidden"]),
    ("C29b", "右键菜单为紧凑面板非整窗铺满", "src/pet/pet.css",
     ["left: calc(var(--pet-side-w) + 6%)", "bottom: auto"]),
    ("C30", "立绘透明度变量且舞台无边框", "src/pet/pet.css",
     ["--pet-opacity", "border: 0 !important", "outline: 0 !important",
      "box-shadow: none !important", "opacity: var(--pet-opacity)"]),
    ("C31", "透明窗关闭系统阴影", "src-tauri/src/overlay.rs",
     [".shadow(false)", "set_shadow(false)"]),
    ("C32", "立绘禁止 drop-shadow 拖影底板", "src/pet/pet.css",
     ["禁止 drop-shadow", "filter: none"]),
    # ---- v1.1：尺寸档位 / 穿透设置项 / 自动交互动作 / manifest v2 ----
    ("C36", "鼠标穿透为设置项且默认关闭", "src-tauri/src/model.rs",
     ["pub pet_click_through: bool", "pet_click_through: false", "#[serde(default)]"]),
    ("C36b", "新建窗口按穿透开关决定初始穿透", "src-tauri/src/overlay.rs",
     ["cfg.pet_click_through && !cfg.edit"]),
    ("C36c", "默认不穿透时整窗常驻可交互", "src/pet/hooks/usePetGaze.ts",
     ["clickThrough: boolean",
      "const passthrough = clickThrough && interactive && !edit",
      "if (!passthrough) setClickable(true)"]),
    ("C36d", "设置页穿透开关与前端字段接线", "src/PetSettingsSection.tsx",
     ["set.petClickThrough", "pet_click_through: e.target.checked"]),
    ("C36e", "穿透设置 zh/en 文案同步", "src/i18n.ts",
     ["'set.petClickThrough': '鼠标穿透（默认关）'",
      "'set.petClickThrough': 'Mouse click-through (off by default)'"]),
    ("C37", "尺寸档位整体下移：旧大档废弃 + 新增更小的小档", "src/pet/config.ts",
     ["width: 120,", "height: 136,", "width: 180,", "height: 204", "PET_SIZES"]),
    ("C37b", "后端默认尺寸与前端最小档一致", "src-tauri/src/model.rs",
     ["120.0", "136.0", "default_pet_width"]),
    ("C38", "自动交互动作接入舞台（庆祝 / 睡醒 / 失落 / 拖拽）", "src/pet/PetStage.tsx",
     ["welcoming: greeting", "shouldWake(", "AUTO.celebrate.holdMs",
      "setDisappointed(", "waking,", "disappointed,", "dragging,"]),
    ("C38b", "行为钩子按 manifest 节奏调度并支持随机转身", "src/pet/hooks/usePetBehaviors.ts",
     ["nextBlinkAt(", "nextFacingAt(", "nextIdleVariantAt(", "nextWanderAt(",
      "pickIdleVariant()", "setFacing((prev)"]),
    ("C39", "manifest v2：裁剪 / 逐帧时长 / 帧序 / 编码 / 自动动作", "src/assets/pet/manifest.json",
     ['"formatVersion": 2', '"cropBox"', '"frameOrder"',
      '"palette-lossless"', '"autoActions"', '"posePriority"']),
    ("C40", "素材重编码脚本与产物校验接线", "docs/孙测测/scripts/pet_reencode_assets.py",
     ["def probe_webp(", "def encode_products(", "def check_products(",
      "palette-lossless"]),
    ("C41", "活感层语义常量进 manifest（互质周期 + 递减 lerp）", "src/assets/pet/manifest.json",
     ['"liveliness"', '"breathMs": 3100', '"secondaryMs": 5300',
      '"weightMs": 7900', '"eyeLerp": 0.32', '"headLerp": 0.13',
      '"bodyLerp": 0.055']),
    ("C42", "舞台四层分层渲染 + 周期变量由 manifest 注入", "src/pet/PetStage.tsx",
     ['className="pet-weight"', 'className="pet-secondary"',
      "AUTO.liveliness.cycles.breathMs", "--pet-breath-ms",
      "--pet-secondary-deg", "--pet-weight-ms"]),
    ("C43", "视线跟随眼→头→身三层递减延迟", "src/pet/hooks/usePetGaze.ts",
     ["GAZE_LERP.eyeLerp", "GAZE_LERP.headLerp", "GAZE_LERP.bodyLerp",
      "'--head-x'", "'--body-x'", "bodyLayer.x"]),
    ("C44", "CSS 三层互质周期 / 独立 translate / 降级同步", "src/pet/pet.css",
     ["pet-weight-sway", "pet-secondary-sway",
      "animation: pet-breathe var(--pet-breath-ms)",
      "translate: calc(var(--body-x)",
      ".pet-stage.still .pet-weight",
      ".pet-weight { translate: none; }"]),
    ("C45", "ComfyUI 脚本统一走 curl 传输适配器", "docs/孙测测/scripts/comfyui_curl_http.py",
     ["def curl_urlopen(", "def install(", "class CurlResponse",
      "subprocess.run(", "--noproxy", "urllib.request.urlopen = curl_urlopen"]),
]

DIST_CSS_SELECTORS = [".pet-side", ".pet-figure", ".pet-weight", ".pet-motion",
                     ".pet-secondary", ".pet-loops", ".pet-blink",
                     ".pet-status-bar", ".pet-edit-bar",
                     ".pet-thought", "pet-heart", "pet-lift-float",
                     "pet-weight-sway", "pet-secondary-sway"]
MANIFEST_REL = Path("src") / "assets" / "pet" / "manifest.json"


def load_pet_manifest(root: Path) -> tuple[tuple[str, ...], tuple[str, ...], tuple[str, ...]]:
    """从 manifest 取姿势清单与素材清单，消灭四处硬编码的第二份真相源。

    返回 (姿势名, 全部素材名, 动画素材名)；素材名均不含 `.webp` 扩展名。
    """
    data = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    poses = tuple(pose["name"] for pose in data["poses"])
    posters = tuple(Path(pose["poster"]).stem for pose in data["poses"])
    extras = tuple(Path(name).stem for name in data.get("extraFrames", []))
    loops = tuple(Path(pose["loop"]).stem for pose in data["poses"])
    return poses, posters + extras + loops, loops


def check_manifest_source(root: Path) -> tuple[str, str, bool, str]:
    """C33：manifest 必须是唯一真相源，生成段与前端常量都必须由它派生。"""
    problems: list[str] = []
    try:
        data = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return "C33", "manifest 单一真相源接线", False, f"读取失败: {exc}"
    if data.get("formatVersion") != 2:
        problems.append("formatVersion 非 2")
    css = read(root, "src/pet/pet.css")
    if "@generated:pet-assets begin" not in css or "@generated:pet-assets end" not in css:
        problems.append("pet.css 缺生成标记")
    generated = read(root, "src/pet/petManifest.generated.ts")
    if "PET_HITBOXES" not in generated:
        problems.append("前端常量缺 PET_HITBOXES")
    for pose in data["poses"]:
        if f'{pose["name"]}: [' not in generated:
            problems.append(f'前端常量缺姿势 {pose["name"]}')
    for rel in ("docs/孙测测/scripts/gen_pet_from_manifest.py",
                "docs/孙测测/scripts/build_pet_preview.py"):
        if rel.endswith("gen_pet_from_manifest.py") and "build_css_block" not in read(root, rel):
            problems.append("生成器缺 CSS 段构建函数")
        if not rel.endswith("gen_pet_from_manifest.py") and "load_pet_manifest" not in read(root, rel):
            problems.append(f"{rel} 未读 manifest")
    return ("C33", "manifest 单一真相源接线", not problems,
            "、".join(problems[:4]) if problems
            else f"{len(data['poses'])} 个姿势、生成段与前端常量齐全")


def check_action_registry(root: Path) -> tuple[str, str, bool, str]:
    """C36：1000 帧动作注册表必须是唯一真相源，且语义/素材/优先级无复用。

    v1.3 红线：每个动作一张专属 poster + 一张专属 loop，拖拽不再复用走路姿；
    新增动作只在 manifest 追加一条，前端按 priority 降序查表，代码零改动。
    """
    try:
        data = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return "C36", "动作注册表 1000 帧契约", False, f"读取失败: {exc}"
    poses = data.get("poses", [])
    problems: list[str] = []
    registry = data.get("actionRegistry", {})
    budget = registry.get("frameBudget", {})
    total = sum(pose.get("frames", 0) for pose in poses)
    if total != 1000 or budget.get("total") != 1000:
        problems.append(f"总帧 {total} ≠ 1000")
    if not budget.get("uniqueBitmapFrames") or not registry.get("semanticUniqueness"):
        problems.append("未声明独立位图/语义唯一")
    for field in ("semantic", "priority", "poster", "loop"):
        values = [pose.get(field) for pose in poses]
        if len(set(values)) != len(poses):
            problems.append(f"{field} 存在复用")
    for pose in poses:
        order = pose.get("frameOrder", [])
        if len(order) != pose.get("frames") or set(order) != set(range(pose["frames"])):
            problems.append(f'{pose["name"]} 帧序不是独立位图排列')
        trigger = pose.get("trigger")
        if trigger != "always" and trigger != pose.get("name"):
            problems.append(f'{pose["name"]} trigger 未登记为同名 flag')
        if trigger == "always" and not pose.get("states"):
            problems.append(f'{pose["name"]} state 类动作缺 states 约束')
        if pose.get("playback") not in ("loop", "once"):
            problems.append(f'{pose["name"]} playback 非法')
    descending = [pose["name"] for pose in sorted(
        poses, key=lambda item: -item["priority"])]
    if descending != data.get("posePriority"):
        problems.append("posePriority 与 priority 降序不一致")
    names = {pose["name"] for pose in poses}
    if not {"lifted", "landing", "idle"} <= names:
        problems.append("缺拖拽三段或基态动作")
    drag = data.get("autoActions", {}).get("drag", {})
    if drag.get("pose") != "lifted" or drag.get("releasePose") != "landing":
        problems.append("autoActions.drag 未指向专属提起/落地动作")
    walk = next((pose for pose in poses if pose["name"] == "walk"), {})
    if "拖拽" in walk.get("semantic", "") or "提起" in walk.get("semantic", ""):
        problems.append("walk 仍在承担拖拽语义")
    return ("C36", "动作注册表 1000 帧 / 语义唯一 / 可扩展契约", not problems,
            "、".join(problems[:4]) if problems else
            f"{len(poses)} 动作 / {total} 帧 / 语义与素材均唯一")


def parse_args() -> argparse.Namespace:
    """解析参数，默认项目根由脚本位置推导。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(description="桌面精灵链路契约检查")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--out", type=Path, default=None, help="自定留痕路径")
    return parser.parse_args()


def read(root: Path, rel: str) -> str:
    """读取项目内文本文件，失败时抛出具名异常便于定位。"""
    path = root / rel
    return path.read_text(encoding="utf-8")


def check_sources(root: Path) -> list[tuple[str, str, bool, str]]:
    """逐项检查源码契约，返回 (编号, 说明, 是否通过, 缺失片段)。"""
    results = []
    for code, desc, rel, needles in SOURCE_CHECKS:
        try:
            text = read(root, rel)
        except OSError as exc:
            results.append((code, desc, False, f"读取失败: {exc}"))
            continue
        missing = [n for n in needles if n not in text]
        results.append((code, desc, not missing, "、".join(missing) if missing else "全部命中"))
    return results


def check_gateway_wrappers(root: Path) -> tuple[str, str, bool, str]:
    """两个代理入口都必须包装响应体，只改一个会留下半条失真的链路。"""
    text = read(root, "src-tauri/src/gateway.rs")
    count = text.count("hold_guard_through_body(response, active_guard)")
    ok = count == 2
    return "C14", "网关两个代理入口均包装响应体", ok, f"命中 {count} 处（期望 2）"


def check_no_shake(root: Path) -> tuple[str, str, bool, str]:
    """用户反馈的「在想时奇怪摇晃」必须彻底消失：不允许无限高频旋转动画。"""
    css = read(root, "src/pet/pet.css")
    bad = [needle for needle in (
        "pet-typing 0.42s ease-in-out infinite",
        "@keyframes pet-typing",
        "pet-error 0.4s ease-in-out 2",
    ) if needle in css]
    return "C16", "无限高频摇晃已移除", not bad, "、".join(bad) if bad else "无残留"


def check_no_stage_backdrop(root: Path) -> tuple[str, str, bool, str]:
    """舞台禁止光晕底板 / 地面投影；演示页虚线窗必须透明。"""
    css = read(root, "src/pet/pet.css")
    stage = read(root, "src/pet/PetStage.tsx")
    preview_builder = read(root, "docs/孙测测/scripts/build_pet_preview.py")
    problems: list[str] = []
    if ".pet-halo" in css or "pet-halo" in stage:
        problems.append("残留 pet-halo")
    if ".pet-ground" in css or "pet-ground" in stage:
        problems.append("残留 pet-ground")
    # 任何生效的 drop-shadow 都会在透明窗里裁成矩形底板并拖影（注释除外）
    in_block_comment = False
    for line in css.splitlines():
        stripped = line.strip()
        if "/*" in stripped:
            in_block_comment = True
        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            continue
        if "drop-shadow" in stripped:
            problems.append(f"残留 drop-shadow: {stripped[:60]}")
    if "background:transparent!important" not in preview_builder.replace(" ", ""):
        problems.append("预览 .win 未强制 transparent")
    if "棋盘格" not in preview_builder and "模拟壁纸" not in preview_builder:
        problems.append("预览未标明棋盘格仅为模拟壁纸")
    return ("C28", "禁止舞台底板且演示窗透明", not problems,
            "、".join(problems) if problems else "无 halo/ground，预览窗透明")


def check_pet_sizes(root: Path) -> tuple[str, str, bool, str]:
    """三档尺寸必须真实落在后端 set_overlay_config 的宽高校验范围内。

    档位定义在 src/pet/config.ts；设置页只引用 PET_SIZES，不再内联宽高字面量。
    """
    config = read(root, "src/pet/config.ts")
    backend = read(root, "src-tauri/src/commands.rs")
    block = re.search(r"export const PET_SIZES\s*=\s*\[(.*?)\]\s*as const", config, re.S)
    sizes = [(int(w), int(h)) for w, h in re.findall(
        r"width:\s*(\d+),\s*height:\s*(\d+)", block.group(1) if block else ""
    )]
    width_range = re.search(
        r"!\((\d+(?:\.\d+)?)\.\.=(\d+(?:\.\d+)?)\)\.contains\(&cfg\.width\)", backend)
    height_range = re.search(
        r"!\((\d+(?:\.\d+)?)\.\.=(\d+(?:\.\d+)?)\)\.contains\(&cfg\.height\)", backend)
    if len(sizes) != 3 or not width_range or not height_range:
        return ("C10", "三档尺寸与后端校验范围相容", False,
                f"解析失败：档位 {len(sizes)} 个 / 宽范围 {bool(width_range)} / 高范围 {bool(height_range)}")
    wlo, whi = float(width_range.group(1)), float(width_range.group(2))
    hlo, hhi = float(height_range.group(1)), float(height_range.group(2))
    detail = "、".join(f"{w}x{h}" for w, h in sizes) + f"（后端 {wlo:g}~{whi:g} / {hlo:g}~{hhi:g}）"
    bad = [f"{w}x{h}" for w, h in sizes if not (wlo <= w <= whi and hlo <= h <= hhi)]
    return ("C10", "三档尺寸与后端校验范围相容", not bad,
           detail if not bad else f"越界 {bad}；{detail}")


def check_pose_layers(root: Path) -> tuple[str, str, bool, str]:
    """v0.9 十个姿势必须各用单层背景图，且静态帧与动画素材都已落包。

    多值 background-image 是分层叠加，不是加载失败时的 fallback：带 alpha 的
    动画帧透明区域会露出下层近义态立绘，形成双重影像（PET-V09-01 回归网）。
    模块化后路径是 ../assets/pet/（相对 src/pet/pet.css）。
    """
    css = read(root, "src/pet/pet.css")
    asset_dir = root / "src" / "assets" / "pet"
    poses, _, _ = load_pet_manifest(root)
    problems: list[str] = []
    for pose in poses:
        for selector, kind, expected in (
            (rf'\.pet-loop\[data-loop="{pose}"\]', "动画层", f"su-{pose}-loop.webp"),
            (rf'\.pet-poster\[data-frame="{pose}"\]', "静态帧", f"su-{pose}.webp"),
        ):
            rule = re.search(selector + r"\s*\{([^}]*)\}", css)
            if not rule:
                problems.append(f"{pose} 缺{kind}规则")
                continue
            urls = re.findall(r"url\('(?:\.\./)*assets/pet/([A-Za-z0-9-]+\.webp)'\)", rule.group(1))
            if urls != [expected]:
                problems.append(f"{pose}{kind}={urls or '空'}（应单层 {expected}）")
        for name in (f"su-{pose}.webp", f"su-{pose}-loop.webp"):
            if not (asset_dir / name).is_file():
                problems.append(f"缺素材 {name}")
    return ("C17", "全部姿势单层背景与素材齐全", not problems,
            "、".join(problems[:4]) if problems else f"{len(poses)}/{len(poses)} 命中")


def read_pet_runtime(root: Path) -> str:
    """拼接精灵运行时相关源码，供跨文件契约断言使用。"""
    parts = (
        "src/pet/PetStage.tsx",
        "src/pet/PetContextMenu.tsx",
        "src/pet/hooks/usePetGaze.ts",
        "src/pet/hooks/usePetDrag.ts",
        "src/pet/hooks/usePetBehaviors.ts",
        "src/pet/config.ts",
    )
    return "\n".join(read(root, rel) for rel in parts)


def check_runtime_wiring(root: Path) -> tuple[str, str, bool, str]:
    """视线/眨眼/轻碰/静态帧/v1.0 互动链路已拆到 hooks，必须跨文件齐全。"""
    text = read_pet_runtime(root)
    needles = (
        "--gaze-x", "setBlinking", "setPoked", "setStill", "PET_LOOPS", "--drag-tilt",
        "let cancelled = false", "document.hidden", "GAZE_LERP.eyeLerp",
        "pickIdleVariant", "easeInOut", "setPetted", "setRoaming",
        "removeEventListener('resize', refreshScale)", "pet_set_clickable",
        # 元素级穿透：禁止 menuOpen 单独把整窗解穿透（透明墙回归）
        "overFigure || overMenu || dragActive.current",
    )
    missing = [n for n in needles if n not in text]
    return ("C15", "运行时视线/眨眼/轻碰/静态帧/v1.0 互动链路", not missing,
            "、".join(missing) if missing else "全部命中")


def check_v10_behaviors(root: Path) -> list[tuple[str, str, bool, str]]:
    """开场招呼 / 走动等 v1.0 行为接线（跨 PetStage + hooks）。"""
    text = read_pet_runtime(root)
    c19 = (
        "setGreeting", "'pet.bubble.hello'", "'pet.bubble.wake'", "playIdlePose",
        "lastUserTouch.current", "pokeCount.current >= 3", "PET_IDLE_BUBBLES",
    )
    c20 = (
        "'pet_roam_to'", "cfg.pet_roam === false", "easeInOut(progress)",
        "pet_finish_drag', { snap: false }", "currentMonitor", "pickTarget",
    )
    miss19 = [n for n in c19 if n not in text]
    miss20 = [n for n in c20 if n not in text]
    return [
        ("C19", "开场招呼 / 闲置差分 / 唤醒 / 连点庆祝已接线", not miss19,
         "、".join(miss19) if miss19 else "全部命中"),
        ("C20", "自主走动前端缓动与后端夹边双向接线", not miss20,
         "、".join(miss20) if miss20 else "全部命中"),
    ]


def check_v09_wiring(root: Path) -> tuple[str, str, bool, str]:
    """全局光标链路与精灵设置子页四处必须接线齐全，缺一处视线跟随静默失效。"""
    targets = (
        ("src-tauri/permissions/app-commands.toml", "pet_global_cursor", "ACL 未放行"),
        ("src-tauri/src/app.rs", "pet::pet_global_cursor", "命令未注册"),
        ("src/pet/hooks/usePetGaze.ts", "invoke<[number, number]>('pet_global_cursor')", "前端未轮询"),
        ("src/SettingsPanel.tsx", "{ id: 'pet', key: 'set.tabPet' }", "设置缺精灵子页"),
    )
    missing = [label for rel, needle, label in targets if needle not in read(root, rel)]
    return ("C18", "全局光标与精灵设置子页接线", not missing,
            "、".join(missing) if missing else "4/4 命中")


def check_dist(root: Path) -> list[tuple[str, str, bool, str]]:
    """检查生产构建产物：CSS 类名、JS 命令名、进包素材。"""
    results = []
    css_files = glob.glob(str(root / "dist" / "assets" / "overlay-*.css"))
    if not css_files:
        results.append(("D01", "dist 精灵 CSS 存在", False, "未找到 dist/assets/overlay-*.css"))
    else:
        css = Path(css_files[-1]).read_text(encoding="utf-8")
        missing = [s for s in DIST_CSS_SELECTORS if s not in css]
        results.append(("D01", "dist 精灵 CSS 含 v1.0 布局与互动类名", not missing,
                        "、".join(missing) if missing else f"全部命中（{len(css)} B）"))
        results.append(("D02", "dist CSS 含编辑态命中修复", "pointer-events:auto" in css.replace(" ", ""),
                        "压缩后仍保留 pointer-events:auto"))

    js_files = glob.glob(str(root / "dist" / "assets" / "overlay-*.js"))
    if not js_files:
        results.append(("D03", "dist 精灵 JS 存在", False, "未找到 dist/assets/overlay-*.js"))
    else:
        js = Path(js_files[-1]).read_text(encoding="utf-8")
        # 设置页文案属于 main chunk，overlay 入口只保留自己引用到的 i18n 键；
        # 这里只断言 overlay 入口必须有的键，设置键另由 D09 在主包里核对。
        missing = [s for s in ("pet_finish_drag", "focus_main_window", "active_requests",
                               "pet_global_cursor", "pet_roam_to", "pet.bubble.hello",
                               "pet.bubble.idle1") if s not in js]
        results.append(("D03", "dist JS 含拖动收尾/聚焦/在途/全局光标/走动/互动文案", not missing,
                        "、".join(missing) if missing else "全部命中"))

    assets = sorted((root / "dist" / "assets").glob("su-*.webp"))
    total = sum(p.stat().st_size for p in assets)
    _, asset_names, loops = load_pet_manifest(root)
    extras = len(json.loads((root / MANIFEST_REL).read_text(
        encoding="utf-8")).get("extraFrames", []))
    names_ok = all(any(p.name.startswith(name + "-") for p in assets)
                   for name in asset_names)
    results.append((
        "D04",
        f"dist 内含 {len(asset_names)} 张精灵 WebP"
        f"（{len(asset_names) - len(loops) - extras} 静态 poster + "
        f"{len(loops)} 动画 loop + {extras} 附加帧）",
        len(assets) == len(asset_names) and names_ok,
        f"{len(assets)} 张 / {total / 1024:.2f} KB"))
    return results


def check_main_chunk_v10(root: Path) -> tuple[str, str, bool, str]:
    """设置页的走动开关必须真的进主包：源码有但产物没有等于功能没上线。"""
    files = glob.glob(str(root / "dist" / "assets" / "main-*.js"))
    if not files:
        return ("D09", "dist 主包含 v1.0 设置开关", False, "未找到 dist/assets/main-*.js")
    js = Path(files[-1]).read_text(encoding="utf-8")
    missing = [s for s in ("set.petRoam", "pet_roam") if s not in js]
    return ("D09", "dist 主包含 v1.0 设置开关", not missing,
            "、".join(missing) if missing else "全部命中")


def check_preview(root: Path) -> tuple[str, str, bool, str]:
    """预览页必须由同源 CSS 生成：页内记录的 hash 与当前源 CSS 一致。"""
    import hashlib

    css = read(root, "src/pet/pet.css")
    expected = hashlib.sha256(css.encode("utf-8")).hexdigest()[:12]
    preview = read(root, "docs/design/pet-v07-preview.html")
    found = re.search(r"源 CSS sha256 ([0-9a-f]{12})", preview)
    actual = found.group(1) if found else "未找到"
    return "D05", "预览页与源 CSS 同源", actual == expected, f"源 {expected} / 预览页 {actual}"


def check_preview_interaction(root: Path) -> tuple[str, str, bool, str]:
    """演示页必须同文档渲染 + 触发式动作引擎，防止「按钮只闪一下」回归。

    旧版把精灵塞进 srcdoc iframe，父页靠 contentDocument 跨文档访问：预览面板
    若给 iframe 一个 opaque origin 或拦截内联脚本，按钮只会亮一下、角色毫无变化。
    同时旧版动作是 0.3~0.7s 一次性动画且为常驻开关，播完静止、再点变成关闭。
    """
    html = read(root, "docs/design/pet-v07-preview.html")
    problems = []
    if "<iframe" in html:
        problems.append("仍在用 iframe 跨文档访问")
    needles = (
        ("id=\"stage\"", "同文档精灵 DOM"),
        ("function playAction(", "动作播放引擎"),
        ("function resetAction(", "动作自动复位"),
        ("function playAll(", "自动演示"),
        ("function startWalk(", "走动位移"),
        ("void stage.offsetWidth", "一次性动画强制重播"),
        ("pointerdown", "点击角色接线"),
        ("pointermove", "视线跟随接线"),
        ("--gaze-x", "视线变量写入"),
    )
    problems += [label for needle, label in needles if needle not in html]
    return ("D07", "演示页同文档渲染且互动可重播", not problems,
            "、".join(problems) if problems else f"{len(needles)}/{len(needles)} 命中，无 iframe")


def check_preview_v10(root: Path) -> tuple[str, str, bool, str]:
    """预览页必须同步 v1.0 互动演示，否则新加的动作没有任何可视化验收入口。"""
    html = read(root, "docs/design/pet-v07-preview.html")
    needles = (
        ("v1.1", "标题版本"),
        ('data-action="cheer"', "请求完成庆祝按钮"),
        ('data-action="failed"', "失败失落按钮"),
        ('data-action="wake"', "睡醒过渡按钮"),
        ('data-action="drag"', "拖拽中按钮"),
        ('data-action="turn"', "转身张望按钮"),
        ("180×297", "v1.3.1 统一坐标系画布说明"), ("[120, 136", "新小档尺寸"),
        ("helloBtn", "开场招呼按钮"), ("roamBtn", "自主走动按钮"),
        ("pet-thought", "思考云 DOM"), ("roaming", "走动反馈类"),
        # v1.3：预览页必须真的能验收拖拽三段与全部 20 个动作。
        ("v1.3 动作注册表", "1000 帧注册表说明"),
        ("data-action=\"greet\"", "点头招呼按钮"),
        ("stage.dataset.dragPhase", "拖拽三段状态接线"),
        ("data-loop=\"lifted\"", "被提起动作层"),
        ("data-loop=\"landing\"", "落地动作层"),
        ("data-loop=\"lookAround\"", "转头张望动作层"),
    )
    missing = [label for needle, label in needles if needle not in html]
    return ("D08", "预览页含 v1.1 自动交互与尺寸演示", not missing,
            "、".join(missing) if missing else f"{len(needles)}/{len(needles)} 命中")


def check_animated_webp(root: Path) -> tuple[str, str, bool, str]:
    """10 个进包动画 WebP 必须同时有 ANIM/ANMF 帧、alpha 标志位与多帧差异。"""
    asset_dir = root / "src" / "assets" / "pet"
    _, _, loop_names = load_pet_manifest(root)
    rows = []
    for name in loop_names:
        path = asset_dir / f"{name}.webp"
        data = path.read_bytes()
        flags = 0
        frames = 0
        offset = 12
        while offset + 8 <= len(data):
            tag = data[offset:offset + 4]
            size = int.from_bytes(data[offset + 4:offset + 8], "little")
            if tag == b"VP8X" and size >= 1:
                flags = data[offset + 8]
            if tag == b"ANMF":
                frames += 1
            offset += 8 + size + (size & 1)
        ok = frames >= 2 and bool(flags & 0x10) and bool(flags & 0x02)
        rows.append(f"{name}:{frames}帧/{'alpha' if flags & 0x10 else '无alpha'}")
        if not ok:
            return "D06", "动画 WebP 带 alpha 且多帧", False, ", ".join(rows)
    return "D06", "动画 WebP 带 alpha 且多帧", True, ", ".join(rows)


def write_trace(path: Path, rows: list[tuple[str, str, bool, str]], passed: int) -> None:
    """输出结构化 Markdown 留痕，含逐项结果、结论与复核命令。"""
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    lines = [
        "# 桌面精灵链路契约检查留痕",
        "",
        f"- 执行时间：{now}",
        "- 检查范围：源码契约 + 构建产物 + 预览同源 + 动画 WebP RIFF 结构",
        f"- 结论：{'通过' if passed == len(rows) else '不通过'}（{passed}/{len(rows)}）",
        "",
        "| 编号 | 检查项 | 结果 | 证据 / 缺失 |",
        "| --- | --- | --- | --- |",
    ]
    for code, desc, ok, detail in rows:
        lines.append(f"| {code} | {desc} | {'通过' if ok else '不通过'} | {detail} |")
    lines += [
        "",
        "## 复核命令",
        "",
        "```bash",
        "python3 docs/孙测测/scripts/check_pet_overlay.py --root .",
        "npx tsc --noEmit && npm test && npm run build",
        "cd src-tauri && cargo test --lib",
        "```",
        "",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    """入口：源码 → 网关 → 产物 → 预览，汇总后写留痕并返回退出码。"""
    args = parse_args()
    root = args.root.resolve()
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    trace = (args.out or root / "docs" / "孙测测" / "traces" / f"桌面精灵链路契约检查-{stamp}.md").resolve()
    try:
        rows = check_sources(root)
        rows.append(check_gateway_wrappers(root))
        rows.append(check_runtime_wiring(root))
        rows.append(check_no_shake(root))
        rows.append(check_no_stage_backdrop(root))
        rows.append(check_pet_sizes(root))
        rows.append(check_pose_layers(root))
        rows.append(check_manifest_source(root))
        rows.append(check_action_registry(root))
        rows.append(check_v09_wiring(root))
        rows += check_v10_behaviors(root)
        rows += check_dist(root)
        rows.append(check_main_chunk_v10(root))
        rows.append(check_preview(root))
        rows.append(check_preview_interaction(root))
        rows.append(check_preview_v10(root))
        rows.append(check_animated_webp(root))
        write_trace(trace, rows, sum(1 for r in rows if r[2]))
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"[失败] 读取或解析异常: {exc}", file=sys.stderr)
        return 2

    failed = [r for r in rows if not r[2]]
    for code, desc, ok, detail in rows:
        print(f"[{'通过' if ok else '不通过'}] {code} {desc} — {detail}")
    print(f"汇总：{len(rows) - len(failed)}/{len(rows)} 通过，留痕 {trace}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
