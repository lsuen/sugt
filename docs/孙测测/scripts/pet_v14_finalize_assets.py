#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：v1.4 新动作落包桥接。把 ComfyUI 候选视频（640×768 / 56 帧）按
         v1.3.3 锁定的 h_ref=176.04 / foot_y=237 归一化为 240×297 / 50 帧透明
         源帧，再复用既有编码链落包。解决「单动作跑 do_frames 会重算 h_ref、
         导致新动作与老 20 动作人物不同高」的阻塞。
适用项目：SUGT（桌面精灵 小酥）
创建日期：2026-11-11
复用方式：/usr/local/bin/python3.13 docs/孙测测/scripts/pet_v14_finalize_assets.py
           --phase assess|build|apply --root <仓库根> [阶段专属参数]
           --h-ref 180 --foot-y 240 --union-pad 16 ...（归一化传参均可 CLI 覆盖）
退出码约定：0 全部通过 / 1 自检不通过 / 2 读取、解析或子命令异常

v1.5 参数化：归一化常量不再只能改源码。优先读仓库唯一真相源
`docs/小精灵维护/config/normalize-baseline.json`，再由 CLI 逐项覆盖；不传任何
覆盖参数时取值与 v1.4 定版完全一致。`--no-baseline` 可忽略基准文件，用脚本
内置常量复跑 v1.4 定版行为，用来证明参数化没有改变默认结果。

三个阶段：
  assess  只读：解析批次日志与 state.json，给每个候选版本打分，产出 picks.json
          与 Markdown 留痕；由人工目视确认后再进入 build。
  build   离线：按 picks 抠图 → 纯缩放+平移归一化到 240×297 → 输出 10 张源帧；
          同时与老 20 动作补边素材做「人物同高」交叉量测并留痕。
  apply   落包：临时 root + 5 动作清单副本调用 pet_reencode_assets.py，
          把 10 张产物复制回 src/assets/pet/，凑齐 49 张。
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path

from PIL import Image, ImageChops

# ---- v1.4 归一化契约常量（不可随单动作重算，否则人物高矮不一）----
CANVAS_W, CANVAS_H = 240, 297
CENTER_X = CANVAS_W / 2          # 120；老 20 动作在 180 画布中心 90，补边 +30 后同为 120
H_REF = 176.04                   # /tmp/sugt-pet-actions-v13/state.json normalise.h_ref
FOOT_Y = 237                     # 同上 normalise.foot_y；脚底同线
SAFE_X, SAFE_Y = 11, 18          # 同上 normalise.safe；老画布 180×297 的安全边
WANTED_FRAMES = 50
ALPHA_THRESHOLD = 16
UNION_PAD = 12
MIN_EDGE_PX = 6
MAX_HEIGHT_SPREAD = 0.08
MAX_HIDDEN_RATIO = 0.02

BASELINE_REL = Path("docs") / "小精灵维护" / "config" / "normalize-baseline.json"

# 由 CLI 覆盖的模块级参数名 → 基准 JSON 键（缺省用上方定版常量）。
PARAM_KEYS = (
    ("CANVAS_W", None), ("CANVAS_H", None), ("CENTER_X", "centerX"),
    ("H_REF", "hRef"), ("FOOT_Y", "footY"),
    ("SAFE_X", "safeX"), ("SAFE_Y", "safeY"),
    ("WANTED_FRAMES", None), ("ALPHA_THRESHOLD", "alphaThreshold"),
    ("UNION_PAD", "unionPad"), ("MIN_EDGE_PX", "minEdgePx"),
    ("MAX_HEIGHT_SPREAD", "heightSpreadMax"),
    ("MAX_HIDDEN_RATIO", "maxHiddenRatio"),
)


