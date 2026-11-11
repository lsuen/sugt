#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-024：桌面精灵人物尺度参数化量测 / 归一化传参推算 CLI。

编写人员：孙测测（孙文龙 sun）
作用用途：
  1) measure（默认，纯只读）：对进包 WebP 逐帧量出人物身高、头顶/脚底 y、
     头部长度（行宽剖面几何估计）、头身比、宽度与宽高比、四边留白、脚底线
     与水平中心偏差，输出逐动作 / 跨动作对比表，并可对照基线判红。
  2) plan（--plan）：按给定「目标身高 h_ref / 脚线 foot_y / 画布 / 安全边 /
     外扩 pad」推算每个动作的缩放因子、平移量、并集安全框与归一后四边留白，
     把「裁切、外扩、缩放、平移」各环节的输入传参与输出参数逐项列出来。
  本脚本不写、不改、不重编码任何进包素材，也不调用 ComfyUI；所有参数均可
  由命令行覆盖，缺省值取仓库内归一化基准（config/normalize-baseline.json）。
  头部长度为 alpha 行宽剖面的几何估计，不是语义分割结果，判定默认不启用。
适用项目：SUGT（小酥控制台）桌面精灵 v1.5（25 动作 / 1250 帧 / 240×297）
创建日期：2026-11-12
复用方式：
  /usr/local/bin/python3.13 docs/孙测测/scripts/pet_metrics.py --root <仓库根>
  /usr/local/bin/python3.13 docs/孙测测/scripts/pet_metrics.py --root <仓库根> \
      --actions yawn,idle --frames 5 --head-method ratio --out-dir /tmp/sugt-metrics
  /usr/local/bin/python3.13 docs/孙测测/scripts/pet_metrics.py --root <仓库根> \
      --compare <基线或另一份 metrics.json> --fail-on height-spread,foot-line
  /usr/local/bin/python3.13 docs/孙测测/scripts/pet_metrics.py --root <仓库根> \
      --plan --h-ref 180 --foot-y 240 --union-pad 16
退出码约定：0 全部通过（或 --fail-on 为空的纯量测）/ 1 判定不通过 / 2 读取、解析或依赖异常
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import statistics
import sys
from datetime import datetime
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("[失败] 需要 Pillow（本机 /usr/local/bin/python3.13）", file=sys.stderr)
    sys.exit(2)

MANIFEST_REL = Path("src") / "assets" / "pet" / "manifest.json"
DEFAULT_ASSETS_REL = Path("src") / "assets" / "pet"
BASELINE_REL = Path("docs") / "小精灵维护" / "config" / "normalize-baseline.json"
DEFAULT_TRACE_REL = Path("docs") / "孙测测" / "traces"

# 无归一化基准文件时的兜底默认：与 v1.4 定版锁定值完全一致，保证行为可复现。
FALLBACK = {
    "hRef": 176.04,
    "footY": 237,
    "canvas": [240, 297],
    "centerX": 120.0,
    "safeX": 11,
    "safeY": 18,
    "unionPad": 12,
    "alphaThreshold": 16,
    "minEdgePx": 6,
    "maxHiddenRatio": 0.02,
    "heightSpreadMax": 0.08,
}

# 默认判据：沿用既有 8% 跨动作极差（不放宽），其余均为实测已满足且不受动作
# 语义干扰的项。帧内波动 / 逐帧脚底 / 宽度比受鞠躬、离地、道具影响，降级为
# 「观察项」：照常量出并写入留痕，可用 --fail-on 显式开启专项判定。
DEFAULT_FAIL_ON = ("height-spread,foot-line,center,poster-height,"
                   "head-ratio,min-edge,assets-sha,compare-delta")
ALL_JUDGEMENTS = (
    "height-spread", "frame-height-spread", "foot-line", "center",
    "poster-height", "width-spread", "head-ratio", "min-edge", "assets-sha",
    "compare-delta",
)


def load_json(path: Path) -> dict:
    """读取 UTF-8 JSON；异常统一由 main 转成退出码 2。"""
    return json.loads(path.read_text(encoding="utf-8"))


def read_baseline(root: Path, explicit: str) -> dict:
    """归一化基准：优先命令行指定，其次仓库 config，最后兜底常量。"""
    if explicit:
        return load_json(Path(explicit).resolve())
    path = root / BASELINE_REL
    if path.is_file():
        return load_json(path)
    return dict(FALLBACK)


