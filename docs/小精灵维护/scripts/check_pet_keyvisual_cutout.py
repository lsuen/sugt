#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-016 桌面精灵定稿图「可进包抠图」验收门禁。

编写人员：孙测测
作用用途：
  修正 SCR-015 只有「四角 RGB 极差」判据带来的假绿：背景均匀并不代表
  能干净抠图。本脚本对定稿图真正执行抠图并做五项可判定验收——
    1. 四角背景均匀；2. alpha 包围盒未退化成整图；
    3. 抠图后仍残留背景色像素的比例（抓封闭白块 / 地面阴影）；
    4. 角色内部白色衣物（T 恤 / 袜 / 鞋 / 挂件）保留率；
    5. 背景确实被清掉（透明占比）。
  浅色纯底走四边泛洪；洋红纯色底直接复用 ``pet_v07_frames.key_chroma``
  与 ``clean_specks``，即 v0.7 已验证的落包色键链路；青蓝纯色底走**全局
  色度优势判据** ``min(G-R, B-G)``，不做连通性泛洪——大缕状发丝之间的
  空隙是封闭背景岛，泛洪进不去会留下大片蓝色块（2026-11-10 实测）。
  产物只落输出目录与 traces，**绝不写入 src/assets/pet**。
适用项目：SUGT 桌面精灵 v1.2 素材链路（本地执行，不依赖 ComfyUI / 网络）
创建日期：2026-11-10
复用方式：
  python3 check_pet_keyvisual_cutout.py --image <定稿.png> [--image ...]
      [--mode auto|light|magenta] [--out /tmp/sugt-cutout] [--root .]
  需 Pillow（本地素材链路脚本，不进 CI）。auto 按四角中位色判底色。