def load_normalize_params(root: Path, args) -> dict:
    """归一化传参：仓库基准 → CLI 覆盖 → 返回本次参数快照。

    返回 dict 的键与模块级常量同名，由 main 在入口一次性注入；build / apply
    的定版算法一行不改，保证「不传覆盖参数 = v1.4 定版行为」可复现。
    """
    values = {name: globals()[name] for name, _ in PARAM_KEYS}
    baseline_path = Path(args.normalize_baseline)
    if not baseline_path.is_absolute():
        baseline_path = root / baseline_path
    if args.no_baseline:
        source = "脚本内置 v1.4 定版常量（--no-baseline）"
    elif baseline_path.is_file():
        data = json.loads(baseline_path.read_text(encoding="utf-8"))
        limits = data.get("limits", {})
        canvas = data.get("canvas", [CANVAS_W, CANVAS_H])
        values.update({
            "CANVAS_W": canvas[0], "CANVAS_H": canvas[1],
            "CENTER_X": data.get("centerX", CENTER_X),
            "H_REF": data.get("hRef", H_REF),
            "FOOT_Y": data.get("footY", FOOT_Y),
            "SAFE_X": data.get("safeX", SAFE_X),
            "SAFE_Y": data.get("safeY", SAFE_Y),
            "ALPHA_THRESHOLD": data.get("alphaThreshold", ALPHA_THRESHOLD),
            "UNION_PAD": data.get("unionPad", UNION_PAD),
            "MIN_EDGE_PX": data.get("minEdgePx", MIN_EDGE_PX),
            "MAX_HEIGHT_SPREAD": limits.get("heightSpreadMax", MAX_HEIGHT_SPREAD),
            "MAX_HIDDEN_RATIO": data.get("maxHiddenRatio", MAX_HIDDEN_RATIO),
        })
        source = str(baseline_path.relative_to(root)) if baseline_path.is_relative_to(root) \
            else str(baseline_path)
    else:
        source = "脚本内置 v1.4 定版常量（未找到基准文件）"

    overrides = {
        "H_REF": args.h_ref, "FOOT_Y": args.foot_y,
        "CENTER_X": args.center_x, "SAFE_X": args.safe_x, "SAFE_Y": args.safe_y,
        "UNION_PAD": args.union_pad, "ALPHA_THRESHOLD": args.alpha_threshold,
        "WANTED_FRAMES": args.wanted_frames, "MIN_EDGE_PX": args.min_edge,
        "MAX_HIDDEN_RATIO": args.max_hidden,
        "MAX_HEIGHT_SPREAD": args.max_height_spread,
    }
    if args.canvas:
        width, height = (int(item) for item in args.canvas.lower().split("x"))
        values["CANVAS_W"], values["CANVAS_H"] = width, height
        overrides["CANVAS_W"], overrides["CANVAS_H"] = width, height
    changed = []
    for key, value in overrides.items():
        if value is not None and values[key] != value:
            values[key] = value
            changed.append("%s=%s" % (key, value))
    values["_source"] = source
    values["_cliOverrides"] = changed
    return values


def param_trace_lines(params: dict) -> list[str]:
    """给 build 留痕写「本次归一化传参」块，便于逐项复核。"""
    shown = {key: value for key, value in params.items()
             if not key.startswith("_")}
    return ["", "## 本次归一化传参（可由 CLI 覆盖）", "",
            "- 来源：`%s`" % params.get("_source", "脚本内置 v1.4 定版常量"),
            "- CLI 覆盖：%s" % ("、".join(params.get("_cliOverrides", []))
                              if params.get("_cliOverrides") else "无（与定版一致）"),
            "", "```json", json.dumps(shown, ensure_ascii=False, indent=2), "```"]

# 默认落包范围（2026-11-11 v1.4 已全部落包）：4 个新闲置动作 + 用 640×768 重生成修双臂的 stretch。
# 期望后续新增动作时通过 --actions 覆盖本元组，让同一套「锁定 h_ref」归一化算法可复用；
# 不传 --actions 时行为与 v1.4 定版完全一致，用 petctl.py doctor 的副本校验与旧留痕复跑验证。
TARGET_ACTIONS = ("stretch", "squatDraw", "wiggleDance", "kickflip", "miniGame")
NEW_ACTIONS = frozenset(("squatDraw", "wiggleDance", "kickflip", "miniGame"))

# 批次日志有三种 START 写法，必须全部兼容，否则段会被静默吞掉：
#   START stretch v1 offset=...          （原 batch.sh）
#   START(续跑) miniGame v1 offset=...   （断点续跑）
#   START(kickflip) v1 offset=...        （定向补跑，日志里没写动作名）
# 第三种缺动作名时从 work 目录名 sugt-pet-v14-<action>-vN 反解。
START_RE = re.compile(
    r"START(?:\([^)]*\))?\s+(?:(\w+)\s+)?v(\d+)\s+offset=\d+ work=(\S+)")
WORK_ACTION_RE = re.compile(r"sugt-pet-v14-([A-Za-z]+)-v\d+$")
DONE_RE = re.compile(r"DONE\s+(\w+) v(\d+)")
TOUCH_RE = re.compile(r"补间四边门禁 (通过|不通过) \| 四边实体 L/R/T/B=\[([^\]]+)\]")
# 只统计「已提交」行：同一 retry 的等待/完成行不能重复计数；
# start/retry 属朝向门禁重试，h3-*/retry 属补间四边门禁重试，两者分开。
H3_RETRY_RE = re.compile(r"h3-\d+/retry\d*\] 已提交")
FACE_RETRY_RE = re.compile(r"/start/retry\d*\] 已提交")


