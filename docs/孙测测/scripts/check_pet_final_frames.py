#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-020：桌面精灵 v1.3 落包 1000 帧逐帧验收。

编写人员：孙测测
作用用途：
  对最终进包的 41 张 WebP 逐帧判定，补齐 SCR-018 只验生产视频、不验落包
  产物的分层缺口：尺寸、帧数、alpha、四边留白、独立位图、跨动作人物
  尺度一致性、poster 与 loop 首帧形状一致、blink 与 sleep poster 字节
  不同、整包体积不超 8MB。
  v1.3.1 新增「四边留白」与「人物高度极差」两项：旧版只查脚部，导致
  stretch / peek / wake 伸展时手发被左右裁切、以及 20 动作高矮胖瘦
  （实测极差 27.9%）都能通过验收。
  v1.3.2 新增「hitBox 与 50 帧 alpha 并集一致」：素材换成纯缩放归一后人物
  变小，manifest 里按旧素材写死的命中框会与实物脱节（点击穿透偏位）；
  本项逐动作逐帧取并集归一化后与 manifest.poses[].hitBox 比对。
适用项目：SUGT 桌面精灵 v1.3.2（20 动作 / 1000 帧 / 180×297 / 56 色）
创建日期：2026-11-10
复用方式：
  /usr/local/bin/python3.13 docs/孙测测/scripts/check_pet_final_frames.py
  --root <项目根目录> --out <留痕路径>
  省略 --root 时按脚本位置推导，省略 --out 时按时间戳写入 traces/。
退出码约定：0 全部通过 / 1 存在判定不通过 / 2 读取、解析或依赖缺失
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("[失败] 需要 Pillow（本机 /usr/local/bin/python3.13）", file=sys.stderr)
    sys.exit(2)

MANIFEST_REL = Path("src") / "assets" / "pet" / "manifest.json"
ASSET_REL = Path("src") / "assets" / "pet"
TRACE_REL = Path("docs") / "孙测测" / "traces"

FOOT_MARGIN_RATIO = 0.02
# 四边安全边：窗口 overflow:hidden 硬裁剪 + lifted/hop/breathe 位移，
# 任一动作任一帧触碰画布边缘都视为「显示框外显示不全」。
SIDE_MARGIN_PX = 6
# 逐动作人物高度中位数允许的相对极差：>8% 即判定「高矮胖瘦」回归。
HEIGHT_SPREAD_RATIO = 0.08
ALPHA_THRESHOLD = 16
BUDGET_BYTES = 8 * 1024 * 1024
# hitBox 与实测并集的允许偏差（归一化坐标）：量化与阈值化会产生 1~2px 误差。
HITBOX_TOLERANCE = 0.012


