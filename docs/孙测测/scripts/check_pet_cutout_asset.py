#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-017 桌面精灵「已抠透明 PNG」进包验收门禁。

编写人员：孙测测
作用用途：
  SCR-016 只验「从带底色原图能否抠干净」，无法发现抠图工具自身引入的
  轮廓溢色（1~3px 绿 / 青描边）、孤立噪点、alpha=0 像素残留彩色 RGB、
  以及量化后的体积超标。本脚本对**已经抠好**的透明 PNG 做七项判定：
    1. 透明占比 >= 30%；              2. alpha bbox 面积 <= 95%；
    3. 青蓝底残留 <= 0.5%；           4. 轮廓溢色（绿 / 青）<= 1%；
    5. 面积 < 50px 的孤立连通块 = 0； 6. 白色衣物保留率 >= 99%（需 --source）；
    7. 105x297 / 96 色 lossless 单帧 <= 8 MB / 1000 帧。
  ``--fix`` 迭代用「向内最近的非溢色邻色」替换 1~3px 溢色边（保留轮廓
  形状与 alpha），并清空 alpha=0 像素的 RGB 垃圾；产修补版 PNG、小窗
  WebP 与浅 / 深 / 棋盘 / 小窗四联对照。
  硬红线：产物只落输出目录与 traces，**绝不写入 src/assets/pet**。
适用项目：SUGT 桌面精灵 v1.2 素材链路（本地执行，不依赖 ComfyUI / 网络）
创建日期：2026-11-10
复用方式：
  python3 check_pet_cutout_asset.py --image good.png \
      --source 20261110013708_8_5.jpg --fix --out /tmp/sugt-cutout-asset
  --image 可重复；--source 需与 --image 同尺寸；不带 --source 时第 6 项跳过。
  需 Pillow（本地素材链路脚本，不进 CI）。
