#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：v1.4 画布扩边——把 180×297 的精灵逐帧素材**纯 paste** 到 240×297
         全透明画布中央（左右各 +30px），不缩放、不裁切、不重算归一化；
         `--sample` 先做单动作样板并量「人物像素高 / 包体积增量 / contain factor /
         三档窗口加宽值」，`--write` 批量补边供 pet_reencode_assets.py 整套重编码。
适用项目：SUGT（桌面精灵 小酥）
创建日期：2026-11-11
复用方式：
  - 样板（只读仓库，产物写 /tmp）：
      /usr/local/bin/python3.13 docs/孙测测/scripts/pet_v14_pad_canvas.py --sample
  - 批量补边（产物写到 --out，供重编码链消费）：
      /usr/local/bin/python3.13 docs/孙测测/scripts/pet_v14_pad_canvas.py --write \
          --source-dir /tmp/sugt-pet-actions-v13/source \
          --out /tmp/sugt-pet-v14-padded/source
  - 上下文与红线见 docs/小精灵维护/v1.4执行契约.md
退出码约定：0 全部通过 / 1 校验不通过（人物像素被改动等） / 2 读取、解析或依赖缺失
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path

try:
    from PIL import Image, features
except ImportError as exc:  # pragma: no cover - 依赖缺失走退出码 2
    raise SystemExit("本脚本需要 Pillow（本机 /usr/local/bin/python3.13）") from exc

OLD_W, OLD_H = 180, 297
NEW_W, NEW_H = 240, 297
PAD_X = (NEW_W - OLD_W) // 2
SIDE_RATIO = 0.28          # pet.css:19 --pet-side-w
INSET_X, INSET_Y = 8, 8    # pet.css:24-25
SAFETY = 1.05              # 高度主导断言留 5% 余量
OLD_SIZES = [(120, 136), (148, 168), (180, 204)]
TRACE_REL = Path("docs") / "孙测测" / "traces"