def parse_args() -> argparse.Namespace:
    """解析参数；项目根由脚本位置推导，不硬编码本机路径。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(description="精灵落包 1000 帧逐帧验收")
    parser.add_argument("--root", type=Path, default=default_root,
                        help="项目根目录")
    parser.add_argument("--out", type=Path, default=None, help="自定留痕路径")
    return parser.parse_args()


def frame_alpha(image: Image.Image):
    """取当前帧的 RGBA 与阈值化 alpha，用于留白与形状判定。"""
    rgba = image.convert("RGBA")
    channel = rgba.getchannel("A")
    solid = channel.point(
        lambda value: 255 if value > ALPHA_THRESHOLD else 0)
    return rgba, channel, solid


def edge_margins(solid: Image.Image) -> tuple[int, int, int, int]:
    """返回角色到画布上/右/下/左四边的距离（像素）。"""
    box = solid.getbbox()
    if box is None:
        return (solid.height, solid.width, solid.height, solid.width)
    return (box[1], solid.width - box[2],
            solid.height - box[3], box[0])


def foot_margin(solid: Image.Image) -> int:
    """返回角色最下方实体像素到画布底边的距离（像素）。"""
    return edge_margins(solid)[2]


def digest(image: Image.Image) -> str:
    """对帧像素取 sha1，用于判定是否为独立位图。"""
    return hashlib.sha1(image.tobytes()).hexdigest()


def evaluate(root: Path) -> tuple[list[tuple[str, bool, str]], dict]:
    """逐动作逐帧验收，返回 (判定行, 统计)。"""
    manifest = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    asset_dir = root / ASSET_REL
    size = (manifest["baseSize"]["w"], manifest["baseSize"]["h"])
    required_margin = max(1, round(size[1] * FOOT_MARGIN_RATIO))
    rows: list[tuple[str, bool, str]] = []
    stats = {"actions": 0, "frames": 0, "min_margin": None,
             "total_bytes": 0, "required_margin": required_margin,
             "min_edge": None, "action_heights": {}, "height_spread": None}

    posters = {pose["name"]: pose["poster"] for pose in manifest["poses"]}
    for pose in manifest["poses"]:
        name = pose["name"]
        loop_path = asset_dir / pose["loop"]
        poster_path = asset_dir / pose["poster"]
        if not loop_path.is_file() or not poster_path.is_file():
            rows.append((f"{name} 素材落盘", False, "loop 或 poster 缺失"))
            continue
        with Image.open(loop_path) as loop, Image.open(poster_path) as poster:
            if loop.size != size or poster.size != size:
                rows.append((f"{name} 尺寸", False,
                             f"loop {loop.size} / poster {poster.size} ≠ {size}"))
                continue
            count = getattr(loop, "n_frames", 1)
            if count != pose["frames"]:
                rows.append((f"{name} 帧数", False,
                             f"{count} ≠ manifest {pose['frames']}"))
                continue
            margins = []
            edges = []
            heights = []
            bounds = []
            alpha_ok = True
            hashes = set()
            first_alpha = None
            for index in range(count):
                loop.seek(index)
                _rgba, _channel, solid = frame_alpha(loop)
                box = solid.getbbox()
                if box is None or solid.histogram()[255] == 0:
                    alpha_ok = False
                if solid.histogram()[0] == 0:
                    alpha_ok = False  # 没有透明像素，抠图失败
                edge = edge_margins(solid)
                margins.append(edge[2])
                edges.append(edge)
                heights.append(box[3] - box[1] if box else 0)
                if box:
                    bounds.append(box)
                hashes.add(digest(_rgba))
                if index == 0:
                    first_alpha = _channel.tobytes()
            min_margin = min(margins)
            min_edge = min(min(edge) for edge in edges)
            side_mins = tuple(min(edge[k] for edge in edges) for k in range(4))
            ordered = sorted(heights)
            median_height = ordered[len(ordered) // 2]
            stats["min_margin"] = (min(stats["min_margin"], min_margin)
                                   if stats["min_margin"] is not None
                                   else min_margin)
            stats["min_edge"] = (min(stats["min_edge"], min_edge)
                                 if stats["min_edge"] is not None
                                 else min_edge)
            stats["action_heights"][name] = median_height
            stats["frames"] += count
            stats["actions"] += 1
            stats["total_bytes"] += loop_path.stat().st_size
            stats["total_bytes"] += poster_path.stat().st_size
            rows.append((f"{name} alpha 透明/实体兼顾", alpha_ok,
                         f"{count} 帧逐帧检查"))
            rows.append((f"{name} 脚部留白 ≥{required_margin}px（2%）",
                         min_margin >= required_margin,
                         f"最小 {min_margin}px / 最大 {max(margins)}px"))
            rows.append((f"{name} 四边留白 ≥{SIDE_MARGIN_PX}px（防左右/顶部裁切）",
                         min_edge >= SIDE_MARGIN_PX,
                         f"最小 上/右/下/左 = {side_mins[0]}/{side_mins[1]}/"
                         f"{side_mins[2]}/{side_mins[3]} px"))
            rows.append((f"{name} 50 帧均为独立位图",
                         len(hashes) == count,
                         f"去重后 {len(hashes)}/{count} 帧"))
            _poster_rgba, _unused, _solid = frame_alpha(poster)
            rows.append((f"{name} poster 与 loop 首帧形状一致",
                         _poster_rgba.getchannel("A").tobytes() == first_alpha,
                         "alpha 通道逐像素比对"))
            if bounds:
                merged = (min(box[0] for box in bounds) / size[0],
                          min(box[1] for box in bounds) / size[1],
                          max(box[2] for box in bounds) / size[0],
                          max(box[3] for box in bounds) / size[1])
                declared = pose.get("hitBox")
                if isinstance(declared, list) and len(declared) == 4:
                    delta = max(abs(merged[i] - declared[i]) for i in range(4))
                    rows.append((f"{name} hitBox 与 50 帧 alpha 并集一致",
                                 delta <= HITBOX_TOLERANCE,
                                 "声明 %s / 实测 %s / 最大偏差 %.4f（阈值 %.3f）"
                                 % ([round(v, 3) for v in declared],
                                    [round(v, 3) for v in merged],
                                    delta, HITBOX_TOLERANCE)))
                else:
                    rows.append((f"{name} hitBox 声明合法", False,
                                 "manifest 缺 hitBox 或不是 4 元数组"))

    extras = manifest.get("extraFrames", [])
    for extra in extras:
        path = asset_dir / extra
        if not path.is_file():
            rows.append((f"附加帧 {extra} 落盘", False, "文件缺失"))
            continue
        with Image.open(path) as image:
            rows.append((f"附加帧 {extra} 尺寸", image.size == size,
                         f"{image.size}"))
        stats["total_bytes"] += path.stat().st_size

    sleep_poster = asset_dir / posters.get("sleep", "")
    blink = asset_dir / "su-blink.webp"
    if sleep_poster.is_file() and blink.is_file():
        rows.append(("blink 与 sleep poster 字节不同（防构建去重）",
                     sleep_poster.read_bytes() != blink.read_bytes(),
                     "sha256 %s / %s" % (
                         hashlib.sha256(sleep_poster.read_bytes())
                         .hexdigest()[:12],
                         hashlib.sha256(blink.read_bytes()).hexdigest()[:12])))

    heights = list(stats["action_heights"].values())
    if heights:
        low, high = min(heights), max(heights)
        spread = (high / low - 1) if low else float("inf")
        stats["height_spread"] = spread
        rows.append(("跨动作人物高度极差 ≤%.0f%%（消除高矮胖瘦）"
                     % (HEIGHT_SPREAD_RATIO * 100),
                     spread <= HEIGHT_SPREAD_RATIO,
                     "最小 %d px（%s）/ 最大 %d px（%s）/ 极差 %.1f%%" % (
                         low, min(stats["action_heights"],
                                  key=stats["action_heights"].get),
                         high, max(stats["action_heights"],
                                   key=stats["action_heights"].get),
                         spread * 100)))
    rows.append(("整包体积 ≤8MB",
                 stats["total_bytes"] <= BUDGET_BYTES,
                 "%d B / %.3f MB" % (
                     stats["total_bytes"], stats["total_bytes"] / 1024 ** 2)))
    return rows, stats


def write_trace(root: Path, out: Path | None, rows, stats) -> Path:
    """把逐项判定写成 Markdown 留痕，供上线回归复核。"""
    path = out or (root / TRACE_REL / (
        "精灵落包逐帧验收-%s.md" % datetime.now().strftime("%Y%m%d-%H%M%S")))
    path.parent.mkdir(parents=True, exist_ok=True)
    passed = sum(1 for _name, ok, _detail in rows if ok)
    lines = [
        "# 精灵 v1.3.2 落包 1000 帧逐帧验收留痕（SCR-020）", "",
        "- 执行时间：%s" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 脚本：SCR-020 `check_pet_final_frames.py`",
        "- 验收对象：`src/assets/pet` 最终进包素材（非 ComfyUI 原始视频）",
        "- 动作 %d 个 / 逐帧检查 %d 帧 / 最小脚部留白 %s px / 最小四边留白 %s px"
        % (stats["actions"], stats["frames"], stats["min_margin"],
           stats["min_edge"]),
        "- 跨动作人物高度极差：%s"
        % ("%.1f%%" % (stats["height_spread"] * 100)
           if stats["height_spread"] is not None else "无数据"),
        "- 整包 %d B（%.3f MB）/ 预算 %d B"
        % (stats["total_bytes"], stats["total_bytes"] / 1024 ** 2,
           BUDGET_BYTES),
        "", "## 判定明细", "",
        "| 判定项 | 结论 | 证据 |", "| --- | --- | --- |",
    ]
    for name, ok, detail in rows:
        lines.append("| %s | %s | %s |" % (
            name, "通过" if ok else "**不通过**", detail.replace("|", "\\|")))
    lines += [
        "", "## 结论", "",
        "- **%d/%d 通过**，退出码 %d。" % (
            passed, len(rows), 0 if passed == len(rows) else 1),
        "- 本脚本只验落包产物；生产视频级判据见 SCR-018，两层证据不可互相替代。",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main() -> int:
    """入口：解析 → 逐帧判定 → 留痕 → 退出码。"""
    args = parse_args()
    root = args.root.resolve()
    try:
        rows, stats = evaluate(root)
    except (OSError, ValueError, json.JSONDecodeError, KeyError) as exc:
        print("[失败] 读取、解析或依赖异常: %r" % exc, file=sys.stderr)
        return 2
    trace = write_trace(root, args.out, rows, stats)
    failed = [(name, detail) for name, ok, detail in rows if not ok]
    for name, ok, detail in rows:
        if not ok:
            print("[不通过] %s — %s" % (name, detail))
    print("汇总：%d/%d 通过，留痕 %s" % (
        len(rows) - len(failed), len(rows), trace))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
