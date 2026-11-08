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
SOURCE_CHECKS = [
    ("C01", "精灵 CSS v0.7 分层选择器齐全", "src/overlay.css",
     [".pet-side", ".pet-figure", ".pet-motion", ".pet-loops", ".pet-loop", ".pet-poster",
      ".pet-blink", ".pet-gaze", ".pet-ambient", ".pet-status-bar", ".pet-status-dot",
      ".pet-bubble", ".pet-edit-bar"]),
    ("C02", "编辑态显式恢复 pointer-events", "src/overlay.css",
     [".pet-stage.interactive .pet-edit-bar { pointer-events: auto; }"]),
    ("C03", "呼吸与状态一次性动效分层", "src/overlay.css",
     ["animation: pet-breathe 3.8s", "[data-state=\"done\"] .pet-loops",
      "[data-state=\"error\"] .pet-loops", "pet-hop 0.62s ease-out 1"]),
    ("C03b", "已移除思考态无限高频摇晃", "src/overlay.css", ["@keyframes pet-breathe"]),
    ("C03c", "带 alpha 动画 WebP 五个状态全覆盖", "src/overlay.css",
     ["su-idle-loop.webp", "su-typing-loop.webp", "su-done-loop.webp",
      "su-error-loop.webp", "su-sleep-loop.webp"]),
    ("C03d", "静态帧降级与眼睑层存在", "src/overlay.css",
     [".pet-stage.still .pet-loop { display: none; }", "pet-stage.blinking .pet-blink"]),
    ("C04", "拖动状态机按本次按下判定", "src/overlay.tsx",
     ["dragActive.current", "movedDuringDrag.current", "pendingDrag.current", "dragRequested.current"]),
    ("C05", "成功气泡等待在途归零", "src/overlay.tsx",
     ["activeNow.current > 0", "wasActive.current"]),
    ("C06", "网关两个入口把守卫挂到响应体", "src-tauri/src/gateway.rs",
     ["fn hold_guard_through_body", "hold_guard_through_body(response, active_guard)"]),
    ("C07", "网关在途守卫单测", "src-tauri/src/gateway/tests.rs",
     ["active_guard_lives_until_streaming_body_ends", "active_guard_releases_when_body_is_dropped"]),
    ("C08", "吸附按中心选屏 / 回位按完整窗口", "src-tauri/src/overlay.rs",
     ["let center_x", "fn window_fully_inside", "monitor_lookup_uses_window_center", "window_visibility_requires_all_corners_inside"]),
    ("C09", "设置项本地草稿与档位记忆", "src/SettingsPanel.tsx",
     ["overlayDraft", "pendingWrites", "PET_SIZE_STORAGE_KEY"]),
    ("C10", "三档尺寸与后端校验范围相容", "src/SettingsPanel.tsx",
     ["width: 200, height: 160", "width: 300, height: 240", "width: 400, height: 320"]),
    ("C11", "吸附文案 zh/en 同步为阈值口径", "src/i18n.ts",
     ["靠近屏幕边缘", "near a screen edge"]),
    ("C11b", "轻碰回应 zh/en 同步", "src/i18n.ts",
     ["'pet.bubble.poke': '嗯？我在呢～'", "'pet.bubble.poke': 'Hmm? Right here~'"]),
    ("C11c", "互动说明覆盖转头/眨眼/轻碰", "src/i18n.ts",
     ["会转头看鼠标、随机眨眼", "turns toward the cursor, blinks"]),
    ("C15", "运行时视线/眨眼/轻碰/静态帧链路", "src/overlay.tsx",
     ["--gaze-x", "setBlinking", "setPoked", "setStill", "PET_LOOPS", "--drag-tilt"]),
    ("C12", "悬浮窗 ACL 含全部精灵命令", "src-tauri/permissions/app-commands.toml",
     ["get_overlay_config", "set_overlay_config", "overlay_start_drag", "focus_main_window", "pet_finish_drag"]),
    ("C13", "悬浮窗能力含 core:default", "src-tauri/capabilities/overlay.json", ["core:default"]),
]

DIST_CSS_SELECTORS = [".pet-side", ".pet-figure", ".pet-motion", ".pet-loops", 
                     ".pet-blink", ".pet-status-bar", ".pet-edit-bar"]
PET_ASSET_NAMES = ("su-idle", "su-typing", "su-done", "su-error", "su-sleep", "su-blink",
                   "su-idle-loop", "su-typing-loop", "su-done-loop", "su-error-loop", "su-sleep-loop")