def parse_args() -> argparse.Namespace:
    """所有量测与变换参数都可由 CLI 覆盖；缺省值在解析后从基准回填。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(
        description="精灵人物尺度量测与归一化传参推算（只读，不改素材）",
        formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--assets", type=Path, default=None,
                        help="素材目录（默认 <root>/src/assets/pet；可指向临时副本做演练）")
    parser.add_argument("--actions", default="",
                        help="逗号分隔动作名，默认按 manifest 全部动作")
    parser.add_argument("--frames", type=int, default=0,
                        help="每动作最多检查多少帧，0 表示全部")
    parser.add_argument("--baseline", default="",
                        help="归一化基准 JSON 路径（默认读仓库 config）")
    parser.add_argument("--compare", default="",
                        help="对照 JSON：另一份 metrics 结果或含 metrics 段的基线")
    parser.add_argument("--fail-on", default=DEFAULT_FAIL_ON,
                        help="逗号分隔判据；传空串表示纯量测不判定。可选：%s"
                             % ",".join(ALL_JUDGEMENTS))
    parser.add_argument("--plan", action="store_true",
                        help="推算归一化传参（缩放 / 平移 / 安全框 / 外扩），不跑 measure")
    parser.add_argument("--frames-dir", type=Path, default=None,
                        help="plan 模式的源帧目录：<dir>/<action>/*.png；省略则用进包 loop")
    # 量测参数
    parser.add_argument("--alpha-threshold", type=int, default=None)
    parser.add_argument("--head-method", choices=("row-profile", "ratio", "off"),
                        default="row-profile", help="头部长度估计方式")
    parser.add_argument("--head-ratio", type=float, default=0.28,
                        help="ratio 法的头长占身高比例")
    parser.add_argument("--neck-width-ratio", type=float, default=0.62,
                        help="row-profile 法颈部相对头部最大行宽的收窄阈值")
    parser.add_argument("--head-search-min", type=float, default=0.14,
                        help="颈部搜索起点（占身高比例）")
    parser.add_argument("--head-search-max", type=float, default=0.42,
                        help="颈部搜索终点（占身高比例）")
    # 归一化传参（plan 与 measure 的判定阈值共用同一套）
    parser.add_argument("--h-ref", type=float, default=None, help="目标人物身高 px")
    parser.add_argument("--foot-y", type=int, default=None, help="脚底同线 y")
    parser.add_argument("--canvas", default="", help="画布，形如 240x297")
    parser.add_argument("--center-x", type=float, default=None, help="归一后水平中心")
    parser.add_argument("--safe-x", type=int, default=None, help="水平安全边")
    parser.add_argument("--safe-y", type=int, default=None, help="垂直安全边")
    parser.add_argument("--union-pad", type=int, default=None, help="并集框外扩像素")
    parser.add_argument("--min-edge", type=int, default=None, help="最小四边留白判据")
    parser.add_argument("--max-hidden", type=float, default=None,
                        help="并集框外实体占比上限")
    parser.add_argument("--height-spread-max", type=float, default=None,
                        help="跨动作身高极差上限（默认 0.08，不放宽）")
    # 输出
    parser.add_argument("--out-dir", type=Path, default=None,
                        help="留痕输出目录，默认 <root>/docs/孙测测/traces")
    parser.add_argument("--out-json", type=Path, default=None)
    parser.add_argument("--out-csv", type=Path, default=None)
    parser.add_argument("--out-md", type=Path, default=None)
    parser.add_argument("--no-trace", action="store_true",
                        help="不写 Markdown 留痕（演练时用，避免污染仓库最新留痕）")
    parser.add_argument("--quiet", action="store_true", help="只打印判定与结论")
    return parser.parse_args()


def resolve_params(args, base: dict, manifest: dict) -> dict:
    """把 CLI 覆盖值合到基准上，形成本次运行的唯一参数快照。"""
    limits = base.get("limits", {})
    size = manifest.get("baseSize", {})
    canvas = args.canvas or None
    if canvas:
        width, height = (int(item) for item in canvas.lower().split("x"))
        canvas_list = [width, height]
    else:
        canvas_list = list(base.get("canvas", [size.get("w"), size.get("h")]))

    def pick(cli_value, key, fallback=None):
        if cli_value is not None:
            return cli_value
        if key in base:
            return base[key]
        if key in limits:
            return limits[key]
        return fallback

    return {
        "hRef": pick(args.h_ref, "hRef", FALLBACK["hRef"]),
        "footY": pick(args.foot_y, "footY", FALLBACK["footY"]),
        "canvas": canvas_list,
        "centerX": pick(args.center_x, "centerX", canvas_list[0] / 2),
        "safeX": pick(args.safe_x, "safeX", FALLBACK["safeX"]),
        "safeY": pick(args.safe_y, "safeY", FALLBACK["safeY"]),
        "unionPad": pick(args.union_pad, "unionPad", FALLBACK["unionPad"]),
        "alphaThreshold": pick(args.alpha_threshold, "alphaThreshold",
                               FALLBACK["alphaThreshold"]),
        "minEdgePx": pick(args.min_edge, "minEdgePx", FALLBACK["minEdgePx"]),
        "maxHiddenRatio": pick(args.max_hidden, "maxHiddenRatio",
                               FALLBACK["maxHiddenRatio"]),
        "heightSpreadMax": pick(args.height_spread_max, "heightSpreadMax",
                                FALLBACK["heightSpreadMax"]),
        "frameHeightSpreadMax": limits.get("frameHeightSpreadMax", 0.35),
        "footLineTolerancePx": limits.get("footLineTolerancePx", 2),
        "centerTolerancePx": limits.get("centerTolerancePx", 12),
        "posterHeightTolerancePx": limits.get("posterHeightTolerancePx", 1),
        "widthRatioSpreadMax": limits.get("widthRatioSpreadMax", 1.00),
        "headRatioSpreadMax": limits.get("headRatioSpreadMax", 0.10),
        # 8% 跨动作极差对「单个动作被放大 5%」不敏感（极差仍 5.11% < 8%），
        # 故另设与基线逐动作对比的 3% 判据；既有 8% 不动、不放宽也不收紧。
        "compareDeltaMax": limits.get("compareDeltaMax", 0.03),
        "headMethod": args.head_method,
        "headRatio": args.head_ratio,
        "neckWidthRatio": args.neck_width_ratio,
        "headSearchMin": args.head_search_min,
        "headSearchMax": args.head_search_max,
        "frames": args.frames,
    }


def solid_alpha(image: Image.Image, threshold: int) -> Image.Image:
    """阈值化 alpha，得到 0/255 的实体掩膜。"""
    return image.convert("RGBA").getchannel("A").point(
        lambda value: 255 if value > threshold else 0)


def row_widths(solid: Image.Image) -> list[int]:
    """逐行实体像素数；用于按「颈部收窄」估计头部下沿。"""
    raw = solid.tobytes()
    width, height = solid.size
    return [raw[y * width:(y + 1) * width].count(255) for y in range(height)]


def head_extent(solid, box, params: dict) -> tuple[int, int, str]:
    """返回 (头顶 y, 头部下沿 y, 估计方式)。

    row-profile：在身高 14%~42% 区间找行宽显著收窄处作为颈线；找不到则回退
    到固定比例。这是几何估计，不是语义识别，故默认不纳入判红项。
    """
    if box is None or params["headMethod"] == "off":
        return (0, 0, "off")
    left, top, _right, bottom = box
    height = bottom - top
    if height <= 0:
        return (top, top, "empty")
    if params["headMethod"] == "ratio":
        return (top, round(top + params["headRatio"] * height), "ratio")
    widths = row_widths(solid.crop((left, top, box[2], bottom)))
    start = max(1, round(params["headSearchMin"] * height))
    end = min(height - 1, round(params["headSearchMax"] * height))
    if end <= start:
        return (top, round(top + params["headRatio"] * height), "ratio-fallback")
    head_peak = max(widths[:start + 1]) or 1
    best_y, best_width = None, None
    for y in range(start, end + 1):
        window = widths[max(0, y - 1):y + 2]
        value = sum(window) / len(window)
        if value <= params["neckWidthRatio"] * head_peak:
            if best_width is None or value < best_width:
                best_y, best_width = y, value
    if best_y is None:
        return (top, round(top + params["headRatio"] * height), "ratio-fallback")
    return (top, top + best_y, "row-profile")


def frame_metrics(solid: Image.Image, canvas: list[int], params: dict) -> dict:
    """单帧尺度指标：身高、头长、头身比、宽度、脚线、中心、四边留白。"""
    box = solid.getbbox()
    if box is None:
        return {"box": None, "height": 0, "width": 0, "headTop": 0,
                "headBottom": 0, "headLength": 0, "headRatio": 0.0,
                "footY": 0, "centerX": 0.0, "widthRatio": 0.0,
                "edges": [canvas[1], canvas[0], canvas[1], canvas[0]],
                "headMethod": "empty"}
    left, top, right, bottom = box
    height, width = bottom - top, right - left
    head_top, head_bottom, method = head_extent(solid, box, params)
    head_length = max(0, head_bottom - head_top)
    return {
        "box": [left, top, right, bottom],
        "height": height,
        "width": width,
        "headTop": head_top,
        "headBottom": head_bottom,
        "headLength": head_length,
        "headRatio": round(head_length / height, 4) if height else 0.0,
        "footY": bottom,
        "centerX": round((left + right) / 2, 2),
        "widthRatio": round(width / height, 4) if height else 0.0,
        "edges": [top, canvas[0] - right, canvas[1] - bottom, left],
        "headMethod": method,
    }


def upper_median(values: list[float]) -> float:
    """上中位数：与 SCR-020 / SCR-019 的 ordered[len//2] 口径完全一致。

    两套量测若一个用 statistics.median（偶数取平均）、一个用上中位数，50 帧时
    会差 0.5~1 px，跨动作极差看起来对不上；统一口径避免「文档数字不对齐」。
    """
    if not values:
        return 0.0
    return float(sorted(values)[len(values) // 2])


def percentile(values: list[float], ratio: float) -> float:
    """不依赖 numpy 的线性插值分位数。"""
    if not values:
        return 0.0
    ordered = sorted(values)
    if len(ordered) == 1:
        return float(ordered[0])
    index = (len(ordered) - 1) * ratio
    low = int(index)
    high = min(low + 1, len(ordered) - 1)
    return ordered[low] + (ordered[high] - ordered[low]) * (index - low)


def spread(values: list[float]) -> float:
    """相对极差；空或零基线返回 0。"""
    if not values:
        return 0.0
    low, high = min(values), max(values)
    return (high / low - 1) if low else 0.0


def assets_fingerprint(assets: Path) -> str:
    """按文件名排序，对全部 WebP 字节求指纹；素材一变留痕即失效。"""
    digest = hashlib.sha256()
    count = 0
    for path in sorted(assets.glob("*.webp")):
        digest.update(path.name.encode("utf-8"))
        digest.update(hashlib.sha256(path.read_bytes()).digest())
        count += 1
    return "%s:%d" % (digest.hexdigest(), count)


def measure(root: Path, args, base: dict, manifest: dict, params: dict) -> dict:
    """逐动作逐帧量测，返回结构化结果。"""
    assets = (args.assets or (root / DEFAULT_ASSETS_REL)).resolve()
    if not assets.is_dir():
        raise FileNotFoundError("素材目录不存在：%s" % assets)
    names = ([item.strip() for item in args.actions.split(",") if item.strip()]
             if args.actions else [pose["name"] for pose in manifest["poses"]])
    known = {pose["name"]: pose for pose in manifest["poses"]}
    unknown = [name for name in names if name not in known]
    if unknown:
        raise ValueError("--actions 含 manifest 未登记动作：%s" % unknown)

    actions = {}
    for name in names:
        pose = known[name]
        loop_path = assets / pose["loop"]
        poster_path = assets / pose["poster"]
        if not loop_path.is_file() or not poster_path.is_file():
            raise FileNotFoundError("%s 素材缺失：%s / %s" % (
                name, loop_path.name, poster_path.name))
        frames = []
        with Image.open(loop_path) as loop:
            count = getattr(loop, "n_frames", 1)
            limit = count if params["frames"] <= 0 else min(count, params["frames"])
            for index in range(limit):
                loop.seek(index)
                frames.append(frame_metrics(
                    solid_alpha(loop, params["alphaThreshold"]),
                    params["canvas"], params))
        with Image.open(poster_path) as poster:
            poster_metric = frame_metrics(
                solid_alpha(poster, params["alphaThreshold"]),
                params["canvas"], params)

        def column(key):
            return [item[key] for item in frames]

        heights = column("height")
        actions[name] = {
            "framesChecked": len(frames),
            "height": {
                "min": min(heights), "median": upper_median(heights),
                "max": max(heights), "p95": round(percentile(heights, 0.95), 2),
                "spread": round(spread(heights), 4),
            },
            "headLength": {
                "min": min(column("headLength")),
                "median": upper_median(column("headLength")),
                "max": max(column("headLength")),
                "method": frames[0]["headMethod"],
            },
            "headRatio": round(upper_median(column("headRatio")), 4),
            "width": {
                "min": min(column("width")), "median": upper_median(column("width")),
                "max": max(column("width")),
            },
            "widthRatio": round(upper_median(column("widthRatio")), 4),
            "footY": {
                "min": min(column("footY")), "median": upper_median(column("footY")),
                "max": max(column("footY")),
            },
            "centerX": {
                "min": min(column("centerX")), "median": upper_median(column("centerX")),
                "max": max(column("centerX")),
            },
            "minEdge": min(min(item["edges"]) for item in frames),
            "firstFrameHeight": frames[0]["height"],
            "posterHeight": poster_metric["height"],
            "posterHeadLength": poster_metric["headLength"],
            "frameHeights": heights,
        }

    medians = [item["height"]["median"] for item in actions.values()]
    frame_spreads = [item["height"]["spread"] for item in actions.values()]
    foot_lows = [item["footY"]["min"] for item in actions.values()]
    foot_highs = [item["footY"]["max"] for item in actions.values()]
    foot_medians = [item["footY"]["median"] for item in actions.values()]
    center_medians = [item["centerX"]["median"] for item in actions.values()]
    width_ratios = [item["widthRatio"] for item in actions.values()]
    head_ratios = [item["headRatio"] for item in actions.values()]
    summary = {
        "actions": len(actions),
        "framesChecked": sum(item["framesChecked"] for item in actions.values()),
        "heightMinPx": min(medians), "heightMaxPx": max(medians),
        "heightSpread": round(spread(medians), 4),
        "frameHeightSpreadMax": round(max(frame_spreads), 4),
        "frameHeightSpreadWorst": max(actions, key=lambda n: actions[n]["height"]["spread"]),
        "footLineMinPx": min(foot_lows), "footLineMaxPx": max(foot_highs),
        "footLineSpreadPx": max(foot_highs) - min(foot_lows),
        "footLineMedianSpreadPx": round(max(foot_medians) - min(foot_medians), 2),
        "centerSpreadPx": round(max(center_medians) - min(center_medians), 2),
        "widthRatioSpread": round(spread(width_ratios), 4),
        "headRatioSpread": round(spread(head_ratios), 4),
        "minEdgePx": min(item["minEdge"] for item in actions.values()),
        # poster 与 loop 首帧比，而不是与 50 帧中位比：后者会被鞠躬/落地这类
        # 帧内大幅动作污染（实测 greeting 差 33.5px，属语义不是缺陷）。
        "posterHeightDeltaMaxPx": max(
            abs(item["posterHeight"] - item["firstFrameHeight"])
            for item in actions.values()),
    }
    return {
        "schema": 1,
        "generatedAt": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "mode": "measure",
        "assets": str(assets),
        "assetsSha256": assets_fingerprint(assets),
        "params": params,
        "actions": actions,
        "summary": summary,
    }


def plan(root: Path, args, base: dict, manifest: dict, params: dict) -> dict:
    """推算归一化传参：并集安全框 → 缩放 → 平移 → 预计四边留白。"""
    assets = (args.assets or (root / DEFAULT_ASSETS_REL)).resolve()
    frame_root = args.frames_dir.resolve() if args.frames_dir else None
    names = ([item.strip() for item in args.actions.split(",") if item.strip()]
             if args.actions else [pose["name"] for pose in manifest["poses"]])
    canvas_w, canvas_h = params["canvas"]
    rows = {}
    for name in names:
        pose = next((p for p in manifest["poses"] if p["name"] == name), None)
        if pose is None:
            raise ValueError("--actions 含 manifest 未登记动作：%s" % name)
        if frame_root:
            paths = sorted((frame_root / name).glob("*.png"))
        else:
            paths = [assets / pose["loop"]]
        if not paths:
            raise FileNotFoundError("%s 无可用源帧：%s" % (name, frame_root))
        boxes = []
        for path in paths[:1] if not frame_root else paths:
            with Image.open(path) as image:
                count = getattr(image, "n_frames", 1) if not frame_root else 1
                limit = count if params["frames"] <= 0 else min(count, params["frames"])
                for index in range(limit):
                    if not frame_root:
                        image.seek(index)
                    solid = solid_alpha(image, params["alphaThreshold"])
                    box = solid.getbbox()
                    if box:
                        boxes.append(box)
        if not boxes:
            raise ValueError("%s 全部帧无有效 alpha bbox" % name)
        rep = (round(upper_median([box[0] for box in boxes])),
               round(upper_median([box[1] for box in boxes])),
               round(upper_median([box[2] for box in boxes])),
               round(upper_median([box[3] for box in boxes])))
        union = (min(box[0] for box in boxes), min(box[1] for box in boxes),
                 max(box[2] for box in boxes), max(box[3] for box in boxes))
        source_size = (canvas_w, canvas_h) if not frame_root else None
        pad = params["unionPad"]
        if source_size is None:
            with Image.open(paths[0]) as first:
                source_size = first.size
        safe_box = (max(0, union[0] - pad), max(0, union[1] - pad),
                    min(source_size[0], union[2] + pad),
                    min(source_size[1], union[3] + pad))
        rep_height = rep[3] - rep[1]
        factor = params["hRef"] / rep_height if rep_height else 0.0
        offset = (round(params["centerX"] - (rep[0] + rep[2]) / 2 * factor),
                  round(params["footY"] - rep[3] * factor))
        expected = (round(union[0] * factor + offset[0]),
                    round(union[1] * factor + offset[1]),
                    round(union[2] * factor + offset[0]),
                    round(union[3] * factor + offset[1]))
        rows[name] = {
            "sourceSize": list(source_size),
            "framesSampled": len(boxes),
            "repBox": list(rep), "repHeightPx": rep_height,
            "unionBox": list(union), "unionPadPx": pad,
            "safeBox": list(safe_box),
            "scale": round(factor, 6), "offset": list(offset),
            "expectedBox": list(expected),
            "expectedEdges": [expected[1], canvas_w - expected[2],
                              canvas_h - expected[3], expected[0]],
            "expectedMinEdge": min(expected[1], canvas_w - expected[2],
                                   canvas_h - expected[3], expected[0]),
            "cropBoxUsed": list(safe_box),
            "cropKeptRatio": round(
                (safe_box[2] - safe_box[0]) * (safe_box[3] - safe_box[1])
                / (source_size[0] * source_size[1]), 4),
            # safeX / safeY 也参与输出：预计四边留白是否容得下两层安全边。
            "expectedSafeOk": bool(
                expected[0] >= params["safeX"]
                and canvas_w - expected[2] >= params["safeX"]
                and expected[1] >= params["safeY"]
                and canvas_h - expected[3] >= params["safeY"]),
        }
    return {
        "schema": 1,
        "generatedAt": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "mode": "plan",
        "assets": str(assets),
        "framesDir": str(frame_root) if frame_root else "(进包 loop 反推)",
        "params": params,
        "actions": rows,
        "summary": {
            "actions": len(rows),
            "scaleMin": min(item["scale"] for item in rows.values()),
            "scaleMax": max(item["scale"] for item in rows.values()),
            "expectedMinEdge": min(item["expectedMinEdge"] for item in rows.values()),
        },
    }


def compare_with(result: dict, compare_path: str) -> dict:
    """与另一份 metrics 结果 / 基线里的 metrics 段逐动作对比身高。"""
    other = load_json(Path(compare_path).resolve())
    reference = other.get("metrics", other)
    actions = reference.get("actions", {})
    if not actions:
        raise ValueError("对照文件缺 actions 段：%s" % compare_path)
    diffs = []
    for name, current in result["actions"].items():
        if name not in actions:
            diffs.append({"action": name, "deltaPx": None, "note": "对照缺该动作"})
            continue
        past = actions[name]
        past_height = (past.get("height", {}).get("median")
                       if isinstance(past, dict) else None)
        if past_height is None:
            past_height = past.get("heightMedianPx") if isinstance(past, dict) else None
        if past_height is None:
            diffs.append({"action": name, "deltaPx": None, "note": "对照缺身高中位"})
            continue
        now = current["height"]["median"]
        diffs.append({"action": name, "baselinePx": past_height, "currentPx": now,
                      "deltaPx": round(now - past_height, 2),
                      "deltaRatio": round((now / past_height - 1) if past_height else 0, 4)})
    numeric = [item["deltaRatio"] for item in diffs if item.get("deltaRatio") is not None]
    return {"path": compare_path, "rows": diffs, "maxAbsRatio":
            round(max((abs(value) for value in numeric), default=0.0), 4)}


def judge(result: dict, fail_on: list[str], baseline_fingerprint: str,
          comparison: dict | None) -> list[dict]:
    """按参数快照里的限值判定；不放宽既有 8% 跨动作极差。

    plan 模式与 --fail-on 空串都必须直接返回空表：否则构造 checks 时会去读
    measure 专属的 summary 字段并抛 KeyError。
    """
    if not fail_on:
        return []
    summary = result["summary"]
    params = result["params"]
    checks = [
        ("height-spread", "跨动作人物身高极差 ≤%.0f%%" % (params["heightSpreadMax"] * 100),
         summary["heightSpread"] <= params["heightSpreadMax"],
         "%d~%d px，极差 %.2f%%" % (summary["heightMinPx"], summary["heightMaxPx"],
                                   summary["heightSpread"] * 100)),
        ("frame-height-spread", "帧内身高波动 ≤%.0f%%（观察项，默认不启用）"
         % (params["frameHeightSpreadMax"] * 100),
         summary["frameHeightSpreadMax"] <= params["frameHeightSpreadMax"],
         "最差 %.2f%%（%s）" % (summary["frameHeightSpreadMax"] * 100,
                              summary["frameHeightSpreadWorst"])),
        ("foot-line", "各动作脚底中位同线 ≤%d px" % params["footLineTolerancePx"],
         summary["footLineMedianSpreadPx"] <= params["footLineTolerancePx"],
         "脚底中位极差 %.2f px；逐帧全局 %d~%d px（离地动作，观察项）"
         % (summary["footLineMedianSpreadPx"], summary["footLineMinPx"],
            summary["footLineMaxPx"])),
        ("center", "各动作水平中心中位偏差 ≤%d px" % params["centerTolerancePx"],
         summary["centerSpreadPx"] <= params["centerTolerancePx"],
         "%.2f px（主要来自 stretch 双臂展开的 bbox 不对称）"
         % summary["centerSpreadPx"]),
        ("poster-height", "poster 与 loop 首帧身高差 ≤%d px"
         % params["posterHeightTolerancePx"],
         summary["posterHeightDeltaMaxPx"] <= params["posterHeightTolerancePx"],
         "最大差 %d px" % summary["posterHeightDeltaMaxPx"]),
        ("width-spread", "宽高比极差 ≤%.0f%%（观察项，默认不启用）"
         % (params["widthRatioSpreadMax"] * 100),
         summary["widthRatioSpread"] <= params["widthRatioSpreadMax"],
         "%.2f%%（道具 / 大幅动作）" % (summary["widthRatioSpread"] * 100)),
        ("head-ratio", "跨动作头身比极差 ≤%.0f%%（几何估计）"
         % (params["headRatioSpreadMax"] * 100),
         summary["headRatioSpread"] <= params["headRatioSpreadMax"],
         "%.2f%%" % (summary["headRatioSpread"] * 100)),
        ("min-edge", "任一帧最小四边留白 ≥%d px" % params["minEdgePx"],
         summary["minEdgePx"] >= params["minEdgePx"],
         "实测 %d px" % summary["minEdgePx"]),
        ("assets-sha", "素材指纹与基线一致",
         (result["assetsSha256"] == baseline_fingerprint
          if baseline_fingerprint else True),
         ("一致" if result["assetsSha256"] == baseline_fingerprint else "已变更，需重跑基线")
         if baseline_fingerprint else "基线未登记指纹，跳过比对"),
        # 8% 跨动作极差会漏掉单动作 5% 漂移；对照基线逐动作看差值才抓得住。
        # 未传 --compare 时记「跳过」，不算通过也不算失败。
        ("compare-delta", "逐动作身高与基线偏差 ≤%.0f%%"
         % (params["compareDeltaMax"] * 100),
         None if comparison is None
         else comparison["maxAbsRatio"] <= params["compareDeltaMax"],
         "未提供 --compare，跳过比对" if comparison is None else
         "最大偏差 %.2f%%（对照 %s）" % (comparison["maxAbsRatio"] * 100,
                                    Path(comparison["path"]).name)),
    ]
    return [{"id": item[0], "name": item[1],
             "ok": None if item[2] is None else bool(item[2]),
             "detail": item[3]}
            for item in checks if item[0] in fail_on]


def write_outputs(root: Path, args, result: dict, judgements: list[dict],
                  comparison: dict | None) -> dict:
    """写 JSON / CSV / Markdown 留痕；路径可由 CLI 指定，演练可指向 /tmp。"""
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = (args.out_dir or (root / DEFAULT_TRACE_REL)).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    stem = "精灵身高量测-%s" % stamp if result["mode"] == "measure" else "精灵归一化传参推算-%s" % stamp
    json_path = args.out_json or (out_dir / (stem + ".json"))
    csv_path = args.out_csv or (out_dir / (stem + ".csv"))
    md_path = args.out_md or (out_dir / (stem + ".md"))
    json_path.parent.mkdir(parents=True, exist_ok=True)
    payload = dict(result)
    payload["judgements"] = judgements
    payload["comparison"] = comparison
    payload["conclusion"] = (
        "通过" if all(item["ok"] is not False for item in judgements)
        else "不通过") if judgements else "仅量测"
    json_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
                         encoding="utf-8")

    rows = []
    for name, item in result["actions"].items():
        if result["mode"] == "measure":
            rows.append([
                name, item["framesChecked"], item["height"]["min"],
                item["height"]["median"], item["height"]["max"],
                round(item["height"]["spread"], 4), item["headLength"]["median"],
                item["headRatio"], item["width"]["median"], item["widthRatio"],
                item["footY"]["min"], item["footY"]["max"],
                item["centerX"]["median"], item["minEdge"], item["posterHeight"],
            ])
        else:
            rows.append([name, item["framesSampled"], item["repHeightPx"],
                         item["scale"], item["offset"][0], item["offset"][1],
                         item["unionPadPx"], item["safeBox"], item["expectedBox"],
                         item["expectedMinEdge"], item["cropKeptRatio"]])
    header = (["动作", "帧数", "身高min", "身高中位", "身高max", "帧内极差",
               "头长中位", "头身比", "宽度中位", "宽高比", "脚底min", "脚底max",
               "中心x中位", "最小四边留白", "poster身高"]
              if result["mode"] == "measure" else
              ["动作", "采样帧", "代表框高", "scale", "offsetX", "offsetY",
               "外扩pad", "安全框", "预计归一框", "预计最小留白", "保留面积比"])
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", encoding="utf-8-sig", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(header)
        writer.writerows(rows)

    md = ["# 精灵 %s 留痕（SCR-024）" % ("人物尺度量测" if result["mode"] == "measure"
                                    else "归一化传参推算"), "",
          "- 执行时间：%s" % result["generatedAt"],
          "- 素材目录：`%s`" % result["assets"],
          "- 素材指纹：`%s`" % result.get("assetsSha256", "(plan 模式不计算)"),
          "- 结论：**%s**" % payload["conclusion"], "",
          "## 本次运行参数（全部可由 CLI 覆盖）", "",
          "```json", json.dumps(result["params"], ensure_ascii=False, indent=2), "```", ""]
    if result["mode"] == "measure":
        summary = result["summary"]
        md += ["## 跨动作汇总", "",
               "- 动作 %d 个 / 检查 %d 帧" % (summary["actions"], summary["framesChecked"]),
               "- 身高中位 %d~%d px，极差 %.2f%%（阈值 %.0f%%）" % (
                   summary["heightMinPx"], summary["heightMaxPx"],
                   summary["heightSpread"] * 100,
                   result["params"]["heightSpreadMax"] * 100),
               "- 帧内身高波动最差 %.2f%%（%s）" % (
                   summary["frameHeightSpreadMax"] * 100,
                   summary["frameHeightSpreadWorst"]),
               "- 脚底中位同线极差 %.2f px（判据）/ 逐帧全局 %d~%d px（离地动作，观察项）"
               " / 中心 x 中位极差 %.2f px" % (
                   summary["footLineMedianSpreadPx"], summary["footLineMinPx"],
                   summary["footLineMaxPx"], summary["centerSpreadPx"]),
               "- 宽高比极差 %.2f%% / 头身比极差 %.2f%%（几何估计）/ "
               "最小四边留白 %d px" % (
                   summary["widthRatioSpread"] * 100,
                   summary["headRatioSpread"] * 100, summary["minEdgePx"]), "",
               "## 逐动作明细", "",
               "| 动作 | 帧 | 身高 min/中位/max px | 帧内极差 | 头长中位 px | 头身比 | "
               "宽度中位 px | 宽高比 | 脚底 min/max | 中心 x | 四边留白 | poster 身高 |",
               "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
        for name, item in result["actions"].items():
            height = item["height"]
            md.append("| %s | %d | %d / %d / %d | %.2f%% | %d | %.3f | %d | %.3f | "
                      "%d / %d | %.1f | %d | %d |" % (
                          name, item["framesChecked"], height["min"],
                          height["median"], height["max"], height["spread"] * 100,
                          item["headLength"]["median"], item["headRatio"],
                          item["width"]["median"], item["widthRatio"],
                          item["footY"]["min"], item["footY"]["max"],
                          item["centerX"]["median"], item["minEdge"],
                          item["posterHeight"]))
    else:
        md += ["## 归一化传参推算（输入传参 → 输出变换）", "",
               "- 源帧：`%s`" % result["framesDir"], "",
               "| 动作 | 采样帧 | 代表框高 px | 并集框 | 外扩后安全框 | scale | "
               "平移 (x,y) | 预计归一框 | 预计最小留白 px | 保留面积比 |",
               "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
        for name, item in result["actions"].items():
            md.append("| %s | %d | %d | %s | %s | %.6f | %s | %s | %d | %.3f |" % (
                name, item["framesSampled"], item["repHeightPx"], item["unionBox"],
                item["safeBox"], item["scale"], item["offset"], item["expectedBox"],
                item["expectedMinEdge"], item["cropKeptRatio"]))
    if comparison:
        md += ["", "## 与对照基线的身高差异", "",
               "| 动作 | 对照中位 px | 本次中位 px | 差 px | 差 % |",
               "| --- | --- | --- | --- | --- |"]
        for row in comparison["rows"]:
            md.append("| %s | %s | %s | %s | %s |" % (
                row["action"], row.get("baselinePx", "-"), row.get("currentPx", "-"),
                row.get("deltaPx", "-"),
                round(row["deltaRatio"] * 100, 2) if row.get("deltaRatio") is not None else "-"))
    if judgements:
        passed = sum(1 for item in judgements if item["ok"] is True)
        md += ["", "## 判定（%d/%d 通过）" % (passed, len(judgements)), "",
               "| 判据 | 结论 | 证据 |", "| --- | --- | --- |"]
        for item in judgements:
            status = ("跳过" if item["ok"] is None
                      else "通过" if item["ok"] else "**不通过**")
            md.append("| %s | %s | %s |" % (item["name"], status, item["detail"]))
    md += ["", "## 机器可读摘要（供技能基线自检读取）", "", "```json",
           json.dumps({"schema": 1, "mode": result["mode"],
                       "assetsSha256": result.get("assetsSha256", ""),
                       "summary": result["summary"],
                       "conclusion": payload["conclusion"]},
                      ensure_ascii=False, indent=2),
           "```", "",
           "> 头部长度为 alpha 行宽剖面的几何估计，不是语义分割；默认不纳入判红。"
           "本脚本只读素材，不写进包目录、不调用 ComfyUI。", ""]
    md_path.parent.mkdir(parents=True, exist_ok=True)
    if not args.no_trace:
        md_path.write_text("\n".join(md), encoding="utf-8")
    return {"json": str(json_path), "csv": str(csv_path),
            "md": "" if args.no_trace else str(md_path)}


def main() -> int:
    """入口：解析 → 量测/推算 → 对比 → 判定 → 留痕 → 退出码。"""
    args = parse_args()
    root = args.root.resolve()
    try:
        manifest = load_json(root / MANIFEST_REL)
        base = read_baseline(root, args.baseline)
        params = resolve_params(args, base, manifest)
        result = (plan(root, args, base, manifest, params) if args.plan
                  else measure(root, args, base, manifest, params))
        fail_on = [item.strip() for item in args.fail_on.split(",") if item.strip()]
        bad = [item for item in fail_on if item not in ALL_JUDGEMENTS]
        if bad:
            raise ValueError("--fail-on 含未知判据：%s" % bad)
        if args.plan:
            fail_on = []
        comparison = (compare_with(result, args.compare)
                      if args.compare and result["mode"] == "measure" else None)
        fingerprint = ""
        limits = base.get("limits", {})
        if "assetsSha256" in limits:
            fingerprint = limits["assetsSha256"]
        elif "metrics" in base and isinstance(base["metrics"], dict):
            fingerprint = base["metrics"].get("assetsSha256", "")
        judgements = judge(result, fail_on, fingerprint, comparison)
    except (OSError, ValueError, KeyError, json.JSONDecodeError,
            statistics.StatisticsError) as exc:
        print("[失败] 读取、解析或依赖异常：%r" % exc, file=sys.stderr)
        return 2

    paths = write_outputs(root, args, result, judgements, comparison)
    if args.plan:
        summary = result["summary"]
        print("传参推算：%d 动作 / scale %.6f~%.6f / 预计最小留白 %d px" % (
            summary["actions"], summary["scaleMin"], summary["scaleMax"],
            summary["expectedMinEdge"]))
    else:
        if not args.quiet:
            summary = result["summary"]
            print("| 动作 | 身高中位 px | 帧内极差 | 头长 px | 头身比 | 宽度 px | 脚底 px | 中心 x |")
            print("| --- | --- | --- | --- | --- | --- | --- | --- |")
            for name, item in result["actions"].items():
                print("| %s | %d | %.2f%% | %d | %.3f | %d | %d~%d | %.1f |" % (
                    name, item["height"]["median"], item["height"]["spread"] * 100,
                    item["headLength"]["median"], item["headRatio"],
                    item["width"]["median"], item["footY"]["min"],
                    item["footY"]["max"], item["centerX"]["median"]))
            print("跨动作：%d~%d px，极差 %.2f%%；帧内最差 %.2f%%（%s）；"
                  "脚底极差 %d px；中心极差 %.2f px" % (
                      summary["heightMinPx"], summary["heightMaxPx"],
                      summary["heightSpread"] * 100,
                      summary["frameHeightSpreadMax"] * 100,
                      summary["frameHeightSpreadWorst"],
                      summary["footLineSpreadPx"], summary["centerSpreadPx"]))
        if comparison:
            print("对照 %s：最大身高偏差 %.2f%%" % (
                comparison["path"], comparison["maxAbsRatio"] * 100))
        for item in judgements:
            if item["ok"] is False:
                print("[不通过] %s — %s" % (item["name"], item["detail"]))
            elif item["ok"] is None:
                print("[跳过] %s — %s" % (item["name"], item["detail"]))
        if judgements:
            passed = sum(1 for item in judgements if item["ok"] is True)
            failed = sum(1 for item in judgements if item["ok"] is False)
            skipped = sum(1 for item in judgements if item["ok"] is None)
            print("结论：%d/%d 通过%s%s" % (
                passed, len(judgements),
                "，判定不通过" if failed else "",
                "（%d 项跳过）" % skipped if skipped else ""))
        else:
            print("结论：仅量测，未启用判定（--fail-on 为空或 plan 模式）")
    print("留痕：JSON %s / CSV %s%s" % (
        paths["json"], paths["csv"],
        (" / MD %s" % paths["md"]) if paths["md"] else " / MD 已跳过"))
    return 1 if any(item["ok"] is False for item in judgements) else 0


if __name__ == "__main__":
    sys.exit(main())