def parse_args() -> argparse.Namespace:
    """参数化输入，不硬编码本机绝对路径。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(description="精灵画布左右补透明边（纯 paste）")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--sample", action="store_true", help="单动作样板 + 量数据，不写仓库")
    mode.add_argument("--write", action="store_true", help="批量补边到 --out")
    parser.add_argument("--source-dir", type=Path,
                        default=Path("/tmp/sugt-pet-actions-v13/source"),
                        help="源帧目录（41 张 180×297 WebP）")
    parser.add_argument("--out", type=Path, default=None, help="产物目录")
    parser.add_argument("--colors", type=int, default=56, help="样板量化色数，与 encoding.colors 一致")
    return parser.parse_args()


def load_frames(path: Path) -> tuple[list, list[int]]:
    """读出全部 RGBA 帧与逐帧时长；源帧尺寸必须是 180×297。"""
    with Image.open(path) as image:
        if image.size != (OLD_W, OLD_H):
            raise ValueError(f"{path.name} 尺寸 {image.size} ≠ {OLD_W}×{OLD_H}")
        frames, durations = [], []
        for index in range(getattr(image, "n_frames", 1)):
            image.seek(index)
            frames.append(image.convert("RGBA"))
            durations.append(int(image.info.get("duration") or 120))
    return frames, durations


def pad_frames(frames: list) -> list:
    """把每帧贴到 240×297 全透明画布中央；人物像素 1:1，不做任何缩放。"""
    out = []
    for frame in frames:
        canvas = Image.new("RGBA", (NEW_W, NEW_H), (0, 0, 0, 0))
        canvas.paste(frame, (PAD_X, 0))
        out.append(canvas)
    return out


def save_frames(frames: list, durations: list[int], dst: Path, colors: int) -> int:
    """按既有编码链口径量化并写 lossless WebP（单帧 / 动画通用）。"""
    quantized = [
        frame.quantize(colors=colors, method=Image.Quantize.FASTOCTREE,
                       dither=Image.Dither.NONE)
        for frame in frames
    ]
    dst.parent.mkdir(parents=True, exist_ok=True)
    if len(quantized) == 1:
        quantized[0].save(dst, format="WEBP", lossless=True, method=4)
    else:
        quantized[0].save(dst, format="WEBP", save_all=True,
                          append_images=quantized[1:], duration=durations,
                          loop=0, lossless=True, method=4)
    return dst.stat().st_size


def alpha_bbox(frame) -> tuple[int, int, int, int] | None:
    """返回非透明像素包围盒，用于证明人物没有被裁切或位移。"""
    return frame.getchannel("A").getbbox()


def box_of(window_w: int, window_h: int) -> tuple[float, float]:
    """按 pet.css 的 .pet-figure 定位算出实际绘制盒子。"""
    return (1 - SIDE_RATIO) * window_w - INSET_X, window_h - 2 * INSET_Y


def factor_of(box_w: float, box_h: float, canvas_w: int, canvas_h: int) -> float:
    """background-size: contain 的等比缩放系数。"""
    return min(box_w / canvas_w, box_h / canvas_h)


def required_width(window_h: int, canvas_w: int, canvas_h: int) -> float:
    """保持「高度主导」且留 5% 余量所需的最小窗口宽度。"""
    box_h = window_h - 2 * INSET_Y
    return (SAFETY * canvas_w * box_h / canvas_h + INSET_X) / (1 - SIDE_RATIO)


def write_trace(root: Path, lines: list[str]) -> Path:
    """每次执行生成一份 Markdown 留痕。"""
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = root / TRACE_REL / f"精灵v1.4扩边留痕-{stamp}.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def do_sample(root: Path, args: argparse.Namespace) -> int:
    """单动作样板：证明补边不变形、体积基本不变、并算出三档窗口加宽值。"""
    assets = root / "src" / "assets" / "pet"
    out = args.out or Path("/tmp/sugt-pet-v14-sample")
    names = ("su-idle.webp", "su-idle-loop.webp")
    rows, lines = [], [
        "# 精灵 v1.4 扩边样板留痕",
        "",
        f"- 执行时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        f"- 画布：{OLD_W}×{OLD_H} → {NEW_W}×{NEW_H}（左右各 +{PAD_X}px 全透明）",
        "- 方式：逐帧纯 paste，不缩放、不裁切、不重算 h_ref",
        "",
        "| 素材 | 原字节 | 补边后字节 | 增量 | 原 alpha bbox | 补边后 alpha bbox | 人物像素一致 |",
        "| --- | --- | --- | --- | --- | --- | --- |",
    ]
    for name in names:
        src = assets / name
        frames, durations = load_frames(src)
        padded = pad_frames(frames)
        dst = out / name
        size = save_frames(padded, durations, dst, args.colors)
        old_box, new_box = alpha_bbox(frames[0]), alpha_bbox(padded[0])
        shifted = (old_box[0] + PAD_X, old_box[1], old_box[2] + PAD_X, old_box[3])
        same = shifted == new_box and len(padded) == len(frames)
        delta = size - src.stat().st_size
        rows.append(same)
        lines.append(
            f"| {name} | {src.stat().st_size} B | {size} B | "
            f"{delta:+} B ({100 * delta / src.stat().st_size:+.2f}%) | "
            f"{old_box} | {new_box} | {'通过' if same else '不通过'} |"
        )
        print(f"[{'通过' if same else '不通过'}] {name}: "
              f"{src.stat().st_size} → {size} B, bbox {old_box} → {new_box}")

    lines += ["", "## contain 高度主导不变式与三档窗口加宽", "",
              "| 档 | 原窗口 | 原 factor | 不加宽直接扩边 factor | 需最小宽度 | 建议新窗口 | 新 factor | 人物像素高变化 |",
              "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    for window_w, window_h in OLD_SIZES:
        box_w, box_h = box_of(window_w, window_h)
        old_factor = factor_of(box_w, box_h, OLD_W, OLD_H)
        raw_factor = factor_of(box_w, box_h, NEW_W, NEW_H)
        need = required_width(window_h, NEW_W, NEW_H)
        suggested = int(window_w * NEW_W / OLD_W + 0.999)
        if suggested < need:
            suggested = int(need + 1.999)
        new_box_w, _ = box_of(suggested, window_h)
        new_factor = factor_of(new_box_w, box_h, NEW_W, NEW_H)
        delta_pct = 100 * (new_factor - old_factor) / old_factor
        lines.append(
            f"| {window_w}×{window_h} | {window_w}×{window_h} | {old_factor:.4f} | "
            f"{raw_factor:.4f}（缩 {100 * (old_factor - raw_factor) / old_factor:.1f}%） | "
            f"{need:.1f} | **{suggested}×{window_h}** | {new_factor:.4f} | {delta_pct:+.2f}% |"
        )
        print(f"窗口 {window_w}×{window_h} → {suggested}×{window_h}: "
              f"factor {old_factor:.4f} → {new_factor:.4f} ({delta_pct:+.2f}%)")

    ok = all(rows)
    lines += ["", f"- 结论：{'通过' if ok else '不通过'}",
              "- 复核命令：`pet_v14_pad_canvas.py --sample` / `pet_reencode_assets.py --check` / `check_pet_final_frames.py`", ""]
    trace = write_trace(root, lines)
    print(f"留痕：{trace}")
    return 0 if ok else 1


def do_write(root: Path, args: argparse.Namespace) -> int:
    """批量补边：产物保持同名，供 pet_reencode_assets.py --source 直接消费。"""
    source = args.source_dir.resolve()
    out = (args.out or Path("/tmp/sugt-pet-v14-padded/source")).resolve()
    if not source.is_dir():
        raise FileNotFoundError(f"源目录不存在: {source}")
    files = sorted(source.glob("*.webp"))
    if not files:
        raise FileNotFoundError(f"源目录没有 WebP: {source}")
    lines = [
        "# 精灵 v1.4 批量扩边留痕",
        "",
        f"- 执行时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        f"- 源目录：{source}（{len(files)} 张）",
        f"- 产物目录：{out}",
        f"- 画布：{OLD_W}×{OLD_H} → {NEW_W}×{NEW_H}，左右各 +{PAD_X}px 全透明",
        "",
        "| 素材 | 帧数 | 人物 alpha bbox 位移 | 结果 |",
        "| --- | --- | --- | --- |",
    ]
    total_frames = 0
    for path in files:
        frames, durations = load_frames(path)
        padded = pad_frames(frames)
        old_box, new_box = alpha_bbox(frames[0]), alpha_bbox(padded[0])
        shifted = (old_box[0] + PAD_X, old_box[1], old_box[2] + PAD_X, old_box[3])
        ok = shifted == new_box and len(frames) == len(padded)
        save_frames(padded, durations, out / path.name, args.colors)
        total_frames += len(frames)
        lines.append(f"| {path.name} | {len(frames)} | {old_box} → {new_box} | "
                     f"{'通过' if ok else '不通过'} |")
        if not ok:
            print(f"[不通过] {path.name}: bbox {old_box} → {new_box}")
            write_trace(root, lines + ["", "- 结论：不通过", ""])
            return 1
        print(f"[通过] {path.name}: {len(frames)} 帧 → {NEW_W}×{NEW_H}")
    lines += ["", f"- 合计：{len(files)} 张 / {total_frames} 帧", "- 结论：通过", ""]
    trace = write_trace(root, lines)
    print(f"批量补边完成：{len(files)} 张 / {total_frames} 帧 → {out}")
    print(f"留痕：{trace}")
    return 0


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    if not features.check("webp") or not features.check("webp_anim"):
        print("[失败] 当前 Pillow 缺少 WebP / 动画 WebP 支持", file=sys.stderr)
        return 2
    try:
        return do_sample(root, args) if args.sample else do_write(root, args)
    except (OSError, ValueError, json.JSONDecodeError, RuntimeError) as exc:
        print(f"[失败] 读取、解析或编码异常: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