退出码约定：0 全部通过 / 1 存在验收不通过 / 2 参数、依赖或解析异常
"""
import argparse
import importlib.util
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

# 浅色底泛洪判据：v2 实测值。放宽到能吞掉 (184,191,208) 的地面阴影，
# 但仍不吃炭灰夹克 / 深炭灰工装裤。B-R 条件排除暖白肤色。
_LIGHT_MIN = 168
_LIGHT_SPREAD = 44
_LIGHT_BLUE_BIAS = -6
# 验收阈值
_MAX_CORNER_SPREAD = 24
_MAX_BBOX_RATIO = 0.95
_MAX_RESIDUE_RATIO = 0.03
_MIN_WHITE_KEEP = 0.99
_MIN_TRANSPARENT_RATIO = 0.30
# 青蓝底全局色度：角色（黑/灰/紫红/薄荷绿）min(G-R,B-G)<=0。
# 背景 score 会随 Edit 2509 逐张漂移（实测 47 / 16 / 45），固定阈值会让
# 低 score 的青底抠不干净，故按「本图四角背景 score」自适应缩放。
_CHROMA_FLOOR_RATIO = 0.18
_CHROMA_FULL_RATIO = 0.75
_CHROMA_MIN_FLOOR = 3.0
_CHROMA_MIN_GAP = 6.0
_CHROMA_NEAR = 95.0


def chroma_thresholds(background):
    """按背景色度优势自适应色键阈值；参考底 (91,145,190) 得 (8.1,33.8)。"""
    score = min(background[1] - background[0],
                background[2] - background[1])
    floor = max(_CHROMA_MIN_FLOOR, score * _CHROMA_FLOOR_RATIO)
    full = max(floor + _CHROMA_MIN_GAP, score * _CHROMA_FULL_RATIO)
    return floor, full


def load_key_module():
    """动态载入 v0.7 色键实现，避免复制粘贴第二套抠图逻辑。"""
    path = _HERE / "pet_v07_frames.py"
    spec = importlib.util.spec_from_file_location("sugt_pet_v07_frames", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def corner_stats(image):
    """四角采样，返回 (中位背景色, 各通道极差, 四个角颜色)。"""
    width, height = image.size
    points = ((4, 4), (width - 5, 4), (4, height - 5), (width - 5, height - 5))
    corners = [image.getpixel(point) for point in points]
    middle = tuple(sorted(c[channel] for c in corners)[2] for channel in range(3))
    spans = [max(c[channel] for c in corners) - min(c[channel] for c in corners)
             for channel in range(3)]
    return middle, spans, corners


def detect_mode(corners):
    """按四角色度特征区分青蓝底 / 洋红底 / 浅色底。

    不能只看中位色：Qwen 常把下半画成白地面，四角中位会被拉成
    (253,234,239) 这种「假浅色底」，把洋红底误判成 light 模式。
    """
    # Edit 2509 会把纯青蓝底漂成 (82,171,187)：B-G 只剩 16，
    # 旧判据 blue-green>=20 会漏判成 light 泛洪（实测 bbox 100%、透明 0.3%）。
    # 改为「两通道同时为正 + 明显蓝偏」，覆盖 score 16~47 的漂移区间。
    for red, green, blue in corners:
        if min(green - red, blue - green) >= 8 and blue - red >= 40:
            return "chroma"
    for red, green, blue in corners:
        if red > 200 and green < 130 and blue > 100:
            return "magenta"
    return "light"


def light_key(image):
    """四边泛洪抠浅色底；返回 RGBA 图、背景色、被判背景的像素数。"""
    width, height = image.size
    pixels = list(image.convert("RGB").getdata())
    total = width * height

    def is_background(color):
        red, green, blue = color
        return (min(color) >= _LIGHT_MIN
                and max(color) - min(color) <= _LIGHT_SPREAD
                and blue >= red + _LIGHT_BLUE_BIAS)

    external = bytearray(total)
    queue = deque()
    for x in range(width):
        for y in (0, height - 1):
            index = y * width + x
            if not external[index] and is_background(pixels[index]):
                external[index] = 1
                queue.append(index)
    for y in range(height):
        for x in (0, width - 1):
            index = y * width + x
            if not external[index] and is_background(pixels[index]):
                external[index] = 1
                queue.append(index)
    while queue:
        index = queue.popleft()
        x, y = index % width, index // width
        neighbours = (index - 1 if x > 0 else -1,
                      index + 1 if x < width - 1 else -1,
                      index - width if y > 0 else -1,
                      index + width if y < height - 1 else -1)
        for neighbour in neighbours:
            if neighbour >= 0 and not external[neighbour] \
                    and is_background(pixels[neighbour]):
                external[neighbour] = 1
                queue.append(neighbour)

    alpha = bytearray(0 if external[i] else 255 for i in range(total))
    rgba = Image.new("RGBA", (width, height))
    rgba.putdata([(c[0], c[1], c[2], alpha[i])
                  for i, c in enumerate(pixels)])
    return rgba, external


def magenta_key(image, module):
    """复用 v0.7 色度空间软色键 + 连通域清噪，返回 RGBA 图与背景色。"""
    width, height = image.size
    raw = image.convert("RGB").tobytes()
    rgba_bytes, background = module.key_chroma(raw, width, height)
    cleaned, removed = module.clean_specks(rgba_bytes, width, height)
    rgba = Image.frombytes("RGBA", (width, height), cleaned)
    return rgba, background, removed


def chroma_key(image, background):
    """青蓝纯色底全局色度抠图：不做连通性泛洪，避免封闭背景岛残留。

    背景青蓝同时满足 G-R 与 B-G 为正；角色的黑发 / 灰卫衣 / 紫红发尾 /
    白鞋 / 薄荷绿配饰至少有一个为负，故可全局判定而不误伤。软 alpha 用
    反预乘去溢色消除蓝边。
    """
    width, height = image.size
    pixels = list(image.convert("RGB").getdata())
    rgba = []
    for color in pixels:
        red, green, blue = color
        score = min(green - red, blue - green)
        distance = sum((color[c] - background[c]) ** 2
                       for c in range(3)) ** 0.5
        floor, full = chroma_thresholds(background)
        strength = max(0.0, min(1.0, (score - floor) / (full - floor)))
        if distance > _CHROMA_NEAR:
            strength = min(strength, 0.30)
        alpha = 1.0 - strength
        alpha = 0.0 if alpha <= 0.08 else (alpha - 0.08) / 0.92
        if alpha >= 0.999:
            rgba.append((red, green, blue, 255))
        elif alpha <= 0.0:
            rgba.append((0, 0, 0, 0))
        else:
            effective = max(alpha, 0.45)
            inverse = 1.0 - alpha
            rgba.append(tuple(
                min(255, max(0, int((color[c] - inverse * background[c])
                                    / effective))) for c in range(3)
            ) + (int(alpha * 255 + 0.5),))
    result = Image.new("RGBA", (width, height))
    result.putdata(rgba)
    return result


def measure(image, rgba, mode, background, external=None):
    """执行五项验收，返回判定列表与指标字典。"""
    width, height = image.size
    total = width * height
    source = list(image.convert("RGB").getdata())
    alpha = list(rgba.getdata())

    occupied = [i for i, pixel in enumerate(alpha) if pixel[3] > 0]
    xs = [i % width for i in occupied]
    ys = [i // width for i in occupied]
    if occupied:
        box = (min(xs), min(ys), max(xs), max(ys))
        bbox_ratio = ((box[2] - box[0] + 1) * (box[3] - box[1] + 1)) / total
    else:
        box = (0, 0, 0, 0)
        bbox_ratio = 0.0
    transparent_ratio = 1.0 - len(occupied) / total

    def residual(color):
        """浅底只认偏蓝灰的背景色，避免把中性白 T 恤算成残留。"""
        red, green, blue = color
        distance = sum((color[c] - background[c]) ** 2 for c in range(3)) ** 0.5
        if mode == "magenta":
            return distance <= 40
        if mode == "chroma":
            background_score = min(background[1] - background[0],
                                   background[2] - background[1])
            return (distance <= 30
                    and min(green - red, blue - green)
                    >= max(6, background_score * 0.45))
        # 背景是偏蓝灰 (B-R >= 3)，白 T / 袜 / 鞋是中性白 (B-R <= 2)。
        return distance <= 30 and blue - red >= 3 and min(color) >= 205

    leftovers = [i for i in occupied if residual(source[i])]
    residue_ratio = len(leftovers) / max(1, len(occupied))

    # 白色衣物只统计「内部中性亮白」：排除与四边连通的浅灰白背景。
    white = [i for i, color in enumerate(source)
             if min(color) >= 238 and max(color) - min(color) <= 10
             and abs(color[2] - color[0]) <= 2
             and (external is None or not external[i])]
    white_inside = [i for i in white if alpha[i][3] > 0]
    white_keep = len(white_inside) / max(1, len(white))

    _, spans, corners = corner_stats(image)
    checks = [
        ("四角背景均匀", max(spans) <= _MAX_CORNER_SPREAD,
         "RGB 极差 %d/%d/%d（阈值 <=%d）" % (
             spans[0], spans[1], spans[2], _MAX_CORNER_SPREAD)),
        ("alpha 包围盒未退化为整图", bbox_ratio <= _MAX_BBOX_RATIO,
         "%s，占全图 %.1f%%（阈值 <=%.0f%%）" % (
             str(box), bbox_ratio * 100, _MAX_BBOX_RATIO * 100)),
        ("无封闭背景块 / 地面阴影残留", residue_ratio <= _MAX_RESIDUE_RATIO,
         "残留 %d 像素 / 主体 %.2f%%（阈值 <=%.0f%%）" % (
             len(leftovers), residue_ratio * 100, _MAX_RESIDUE_RATIO * 100)),
        ("白色衣物未被误抠", white_keep >= _MIN_WHITE_KEEP,
         "白 T / 袜 / 鞋 / 挂件保留 %.2f%%（阈值 >=%.0f%%）" % (
             white_keep * 100, _MIN_WHITE_KEEP * 100)),
        ("背景已清除", transparent_ratio >= _MIN_TRANSPARENT_RATIO,
         "透明占比 %.1f%%（阈值 >=%.0f%%）" % (
             transparent_ratio * 100, _MIN_TRANSPARENT_RATIO * 100)),
    ]
    metrics = {
        "corners": corners, "spans": spans, "box": box,
        "bbox_ratio": bbox_ratio, "residue_ratio": residue_ratio,
        "white_keep": white_keep, "transparent_ratio": transparent_ratio,
        "leftovers": len(leftovers),
    }
    return checks, metrics


def checkerboard(size, cell=16):
    """生成浅灰棋盘底，用于目视检查透明区与白边。"""
    canvas = Image.new("RGB", size, (235, 235, 238))
    draw = ImageDraw.Draw(canvas)
    for y in range(0, size[1], cell):
        for x in range(0, size[0], cell):
            if (x // cell + y // cell) % 2:
                draw.rectangle((x, y, x + cell - 1, y + cell - 1),
                               fill=(200, 200, 205))
    return canvas


def build_compare(rgba, box, out_path):
    """输出浅底 / 深底 / 棋盘 / 105×297 小窗四联对照，供人工复核。"""
    width, height = rgba.size
    crop = rgba.crop((box[0], box[1], box[2] + 1, box[3] + 1))
    panels = []
    for base in (Image.new("RGBA", (width, height), (245, 245, 247, 255)),
                 Image.new("RGBA", (width, height), (28, 32, 44, 255)),
                 checkerboard((width, height)).convert("RGBA")):
        panel = base.copy()
        panel.alpha_composite(rgba)
        panels.append(panel.convert("RGB"))
    small = checkerboard((210, 594), 10).convert("RGBA")
    fitted = crop.copy()
    fitted.thumbnail((190, 560), Image.LANCZOS)
    small.alpha_composite(fitted, ((210 - fitted.width) // 2,
                                   (594 - fitted.height) // 2))
    canvas = Image.new("RGB", (width * 3 + 40 + 210, height), (255, 255, 255))
    for index, panel in enumerate(panels):
        canvas.paste(panel, (index * (width + 10), 0))
    canvas.paste(small.convert("RGB"), (width * 3 + 30, 0))
    canvas.save(out_path)


def write_trace(path, results, started):
    """把逐图判定落盘留痕，失败项单独列出，便于开发复现。"""
    lines = [
        "# 精灵定稿抠图验收（SCR-016）",
        "",
        "- 执行时间：%s" % started,
        "- 脚本：`docs/孙测测/scripts/check_pet_keyvisual_cutout.py`",
        "- 判据：四角均匀 / bbox 未退化 / 背景残留 / 白色衣物保留 / 背景清除",
        "- 口径：浅底四边泛洪；洋红底复用 `pet_v07_frames.key_chroma`",
        "",
    ]
    for item in results:
        lines += ["## `%s`（模式 %s）" % (item["name"], item["mode"]), ""]
        for title, passed, detail in item["checks"]:
            lines.append("- %s **%s**：%s" % (
                "通过" if passed else "不通过", title, detail))
        lines += ["", "产物：`%s`、`%s`" % (
            item["cutout"], item["compare"]), ""]
    total = sum(len(item["checks"]) for item in results)
    failed = sum(1 for item in results
                 for _title, passed, _detail in item["checks"] if not passed)
    lines += ["## 汇总", "", "%d/%d 项通过，%d 项不通过，退出码 %d。" % (
        total - failed, total, failed, 1 if failed else 0), ""]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")
    return total - failed, total


def main():
    parser = argparse.ArgumentParser(description="精灵定稿图可进包抠图验收")
    parser.add_argument("--image", action="append", required=True,
                        help="待验收定稿图，可重复传入")
    parser.add_argument("--mode", choices=("auto", "light", "magenta", "chroma"),
                        default="auto", help="底色模式，默认自动判定")
    parser.add_argument("--out", default="/tmp/sugt-pet-cutout-check",
                        help="产物目录，禁止指向 src/assets/pet")
    parser.add_argument("--root", default=str(_ROOT), help="项目根目录")
    args = parser.parse_args()

    out_dir = Path(args.out).resolve()
    protected = (Path(args.root).resolve() / "src" / "assets" / "pet")
    if out_dir == protected or protected in out_dir.parents:
        print("输出目录不得位于 src/assets/pet")
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)
    key_module = load_key_module()

    started = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    results = []
    for raw_path in args.image:
        path = Path(raw_path).resolve()
        if not path.is_file():
            print("源文件不存在：%s" % path)
            return 2
        image = Image.open(path).convert("RGB")
        background, spans, corners = corner_stats(image)
        mode = detect_mode(corners) if args.mode == "auto" else args.mode
        if mode == "magenta":
            rgba, background, removed = magenta_key(image, key_module)
            external = None
            print("%s | 洋红色键，清噪 %d 块" % (path.name, removed))
        elif mode == "chroma":
            rgba = chroma_key(image, background)
            external = None
            print("%s | 青蓝底全局色度色键" % path.name)
        else:
            rgba, external = light_key(image)
            print("%s | 浅色底泛洪" % path.name)

        checks, metrics = measure(image, rgba, mode, background, external)
        cutout = out_dir / ("cutout-%s.png" % path.stem)
        compare = out_dir / ("cutout-%s-compare.png" % path.stem)
        rgba.save(cutout)
        build_compare(rgba, metrics["box"], compare)
        for title, passed, detail in checks:
            print("  [%s] %s — %s" % (
                "通过" if passed else "不通过", title, detail))
        results.append({
            "name": path.name, "mode": mode, "checks": checks,
            "cutout": str(cutout), "compare": str(compare),
        })

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    trace = _TRACES / ("精灵定稿抠图验收-%s.md" % stamp)
    passed, total = write_trace(trace, results, started)
    print("汇总：%d/%d 通过，留痕 %s" % (passed, total, trace))
    return 0 if passed == total else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as exc:
        print("解析/运行异常：%r" % exc)
        sys.exit(2)