LOOP_ASSET_NAMES = ("su-idle-loop", "su-typing-loop", "su-done-loop",
                    "su-error-loop", "su-sleep-loop")


def parse_args() -> argparse.Namespace:
    """解析参数，默认项目根由脚本位置推导。"""
    default_root = Path(__file__).resolve().parents[2]
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
    css = read(root, "src/overlay.css")
    bad = [needle for needle in (
        "pet-typing 0.42s ease-in-out infinite",
        "@keyframes pet-typing",
        "pet-error 0.4s ease-in-out 2",
    ) if needle in css]
    return "C16", "无限高频摇晃已移除", not bad, "、".join(bad) if bad else "无残留"


def check_dist(root: Path) -> list[tuple[str, str, bool, str]]:
    """检查生产构建产物：CSS 类名、JS 命令名、5 张素材。"""
    results = []
    css_files = glob.glob(str(root / "dist" / "assets" / "overlay-*.css"))
    if not css_files:
        results.append(("D01", "dist 精灵 CSS 存在", False, "未找到 dist/assets/overlay-*.css"))
    else:
        css = Path(css_files[-1]).read_text(encoding="utf-8")
        missing = [s for s in DIST_CSS_SELECTORS if s not in css]
        results.append(("D01", "dist 精灵 CSS 含 v0.7 布局类名", not missing,
                        "、".join(missing) if missing else f"全部命中（{len(css)} B）"))
        results.append(("D02", "dist CSS 含编辑态命中修复", "pointer-events:auto" in css.replace(" ", ""),
                        "压缩后仍保留 pointer-events:auto"))

    js_files = glob.glob(str(root / "dist" / "assets" / "overlay-*.js"))
    if not js_files:
        results.append(("D03", "dist 精灵 JS 存在", False, "未找到 dist/assets/overlay-*.js"))
    else:
        js = Path(js_files[-1]).read_text(encoding="utf-8")
        missing = [s for s in ("pet_finish_drag", "focus_main_window", "active_requests") if s not in js]
        results.append(("D03", "dist JS 含拖动收尾/聚焦/在途字段", not missing,
                        "、".join(missing) if missing else "全部命中"))

    assets = sorted((root / "dist" / "assets").glob("su-*.webp"))
    total = sum(p.stat().st_size for p in assets)
    names_ok = all(any(p.name.startswith(name + "-") for p in assets) for name in PET_ASSET_NAMES)
    results.append(("D04", "dist 内含 11 张精灵 WebP（6 静态 + 5 动画）",
                    len(assets) == 11 and names_ok, f"{len(assets)} 张 / {total / 1024:.2f} KB"))
    return results


def check_preview(root: Path) -> tuple[str, str, bool, str]:
    """预览页必须由同源 CSS 生成：页内记录的 hash 与当前源 CSS 一致。"""
    import hashlib

    css = read(root, "src/overlay.css")
    expected = hashlib.sha256(css.encode("utf-8")).hexdigest()[:12]
    preview = read(root, "docs/design/pet-v07-preview.html")
    found = re.search(r"源 CSS sha256 ([0-9a-f]{12})", preview)
    actual = found.group(1) if found else "未找到"
    return "D05", "预览页与源 CSS 同源", actual == expected, f"源 {expected} / 预览页 {actual}"


def check_preview_frame_script(root: Path) -> tuple[str, str, bool, str]:
    """预览页 iframe 内脚本必须闭合且复刻视线映射。

    结束标签写成两个反斜杠时，JS 求值后 srcdoc 里是非法标签，iframe 内
    script 不闭合、整段不执行，预览页「鼠标移到角色身上会转头」静默失效。
    """
    html = read(root, "docs/design/pet-v07-preview.html")
    problems = []
    if "<\\\\/script>" in html:
        problems.append("结束标签双重转义")
    if "<\\/script>" not in html:
        problems.append("缺少单反斜杠转义的结束标签")
    if 'document.addEventListener("pointermove"' not in html:
        problems.append("缺少 iframe 内视线映射")
    return ("D07", "预览页 iframe 内视线脚本可闭合执行", not problems,
            "、".join(problems) if problems else "闭合标签与视线映射均命中")


def check_animated_webp(root: Path) -> tuple[str, str, bool, str]:
    """5 个进包动画 WebP 必须同时有 ANIM/ANMF 帧、alpha 标志位与多帧差异。"""
    asset_dir = root / "src" / "assets" / "pet"
    rows = []
    for name in LOOP_ASSET_NAMES:
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
        rows.append(check_no_shake(root))
        rows += check_dist(root)
        rows.append(check_preview(root))
        rows.append(check_preview_frame_script(root))
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