退出码约定：0 全部通过 / 1 存在验收不通过 / 2 参数、依赖或解析异常
"""
import argparse
import io
import sys
from collections import deque
from datetime import datetime
from pathlib import Path

try:
    from PIL import Image, ImageDraw
except ImportError:
    print("缺少 Pillow，本脚本只在本地素材链路运行")
    sys.exit(2)

_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parents[2]
_TRACES = _ROOT / "docs" / "孙测测" / "traces"

# 验收阈值：青蓝底、赛璐璐平涂、96 色 palette-lossless 的实测口径。
_MIN_TRANSPARENT_RATIO = 0.30
_MAX_BBOX_RATIO = 0.95
_MAX_CHROMA_RESIDUE = 0.005
_MAX_EDGE_SPILL = 0.01
_MAX_SMALL_COMPS = 0
_MIN_WHITE_KEEP = 0.99
_MAX_FRAME_BYTES = 8 * 1024 * 1024 / 1000  # 1000 帧 <= 8 MB
_SMALL_COMP_AREA = 50
_PALETTE_COLORS = 96
_BASE_SIZE = (105, 297)
# 溢出到角色轮廓的 1~3px 描边：绿优势或青蓝优势都算。
_CHROMA_EDGE_MIN = 20


def chroma_score(color):
    """青蓝色度优势：背景为正，角色黑 / 灰 / 紫红 / 薄荷绿均 <= 0。"""
    red, green, blue = color
    return min(green - red, blue - green)


def spill(color):
    """轮廓溢色判据：绿优势或青蓝优势超过阈值。"""
    red, green, blue = color[:3]
    greenish = green > red + 20 and green > blue + 20
    return greenish or chroma_score((red, green, blue)) >= _CHROMA_EDGE_MIN


def edge_indexes(pixels, width, height):
    """返回与透明像素相邻的不透明像素下标集合。"""
    result = set()
    for y in range(1, height - 1):
        for x in range(1, width - 1):
            i = y * width + x
            if pixels[i][3] <= 0:
                continue
            neighbors = (i - 1, i + 1, i - width, i + width)
            if any(pixels[j][3] == 0 for j in neighbors):
                result.add(i)
    return result


def fix_image(image):
    """去溢色 + 清空透明区 RGB 垃圾，返回 (修补图, 替换像素数, 剩余溢色数)。"""
    width, height = image.size
    pixels = list(image.convert("RGBA").getdata())
    edges = edge_indexes(pixels, width, height)
    targets = {i for i in edges if spill(pixels[i])}
    replaced = 0
    # 每轮把「已有非溢色内邻」的溢色像素改成邻色均值，逐步向内收敛。
    for _ in range(4):
        changed = 0
        for i in list(targets):
            x, y = i % width, i // width
            candidates = []
            for dx, dy in ((-1, 0), (1, 0), (0, -1), (0, 1),
                           (-1, -1), (1, -1), (-1, 1), (1, 1)):
                nx, ny = x + dx, y + dy
                if 0 <= nx < width and 0 <= ny < height:
                    neighbor = pixels[ny * width + nx]
                    if neighbor[3] > 0 and not spill(neighbor):
                        candidates.append(neighbor[:3])
            if not candidates:
                continue
            color = tuple(round(sum(c[k] for c in candidates) / len(candidates))
                          for k in range(3))
            pixels[i] = (color[0], color[1], color[2], pixels[i][3])
            targets.discard(i)
            changed += 1
        replaced += changed
        if not changed:
            break
    # alpha=0 的 RGB 归零：避免量化后被当成彩色噪点，也省 WebP 字节。
    pixels = [c if c[3] > 0 else (0, 0, 0, 0) for c in pixels]
    fixed = Image.new("RGBA", (width, height))
    fixed.putdata(pixels)
    remain = sum(1 for i in edge_indexes(pixels, width, height)
                 if spill(pixels[i]))
    return fixed, replaced, remain


def small_components(image):
    """返回面积小于阈值的孤立连通块数量与最大块面积。"""
    width, height = image.size
    alpha = image.getchannel("A").tobytes()
    occupied = bytearray(1 if a > 0 else 0 for a in alpha)
    seen = bytearray(width * height)
    sizes = []
    for start in range(width * height):
        if not occupied[start] or seen[start]:
            continue
        queue = deque([start])
        seen[start] = 1
        size = 0
        while queue:
            i = queue.popleft()
            size += 1
            x, y = i % width, i // width
            neighbors = (i - 1 if x > 0 else -1,
                         i + 1 if x < width - 1 else -1,
                         i - width if y > 0 else -1,
                         i + width if y < height - 1 else -1)
            for j in neighbors:
                if j >= 0 and occupied[j] and not seen[j]:
                    seen[j] = 1
                    queue.append(j)
        sizes.append(size)
    sizes.sort(reverse=True)
    small = [s for s in sizes[1:] if s < _SMALL_COMP_AREA]
    return len(small), (sizes[0] if sizes else 0), len(sizes)


def frame_bytes(image):
    """按 baseSize contain 缩小、96 色量化、lossless WebP，返回 (字节数, 图, 利用率)。"""
    box = image.getbbox()
    crop = image.crop(box) if box else image
    canvas = Image.new("RGBA", _BASE_SIZE, (0, 0, 0, 0))
    fit = crop.copy()
    fit.thumbnail(_BASE_SIZE, Image.LANCZOS)
    canvas.alpha_composite(fit, ((_BASE_SIZE[0] - fit.width) // 2,
                                 (_BASE_SIZE[1] - fit.height) // 2))
    quantized = canvas.quantize(colors=_PALETTE_COLORS,
                                method=Image.Quantize.FASTOCTREE).convert("RGBA")
    buffer = io.BytesIO()
    quantized.save(buffer, format="WEBP", lossless=True, method=4)
    return buffer.tell(), quantized, fit.height / _BASE_SIZE[1]


def measure(image, source):
    """执行七项判定，返回 (检查列表, 指标字典)。"""
    width, height = image.size
    pixels = list(image.convert("RGBA").getdata())
    total = width * height
    occupied = [i for i, c in enumerate(pixels) if c[3] > 0]
    transparent = 1 - len(occupied) / total
    box = image.getbbox()
    bbox_ratio = ((box[2] - box[0]) * (box[3] - box[1]) / total) if box else 1.0
    residue = sum(1 for i in occupied
                  if chroma_score(pixels[i][:3]) >= _CHROMA_EDGE_MIN) / max(1, len(occupied))
    edges = edge_indexes(pixels, width, height)
    spill_ratio = sum(1 for i in edges if spill(pixels[i])) / max(1, len(edges))
    small_count, biggest, comp_count = small_components(image)
    size_bytes, quantized, fill = frame_bytes(image)

    white_keep = None
    if source is not None and source.size == (width, height):
        src = source.convert("RGB").getdata()
        whites = [i for i, c in enumerate(src)
                  if min(c) >= 225 and max(c) - min(c) <= 22]
        kept = sum(1 for i in whites if pixels[i][3] > 127)
        white_keep = kept / max(1, len(whites))

    checks = [
        ("透明占比 >= 30%", transparent >= _MIN_TRANSPARENT_RATIO,
         "%.2f%%" % (100 * transparent), ">= 30%"),
        ("alpha bbox <= 95%", bbox_ratio <= _MAX_BBOX_RATIO,
         "%.2f%%" % (100 * bbox_ratio), "<= 95%"),
        ("青蓝残留 <= 0.5%", residue <= _MAX_CHROMA_RESIDUE,
         "%.3f%%" % (100 * residue), "<= 0.5%"),
        ("轮廓溢色 <= 1%", spill_ratio <= _MAX_EDGE_SPILL,
         "%.2f%%" % (100 * spill_ratio), "<= 1%"),
        ("孤立噪点 = 0", small_count <= _MAX_SMALL_COMPS,
         "%d 块（共 %d 块，最大 %d px）" % (small_count, comp_count, biggest), "= 0"),
        ("白色衣物保留 >= 99%", None if white_keep is None else white_keep >= _MIN_WHITE_KEEP,
         "N/A（未提供同尺寸 --source）" if white_keep is None else "%.2f%%" % (100 * white_keep),
         ">= 99%"),
        ("1000 帧 <= 8 MB", size_bytes <= _MAX_FRAME_BYTES,
         "%.2f MB（单帧 %d B）" % (size_bytes * 1000 / 1024 / 1024, size_bytes),
         "<= 8.00 MB"),
    ]
    metrics = {
        "size": (width, height), "bbox": box, "transparent": transparent,
        "residue": residue, "edge_spill": spill_ratio, "edge_pixels": len(edges),
        "small_components": small_count, "white_keep": white_keep,
        "frame_bytes": size_bytes, "frame_fill": fill,
    }
    return checks, metrics, quantized


def checker(size, cell=12):
    """棋盘格底，用来暴露半透明毛边与残留色块。"""
    result = Image.new("RGBA", size, (235, 235, 238, 255))
    draw = ImageDraw.Draw(result)
    for y in range(0, size[1], cell):
        for x in range(0, size[0], cell):
            if (x // cell + y // cell) % 2:
                draw.rectangle([x, y, x + cell - 1, y + cell - 1],
                               fill=(200, 200, 205, 255))
    return result


def build_panels(image, quantized, out_dir, tag):
    """输出浅 / 深 / 棋盘 / 小窗四联对照与 105x297 WebP。"""
    width, height = image.size
    bases = {
        "white": Image.new("RGBA", (width, height), (250, 250, 250, 255)),
        "dark": Image.new("RGBA", (width, height), (28, 32, 44, 255)),
        "checker": checker((width, height)),
    }
    for name, base in bases.items():
        panel = base.copy()
        panel.alpha_composite(image)
        panel.convert("RGB").save(out_dir / ("asset-%s-%s.png" % (tag, name)))
    small = checker((210, 594), 10)
    box = image.getbbox()
    crop = image.crop(box) if box else image
    fit = crop.copy()
    fit.thumbnail((190, 560), Image.LANCZOS)
    small.alpha_composite(fit, ((210 - fit.width) // 2,
                                (594 - fit.height) // 2))
    small.convert("RGB").save(out_dir / ("asset-%s-small.png" % tag))
    webp = out_dir / ("asset-%s-105x297.webp" % tag)
    quantized.save(webp, format="WEBP", lossless=True, method=4)
    return webp


def write_trace(records, out_dir, source_name):
    """把本轮验收结果写成可追溯 Markdown 留痕。"""
    _TRACES.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = _TRACES / ("精灵抠图产物验收-%s.md" % stamp)
    lines = [
        "# 精灵抠图产物验收（SCR-017）",
        "",
        "- 执行时间：%s" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 源图（白色衣物对照）：%s" % source_name,
        "- 输出目录：`%s`（**不进 `src/assets/pet`**）" % out_dir,
        "",
    ]
    for record in records:
        lines += ["## %s" % record["tag"], "",
                  "- 输入：`%s`" % record["path"],
                  "- 尺寸 / bbox：%s / %s" % (record["size"], record["bbox"]),
                  "- 透明占比 %.2f%%，边缘像素 %d，小窗高度利用率 %.1f%%" % (
                      100 * record["transparent"], record["edge_pixels"],
                      100 * record["frame_fill"]),
                  "- 修补：%s" % record["fix_note"], "",
                  "| 判定 | 结果 | 实测 | 阈值 |", "| --- | --- | --- | --- |"]
        for name, passed, actual, limit in record["checks"]:
            mark = "跳过" if passed is None else ("通过" if passed else "**不通过**")
            lines.append("| %s | %s | %s | %s |" % (name, mark, actual, limit))
        lines.append("")
    lines += [
        "## 结论",
        "",
        "- 退出码按 %s 判定。" % (
            "修补版（`--fix`）" if any(r["is_fix"] for r in records) else "全部输入"),
        "- 本脚本只做验收与修补，**不替换现行 21 张进包素材**；1000 帧真动作仍受",
        "  RIFE/FILM 补间权重、BiRefNet 抠图权重与模型分发许可阻断。",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description="SCR-017 已抠透明 PNG 进包门禁")
    parser.add_argument("--image", action="append", required=True,
                        help="已抠好的透明 PNG，可重复")
    parser.add_argument("--source", default=None,
                        help="同尺寸原始带底图，用于白色衣物保留判定")
    parser.add_argument("--fix", action="store_true",
                        help="输出去溢色修补版并以修补版判退出码")
    parser.add_argument("--out", default="/tmp/sugt-cutout-asset",
                        help="对照图与修补产物输出目录")
    parser.add_argument("--root", default=None, help="仓库根目录（留痕路径）")
    args = parser.parse_args()

    global _ROOT, _TRACES
    if args.root:
        _ROOT = Path(args.root).resolve()
        _TRACES = _ROOT / "docs" / "孙测测" / "traces"

    source = None
    if args.source:
        source_path = Path(args.source)
        if not source_path.is_file():
            print("源图不存在：%s" % source_path)
            return 2
        source = Image.open(source_path)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    records = []
    failed = 0

    for raw in args.image:
        path = Path(raw)
        if not path.is_file():
            print("输入不存在：%s" % path)
            return 2
        original = Image.open(path).convert("RGBA")
        tag = path.stem.replace(" ", "-")
        variants = [("原版", original, False, "未修补")]
        if args.fix:
            fixed, replaced, remain = fix_image(original)
            fixed_path = out_dir / ("asset-%s-fixed.png" % tag)
            fixed.save(fixed_path)
            variants.append(("修补版", fixed, True,
                             "替换溢色边 %d px，剩余 %d px；透明区 RGB 清零" % (
                                 replaced, remain)))
        for label, image, is_fix, note in variants:
            checks, metrics, quantized = measure(image, source)
            panel_tag = "%s-%s" % (tag, "fixed" if is_fix else "raw")
            webp = build_panels(image, quantized, out_dir, panel_tag)
            passed = [c[1] for c in checks if c[1] is not None]
            ok = all(passed)
            print("%s / %s：%d/%d 通过，溢色 %.2f%%，单帧 %d B，产物 %s" % (
                path.name, label, sum(1 for p in passed if p), len(passed),
                100 * metrics["edge_spill"], metrics["frame_bytes"], webp.name))
            for name, result, actual, _ in checks:
                if result is False:
                    print("  [不通过] %s：%s" % (name, actual))
            # 有 --fix 时原版只作参照，不参与退出码。
            if not args.fix or is_fix:
                failed += 0 if ok else 1
            records.append({
                "tag": "%s / %s" % (path.name, label), "path": str(path),
                "size": metrics["size"], "bbox": metrics["bbox"],
                "transparent": metrics["transparent"],
                "edge_pixels": metrics["edge_pixels"],
                "frame_fill": metrics["frame_fill"], "fix_note": note,
                "checks": checks, "is_fix": is_fix,
            })

    trace = write_trace(records, out_dir, args.source or "N/A")
    print("留痕：%s" % trace)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