def load_prod(root: Path):
    """复用生产线与抠图脚本的工具函数，避免算法二次实现造成漂移。"""
    scripts = root / "docs" / "孙测测" / "scripts"
    sys.path.insert(0, str(scripts))
    import check_pet_keyvisual_cutout as key_module  # noqa: E402
    import comfyui_pet_action_series as prod  # noqa: E402
    return prod, key_module


def trace_path(root: Path, stem: str) -> Path:
    """生成本次执行的 Markdown 留痕路径。"""
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    return root / "docs" / "孙测测" / "traces" / ("%s-%s.md" % (stem, stamp))


# ---------------------------------------------------------------- assess


def assess(root: Path, batch_log: Path, picks_out: Path) -> int:
    """只读挑版：按批次日志分段打分，产出 picks.json 与留痕。"""
    text = batch_log.read_text(encoding="utf-8")
    lines = text.splitlines()
    segments: dict[str, dict] = {}
    current = None
    for line in lines:
        started = START_RE.search(line)
        if started:
            action, version, work = started.groups()
            if action is None:
                found = WORK_ACTION_RE.search(work.rstrip("/"))
                if not found:
                    raise ValueError("START 行缺动作名且 work 无法反解：%s" % line)
                action = found.group(1)
            current = "%s-v%s" % (action, version)
            segments[current] = {
                "action": action, "version": int(version), "work": work,
                "done": False, "touch": None, "touch_sum": None,
                "face_retry": 0, "h3_retry": 0, "lines": [],
            }
            continue
        if current is None:
            continue
        segments[current]["lines"].append(line)
        done = DONE_RE.search(line)
        if done and done.group(1) == segments[current]["action"]:
            segments[current]["done"] = True
            current = None

    rows = []
    for key, seg in sorted(segments.items()):
        state_file = Path(seg["work"]) / "state.json"
        video = Path(seg["work"]) / "videos" / ("%s.webp" % seg["action"])
        note = []
        if not seg["done"]:
            note.append("未 DONE")
        if not state_file.is_file():
            note.append("无 state.json")
        state = json.loads(state_file.read_text(encoding="utf-8")) if state_file.is_file() else {}
        if seg["action"] not in state.get("videos", {}):
            note.append("videos 缺该动作（补间门禁未过）")
        if state.get("rejected"):
            note.append("rejected=%d" % len(state["rejected"]))
        if not video.is_file():
            note.append("视频缺失")
        body = "\n".join(seg["lines"])
        # 同一候选会多次换 seed 重试，只有**最后一条**补间门禁代表最终产物；
        # 取第一条会把「失败 attempt」误当成该版本的四边成绩。
        touches = TOUCH_RE.findall(body)
        if touches:
            status, values_text = touches[-1]
            seg["touch"] = status
            values = [int(v.strip()) for v in values_text.split(",")
                      if v.strip().isdigit()]
            seg["touch_sum"] = sum(values)
        else:
            seg["touch_sum"] = 999
            note.append("无补间四边记录")
        seg["face_retry"] = len(FACE_RETRY_RE.findall(body))
        seg["h3_retry"] = len(H3_RETRY_RE.findall(body))
        seg["video_bytes"] = video.stat().st_size if video.is_file() else 0
        seg["timings"] = state.get("timings", {}).get(seg["action"], 0)

        score = 0
        if not note:
            score += 100
        if seg["touch"] == "通过" and seg["touch_sum"] == 0:
            score += 60
        # 最终四边余量比「生成时重试过几次」更能代表产物质量，权重更高。
        score -= 2 * seg["touch_sum"]
        score -= 8 * seg["h3_retry"]
        score -= 10 * seg["face_retry"]
        if seg["video_bytes"] > 15 * 1024 ** 2:
            score += 10
        seg["score"] = score
        seg["note"] = "；".join(note) if note else "硬门槛全过"
        rows.append(seg)

    picks = {}
    for action in TARGET_ACTIONS:
        candidates = [r for r in rows
                      if r["action"] == action and r["done"] and "未 DONE" not in r["note"]]
        if not candidates:
            print("  [跳过] %s 暂无完成候选" % action)
            continue
        best = max(candidates, key=lambda r: r["score"])
        picks[action] = best["work"]
        print("  [挑版] %-12s → %s (v%s, %d 分)" % (
            action, Path(best["work"]).name, best["version"], best["score"]))

    picks_out.write_text(json.dumps(picks, ensure_ascii=False, indent=2) + "\n",
                         encoding="utf-8")
    out = trace_path(root, "精灵v1.4候选挑版")
    md = [
        "# 精灵 v1.4 ComfyUI 候选挑版评估",
        "",
        "- 生成时间：%s" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 批次日志：`%s`" % batch_log,
        "- 评分：硬门槛 +100、最终四边全零 +60、最终触边实体 ×2 扣分、"
        "补间 retry ×8、朝向 retry ×10、视频 >15 MiB +10。"
        "取**最后一条**补间门禁代表最终产物。**分数只作初筛，必须人工目视复核**"
        "（kickflip 滑板 360° 翻转、stretch 双臂完整、squatDraw 蹲姿朝向）。",
        "- picks 输出：`%s`" % picks_out,
        "",
        "| 版本 | 动作 | 完成 | 四边 | L/R/T/B 合计 | 补间 retry | 朝向 retry | "
        "视频 MiB | H3 秒 | 分数 | 备注 |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for r in rows:
        md.append("| %s | %s | %s | %s | %s | %d | %d | %.1f | %.1f | %d | %s |" % (
            Path(r["work"]).name, r["action"], "是" if r["done"] else "否",
            r["touch"] or "-", r["touch_sum"], r["h3_retry"], r["face_retry"],
            r["video_bytes"] / 1024 ** 2, r["timings"], r["score"], r["note"]))
    md += ["", "## 推荐 picks", "", "```json", json.dumps(picks, ensure_ascii=False, indent=2),
           "```", "", "> 人工确认后再执行 --phase build。历史留痕只增不改。", ""]
    out.write_text("\n".join(md), encoding="utf-8")
    print("挑版留痕：%s" % out)
    print("picks：%s（%d/%d 个动作）" % (picks_out, len(picks), len(TARGET_ACTIONS)))
    if len(picks) != len(TARGET_ACTIONS):
        print("  [警告] 缺动作：%s（段解析不到或全部候选失败，禁止直接落包）"
              % "、".join(a for a in TARGET_ACTIONS if a not in picks))
        return 1
    return 0


# ----------------------------------------------------------------- build


def cutout_frames(prod, key_module, video: Path, cache: Path, action: str) -> list:
    """640×768 视频 → 50 帧透明 PNG 缓存；ComfyUI 阶段已过朝向门禁，此处不重判。"""
    cached = sorted(cache.glob("*.png"))
    if len(cached) == WANTED_FRAMES:
        return cached
    cache.mkdir(parents=True, exist_ok=True)
    with Image.open(video) as im:
        count = getattr(im, "n_frames", 1)
        if count < WANTED_FRAMES:
            raise RuntimeError("%s 仅 %d 帧，不足 %d" % (action, count, WANTED_FRAMES))
        indices = [round(i * (count - 1) / (WANTED_FRAMES - 1))
                   for i in range(WANTED_FRAMES)]
        for index, frame_index in enumerate(indices):
            im.seek(frame_index)
            rgb = im.convert("RGB")
            background = prod._corner_background(rgb)
            key_module.chroma_key(rgb, background).save(
                cache / ("%03d.png" % index), format="PNG")
        print("  [%s] 抠图 %d/%d 帧（源 %d 帧）" % (
            action, len(indices), WANTED_FRAMES, count), flush=True)
    cached = sorted(cache.glob("*.png"))
    if len(cached) != WANTED_FRAMES:
        raise RuntimeError("%s 抠图缓存 %d 帧 ≠ %d" % (action, len(cached), WANTED_FRAMES))
    return cached


def reference_heights(padded: Path, manifest: dict) -> dict:
    """老 20 动作补边素材的 alpha 高度中位数，作为「人物同高」交叉基准。"""
    result = {}
    for pose in manifest["poses"]:
        name = pose["name"]
        if name in NEW_ACTIONS or name == "stretch":
            continue
        path = padded / ("su-%s-loop.webp" % name)
        if not path.is_file():
            continue
        heights = []
        with Image.open(path) as im:
            for index in range(getattr(im, "n_frames", 1)):
                im.seek(index)
                alpha = im.convert("RGBA").getchannel("A").point(
                    lambda value: 255 if value > ALPHA_THRESHOLD else 0)
                box = alpha.getbbox()
                if box:
                    heights.append(box[3] - box[1])
        if heights:
            result[name] = sorted(heights)[len(heights) // 2]
    return result


def build(root: Path, picks_file: Path, source_out: Path, padded: Path,
          params: dict | None = None) -> int:
    """按 picks 归一化到 240×297，与老动作做同高交叉量测，输出源帧与留痕。"""
    prod, key_module = load_prod(root)
    picks = json.loads(picks_file.read_text(encoding="utf-8"))
    unknown = [a for a in picks if a not in TARGET_ACTIONS]
    if not picks or unknown:
        raise ValueError("picks 为空或含未知动作：%s" % unknown)
    # 允许只覆盖部分动作（流程 B：只重做某几个动作，其余不动）。
    chosen = [a for a in TARGET_ACTIONS if a in picks]
    print("本次归一化覆盖 %d/%d 个动作：%s" % (
        len(chosen), len(TARGET_ACTIONS), "、".join(chosen)))
    manifest = json.loads((root / "src" / "assets" / "pet" / "manifest.json").read_text(
        encoding="utf-8"))
    if (manifest["baseSize"]["w"], manifest["baseSize"]["h"]) != (CANVAS_W, CANVAS_H):
        raise AssertionError("manifest.baseSize 应为 240×297，实际 %s" % manifest["baseSize"])

    source_out.mkdir(parents=True, exist_ok=True)
    cutout_root = source_out.parent / "cutouts"
    target = (CANVAS_W, CANVAS_H)
    stats = {}
    problems = []

    for action in chosen:
        work = Path(picks[action])
        video = work / "videos" / ("%s.webp" % action)
        if not video.is_file():
            raise FileNotFoundError("候选视频缺失：%s" % video)
        # 缓存必须按「候选版本」隔离：只按动作名分目录时，挑版从 v1 改成 v2
        # 会静默复用试跑的 50 帧缓存，最终落包的是错版本。
        paths = cutout_frames(prod, key_module, video,
                              cutout_root / ("%s-%s" % (action, work.name)), action)

        boxes = []
        frame_size = None
        for path in paths:
            with Image.open(path) as frame:
                frame_size = frame.size
                boxes.append(prod.tight_box(frame.getchannel("A")))
        boxes = [box for box in boxes if box]
        if not boxes:
            raise RuntimeError("%s 全部帧无有效 alpha bbox" % action)
        rep = tuple(prod._quantile([box[i] for box in boxes], 0.5) for i in range(4))
        union = (min(b[0] for b in boxes), min(b[1] for b in boxes),
                 max(b[2] for b in boxes), max(b[3] for b in boxes))
        left, top, right, bottom = rep
        # 与生产线完全同式，但 h_ref / foot_y 锁定为 v1.3.3 实测值；
        # 画布由 180 换成 240，仅把水平中心 90 → 120，人物像素高不变。
        factor = H_REF / (bottom - top)
        offset = (round(CENTER_X - (left + right) / 2 * factor),
                  round(FOOT_Y - bottom * factor))
        ul, ut, ur, ub = union
        safe_box = (max(0, ul - UNION_PAD), max(0, ut - UNION_PAD),
                    min(frame_size[0], ur + UNION_PAD),
                    min(frame_size[1], ub + UNION_PAD))

        def hidden_ratio(frame):
            """并集安全框外实体像素占比；>2% 说明有肢体被落在框外。"""
            solid = frame.getchannel("A").point(
                lambda value: 255 if value > ALPHA_THRESHOLD else 0)
            total = solid.histogram()[255]
            if not total:
                return 0.0
            outside = solid.copy()
            outside.paste(0, safe_box)
            return outside.histogram()[255] / total

        def normalise(frame):
            """纯缩放 + 平移；不裁一像素人物。"""
            cleaned = frame.copy()
            keep = Image.new("L", frame.size, 0)
            keep.paste(255, safe_box)
            cleaned.putalpha(ImageChops.multiply(cleaned.getchannel("A"), keep))
            size = (max(1, round(frame.width * factor)),
                    max(1, round(frame.height * factor)))
            resized = cleaned.resize(size, Image.LANCZOS)
            fitted = Image.new("RGBA", target, (0, 0, 0, 0))
            fitted.paste(resized, offset, resized)
            return fitted

        normalised, heights, edges, worst_hidden = [], [], [], 0.0
        for path in paths:
            with Image.open(path) as source_frame:
                frame = source_frame.convert("RGBA")
            worst_hidden = max(worst_hidden, hidden_ratio(frame))
            fitted = normalise(frame)
            alpha = fitted.getchannel("A").point(
                lambda value: 255 if value > ALPHA_THRESHOLD else 0)
            box = alpha.getbbox()
            if box is None:
                edges.append((CANVAS_H, CANVAS_W, CANVAS_H, CANVAS_W))
                heights.append(0)
            else:
                edges.append((box[1], CANVAS_W - box[2],
                              CANVAS_H - box[3], box[0]))
                heights.append(box[3] - box[1])
            normalised.append(fitted)
        min_edge = min(min(edge) for edge in edges)
        median_height = sorted(heights)[len(heights) // 2]
        stats[action] = {
            "work": str(work), "scale": round(factor, 4), "offset": offset,
            "height": median_height, "min_edge": min_edge,
            "hidden": round(worst_hidden, 5), "union": list(union),
            "size": "%dx%d" % target,
        }
        if min_edge < MIN_EDGE_PX:
            problems.append("%s 最小四边留白 %dpx < %dpx" % (
                action, min_edge, MIN_EDGE_PX))
        if worst_hidden > MAX_HIDDEN_RATIO:
            problems.append("%s 并集框外实体 %.3f%% > 2%%" % (
                action, worst_hidden * 100))
        loop_out = source_out / ("su-%s-loop.webp" % action)
        normalised[0].save(loop_out, format="WEBP", save_all=True,
                           append_images=normalised[1:], duration=100,
                           loop=0, lossless=True, method=4)
        normalised[0].save(source_out / ("su-%s.webp" % action),
                           format="WEBP", lossless=True, method=4)
        print("  [%s] scale=%.4f 人物高=%dpx 最小留白=%dpx 框外=%.3f%%" % (
            action, factor, median_height, min_edge, worst_hidden * 100), flush=True)

    # 与老 20 动作交叉量测：人物必须同高，否则会出现「高矮胖瘦」。
    refs = reference_heights(padded, manifest) if padded.is_dir() else {}
    if refs:
        all_h = list(refs.values()) + [s["height"] for s in stats.values()]
        low, high = min(all_h), max(all_h)
        spread = high / low - 1 if low else float("inf")
        print("  [同高交叉量测] 老 %d 动作 + 新 %d 动作：%d~%d px，极差 %.2f%%" % (
            len(refs), len(stats), low, high, spread * 100))
        if spread > MAX_HEIGHT_SPREAD:
            problems.append("新老动作人物高度极差 %.2f%% > %.0f%%" % (
                spread * 100, MAX_HEIGHT_SPREAD * 100))
    else:
        print("  [同高交叉量测] 跳过：补边源目录不存在 %s" % padded)

    # 合并老 41 张（不覆盖刚生成的 10 张，stretch 用新素材替换）。
    added = 0
    if padded.is_dir():
        for path in sorted(padded.glob("*.webp")):
            dst = source_out / path.name
            if not dst.is_file():
                shutil.copy2(path, dst)
                added += 1
    count = len(list(source_out.glob("*.webp")))
    # 期望张数由 manifest 推导：每个动作 poster + loop，再加 extraFrames。
    # 这样新增动作后不必再改一处硬编码，用 build 的留痕张数与 doctor 核对验证。
    expected_count = 2 * len(manifest["poses"]) + len(manifest.get("extraFrames", []))
    if len(chosen) == len(TARGET_ACTIONS) and count != expected_count:
        problems.append("全量落包时源目录应有 %d 张，实际 %d 张（新增拷贝 %d）" % (
            expected_count, count, added))

    out = trace_path(root, "精灵v1.4新动作归一化")
    md = [
        "# 精灵 v1.4 新动作归一化与同高量测",
        "",
        "- 生成时间：%s" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 画布：`240×297`（老 180 画布左右各补 30px 全透明）；水平中心 `90 → 120`",
        "- **锁定**：`h_ref=%.2f`、`foot_y=%d`，来自 `/tmp/sugt-pet-actions-v13/state.json`；"
        "画布高度 297 不变，人物像素高与老 20 动作一致。" % (H_REF, FOOT_Y),
        "- 归一化：并集安全框 + 纯缩放(LANCZOS) + 平移，**不裁一像素人物**；"
        "并集框外只清色键噪点。",
        "- picks：`%s`；源帧输出：`%s`" % (picks_file, source_out),
        "",
        "| 动作 | 候选目录 | scale | 平移 | 人物高中位(px) | 最小四边留白(px) | 框外实体 |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for action, s in stats.items():
        md.append("| %s | `%s` | %.4f | %s | %d | %d | %.3f%% |" % (
            action, Path(s["work"]).name, s["scale"], s["offset"], s["height"],
            s["min_edge"], s["hidden"] * 100))
    md += param_trace_lines(params or {})
    md += ["", "## 与老 20 动作同高交叉量测", ""]
    if refs:
        md.append("老动作基准（alpha 高度中位数）：%d~%d px，共 %d 个；"
                  "含新动作后极差 %.2f%%（阈值 ≤%.0f%%）。" % (
                      min(refs.values()), max(refs.values()), len(refs),
                      spread * 100, MAX_HEIGHT_SPREAD * 100))
        md.append("")
        md.append("```json")
        md.append(json.dumps(refs, ensure_ascii=False, indent=2))
        md.append("```")
    else:
        md.append("未执行（补边源目录不存在）。")
    md += ["", "## 自检结论", ""]
    md.append("源目录共 %d 张（期望 %d）。" % (count, expected_count))
    md.append("结论：%s" % ("全部通过" if not problems else "不通过：" + "；".join(problems)))
    md += ["", "> 需人工目视复核 kickflip 滑板与 stretch 双臂后再 --phase apply。", ""]
    out.write_text("\n".join(md), encoding="utf-8")
    print("归一化留痕：%s" % out)
    if problems:
        for item in problems:
            print("  [不通过] %s" % item)
        return 1
    print("归一化自检通过：%d 个动作 / 源目录 %d 张" % (len(stats), count))
    return 0


# ----------------------------------------------------------------- apply


def apply(root: Path, source_out: Path, backup: Path,
          py: str = "/usr/local/bin/python3.13") -> int:
    """临时 root 编码目标动作，把产物落回真实进包目录，凑齐 manifest 张数。"""
    assets = root / "src" / "assets" / "pet"
    real_manifest = assets / "manifest.json"
    data = json.loads(real_manifest.read_text(encoding="utf-8"))

    needed = set()
    for action in TARGET_ACTIONS:
        needed.add("su-%s.webp" % action)
        needed.add("su-%s-loop.webp" % action)
    missing = [name for name in sorted(needed) if not (source_out / name).is_file()]
    if missing:
        raise FileNotFoundError("源帧缺失：%s" % "、".join(missing))

    # 备份当前 41 张，便于回滚。
    backup.mkdir(parents=True, exist_ok=True)
    for path in assets.glob("*.webp"):
        shutil.copy2(path, backup / path.name)

    # 期望每次落包用独立临时 root：v1.4 定版写死 `finalize-root`，重跑时
    # 旧批次的 .webp 会残留并被算进「编码产物张数」，实测 2 张被误判成 12 张。
    # 用目录名 + 时间戳隔离，不删任何历史产物，用 apply 的产物张数断言验证。
    tmp_root = source_out.parent / ("finalize-root-%s-%s" % (
        source_out.name, datetime.now().strftime("%H%M%S")))
    tmp_assets = tmp_root / "src" / "assets" / "pet"
    tmp_assets.mkdir(parents=True, exist_ok=True)
    (tmp_root / "docs" / "孙测测" / "traces").mkdir(parents=True, exist_ok=True)
    subset = dict(data)
    subset["poses"] = [p for p in data["poses"] if p["name"] in TARGET_ACTIONS]
    subset["posePriority"] = [n for n in data["posePriority"] if n in TARGET_ACTIONS]
    subset["extraFrames"] = []
    subset["actionRegistry"] = dict(data["actionRegistry"])
    subset["actionRegistry"]["frameBudget"] = dict(
        data["actionRegistry"]["frameBudget"])
    subset["actionRegistry"]["frameBudget"]["total"] = sum(
        p["frames"] for p in subset["poses"])
    if len(subset["poses"]) != len(TARGET_ACTIONS):
        raise AssertionError("临时清单动作数应为 %d" % len(TARGET_ACTIONS))
    (tmp_assets / "manifest.json").write_text(
        json.dumps(subset, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    trace = trace_path(root, "精灵v1.4新动作落包")
    cmd = [py, str(root / "docs" / "孙测测" / "scripts" / "pet_reencode_assets.py"),
           "--root", str(tmp_root), "--source", str(source_out), "--out", str(trace)]
    print("编码：%s" % " ".join(cmd))
    done = subprocess.run(cmd, cwd=str(root), capture_output=True,
                          text=True, timeout=1800)
    print(done.stdout[-2500:])
    if done.returncode != 0:
        print(done.stderr[-2000:], file=sys.stderr)
        raise RuntimeError("pet_reencode_assets.py 退出码 %d" % done.returncode)

    products = sorted(tmp_assets.glob("*.webp"))
    if len(products) != len(needed):
        raise AssertionError("编码产物应为 %d 张，实际 %d" % (len(needed), len(products)))
    for path in products:
        shutil.copy2(path, assets / path.name)

    bad = []
    for path in sorted(assets.glob("*.webp")):
        with Image.open(path) as image:
            if image.size != (CANVAS_W, CANVAS_H):
                bad.append("%s 尺寸 %s" % (path.name, image.size))
            frames = getattr(image, "n_frames", 1)
            expected = 1 if path.name == "su-blink.webp" or not path.name.endswith(
                "-loop.webp") else WANTED_FRAMES
            if frames != expected:
                bad.append("%s 帧数 %d ≠ %d" % (path.name, frames, expected))
    total = sum(path.stat().st_size for path in assets.glob("*.webp"))
    count = len(list(assets.glob("*.webp")))
    expected_count = 2 * len(data["poses"]) + len(data.get("extraFrames", []))
    if bad or count != expected_count:
        raise AssertionError("落包复核不通过：%s；张数 %d（期望 %d）" % (
            "；".join(bad[:6]), count, expected_count))
    print("落包完成：%d 张 / 240×297 / 合计 %d B（%.2f MiB）" % (
        count, total, total / 1024 ** 2))
    print("回滚备份：%s" % backup)
    print("下一步：跑 gen_pet_from_manifest.py 写模式回填 %d 个 hitBox（不要用 --emit-only）"
          % len(data["poses"]))
    return 0


def main() -> int:
    global TARGET_ACTIONS, CANVAS_W, CANVAS_H, CENTER_X, H_REF, FOOT_Y
    global SAFE_X, SAFE_Y, WANTED_FRAMES, ALPHA_THRESHOLD, UNION_PAD
    global MIN_EDGE_PX, MAX_HEIGHT_SPREAD, MAX_HIDDEN_RATIO
    parser = argparse.ArgumentParser(
        description="新动作挑版 / 归一化 / 落包桥接（锁定 h_ref，v1.4 定版算法）")
    parser.add_argument("--phase", required=True, choices=("assess", "build", "apply"))
    parser.add_argument("--root", required=True, help="仓库根目录绝对路径")
    parser.add_argument("--actions", default="",
                        help="逗号分隔的动作名，覆盖 v1.4 默认 5 个；新增动作时指定，"
                             "不传则与 v1.4 定版行为一致")
    parser.add_argument("--batch-log", default="/tmp/sugt-pet-v14-batch.log")
    parser.add_argument("--picks", default="/tmp/sugt-pet-v14-picks.json")
    parser.add_argument("--source-out", default="/tmp/sugt-pet-v14-final-source")
    parser.add_argument("--padded-source", default="/tmp/sugt-pet-v14-padded/source")
    parser.add_argument("--backup", default="/tmp/sugt-pet-v14-backup-41")
    parser.add_argument("--python", default="/usr/local/bin/python3.13",
                        help="apply 阶段调用重编码脚本的解释器")
    parser.add_argument("--normalize-baseline", default=str(BASELINE_REL),
                        help="归一化基准 JSON（默认仓库 config 下的唯一真相源）")
    parser.add_argument("--no-baseline", action="store_true",
                        help="忽略基准文件，用脚本内置 v1.4 常量复跑定版行为")
    group = parser.add_argument_group("归一化传参（覆盖基准 / 内置常量）")
    group.add_argument("--canvas", default="", help="画布，形如 240x297")
    group.add_argument("--h-ref", type=float, default=None, help="目标人物身高 px")
    group.add_argument("--foot-y", type=int, default=None, help="脚底同线 y")
    group.add_argument("--center-x", type=float, default=None, help="水平中心")
    group.add_argument("--safe-x", type=int, default=None, help="水平安全边")
    group.add_argument("--safe-y", type=int, default=None, help="垂直安全边")
    group.add_argument("--union-pad", type=int, default=None, help="并集框外扩像素")
    group.add_argument("--alpha-threshold", type=int, default=None, help="alpha 实体阈值")
    group.add_argument("--wanted-frames", type=int, default=None, help="每动作帧数")
    group.add_argument("--min-edge", type=int, default=None, help="最小四边留白")
    group.add_argument("--max-hidden", type=float, default=None,
                       help="并集框外实体占比上限")
    group.add_argument("--max-height-spread", type=float, default=None,
                       help="跨动作身高极差上限（定版 0.08，不放宽）")
    args = parser.parse_args()
    if args.actions:
        names = tuple(item.strip() for item in args.actions.split(",") if item.strip())
        if not names:
            raise ValueError("--actions 解析后为空")
        TARGET_ACTIONS = names
        print("本次落包动作（--actions）：%s" % "、".join(names), flush=True)
    root = Path(args.root).resolve()
    params = load_normalize_params(root, args)
    # 归一化参数在入口一次性注入；build / apply 的算法主体保持 v1.4 定版不变。
    for name, _key in PARAM_KEYS:
        globals()[name] = params[name]
    print("归一化传参来源：%s | CLI 覆盖：%s" % (
        params["_source"], "、".join(params["_cliOverrides"]) or "无"), flush=True)
    if args.phase == "assess":
        return assess(root, Path(args.batch_log), Path(args.picks))
    if args.phase == "build":
        return build(root, Path(args.picks), Path(args.source_out),
                     Path(args.padded_source), params)
    return apply(root, Path(args.source_out), Path(args.backup), args.python)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, AssertionError, RuntimeError,
            json.JSONDecodeError) as exc:
        print("v1.4 落包桥接失败：%s" % exc, file=sys.stderr)
        sys.exit(2)
