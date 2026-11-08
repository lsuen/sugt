#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵素材落包：纯白底状态帧 → 透明 @2x WebP。

编写人员：孙测测
作用用途：把本地 ComfyUI 产出的状态帧抠成透明底、缩放到 @2x 并编码为 WebP；
          不用单纯颜色键，避免米白外套、白袜白鞋被误抠。
          v2 抠图修复发丝缝隙：纯白严格判据 + 四边泛洪 + 上半身宽松扩散
          + 边缘去白，下半身不做宽松清除，保护与背景同色的白鞋与白袜。
          最后按人物 alpha 包围盒做几何对齐，消除各帧构图漂移。
适用项目：SUGT 桌面精灵素材链路
创建日期：2026-11-08（v2 抠图修订同日）
复用方式：python3 pet_frames_to_webp.py [--src 生成目录] [--out 资产目录] [--size 360x560]
          依赖系统命令 ffmpeg / cwebp；不需要 Pillow、numpy 等第三方库
退出码约定：0 全部成功 / 1 源文件或外部命令失败 / 2 参数或解析异常
"""
import argparse
import datetime
import os
import struct
import subprocess
import sys
import zlib
from collections import deque

# 状态帧文件名前缀 → 进包素材名；顺序即状态机使用的帧集合
STATES = ("idle", "typing", "done", "error", "sleep")


def run(cmd, **kwargs):
    """执行外部命令并返回 stdout 二进制。"""
    return subprocess.run(cmd, check=True, stdout=subprocess.PIPE, **kwargs).stdout


def decode_rgb(png_path, width, height):
    """用 ffmpeg 一次性完成缩放与 RGB24 解码，输出紧凑字节流。"""
    return run([
        "ffmpeg", "-v", "error", "-y", "-i", png_path,
        "-vf", "scale=%d:%d:flags=lanczos" % (width, height),
        "-pix_fmt", "rgb24", "-f", "rawvideo", "-",
    ])


def is_background(r, g, b):
    """只认「纯白 / 极近白」背景，不认米白外套。

    实测 v2 生成帧（缩放后取色）：
      背景、白袜、白鞋 = (255,255,255)
      米白外套亮部 = (247,243,240) / (250,250,250)
      米白外套阴影 = (219,214,215)
    v1 的 low>=224 会把外套整片当成背景，宽松扩散后外套被打成噪点破洞；
    收紧到 low>=252 后外套完全不参与背景判定，白鞋白袜靠下半身保护。
    """
    low = min(r, g, b)
    spread = max(r, g, b) - low
    return low >= 252 and spread <= 4


def neighbours(index, width, height):
    """四邻域，避免斜向泄漏到角色内部。"""
    x, y = index % width, index // width
    if x > 0:
        yield index - 1
    if x + 1 < width:
        yield index + 1
    if y > 0:
        yield index - width
    if y + 1 < height:
        yield index + width


def cutout(rgb, width, height):
    """纯白底抠图：四边泛洪 + 上半身宽松扩散 + 小封闭白块清除。

    v1 只做四边泛洪，发丝之间被 1~2px 线稿隔开的白区不与画面边缘连通，
    会残留成白色碎块（用户实测反馈）。v2 分三步处理：
      1. 四边泛洪：清掉与外部连通的背景；
      2. 宽松扩散：允许跨过 3px 非白像素再命中白像素，清掉发丝缝隙；
         只在画面上半 55% 生效，避免误伤下方的白袜与白鞋；
    米白外套（240~250）不满足纯白判据，白鞋白袜在下半身不受扩散影响，
    眼睛高光距外部背景很远，三者都不会被打成透明洞。
    """
    total = width * height
    like = bytearray(total)  # 1 = 近白中性色，可能是背景
    for index in range(total):
        offset = index * 3
        if is_background(rgb[offset], rgb[offset + 1], rgb[offset + 2]):
            like[index] = 1

    transparent = bytearray(total)
    queue = deque()
    for x in range(width):
        for index in (x, (height - 1) * width + x):
            if like[index] and not transparent[index]:
                transparent[index] = 1
                queue.append(index)
    for y in range(height):
        for index in (y * width, y * width + width - 1):
            if like[index] and not transparent[index]:
                transparent[index] = 1
                queue.append(index)
    while queue:
        for neighbour in neighbours(queue.popleft(), width, height):
            if like[neighbour] and not transparent[neighbour]:
                transparent[neighbour] = 1
                queue.append(neighbour)

    # 上半身宽松扩散：把「距外部背景 3 像素以内」的白像素也判成背景，
    # 这样被细线稿/发丝隔开的白缝隙能被清掉，而角色深处的白色高光不受影响。
    upper = int(height * 0.55)
    reach = bytearray(transparent)
    frontier = [index for index in range(total) if transparent[index]]
    for _ in range(3):
        next_frontier = []
        for index in frontier:
            for neighbour in neighbours(index, width, height):
                if not reach[neighbour]:
                    reach[neighbour] = 1
                    next_frontier.append(neighbour)
        frontier = next_frontier
        if not frontier:
            break
    for index in range(upper * width):
        if like[index] and reach[index] and not transparent[index]:
            transparent[index] = 1

    # 注意：不再做「小封闭白块清除」。外套亮部与白袜同样是纯白，
    # 按面积清除会把角色内部打成噪点破洞（v2 首轮实测缺陷）。
    # 发丝缝隙靠上面的宽松扩散已能覆盖；角色深处的白高光保持不透明。

    # 上半身边缘去白：缩图插值会在发梢轮廓留一圈浅色抗锯齿边，
    # 把「紧邻透明区且偏白」的不透明像素再清掉 1~2 层。
    # 下半身不处理，否则与背景同色的白鞋白袜会被一层层吃掉。
    for _ in range(2):
        edge = []
        for index in range(upper * width):
            if transparent[index]:
                continue
            offset = index * 3
            red, green, blue = rgb[offset], rgb[offset + 1], rgb[offset + 2]
            if min(red, green, blue) < 224 or max(red, green, blue) - min(red, green, blue) > 28:
                continue
            if any(transparent[j] for j in neighbours(index, width, height)):
                edge.append(index)
        if not edge:
            break
        for index in edge:
            transparent[index] = 1

    rgba = bytearray(total * 4)
    count = 0
    for index in range(total):
        source = index * 3
        target = index * 4
        rgba[target] = rgb[source]
        rgba[target + 1] = rgb[source + 1]
        rgba[target + 2] = rgb[source + 2]
        if transparent[index]:
            rgba[target + 3] = 0
            count += 1
        else:
            rgba[target + 3] = 255
    return bytes(rgba), count / total


def alpha_bbox(rgba, width, height):
    """非透明像素的包围盒；用于检测各帧人物构图是否漂移。"""
    min_x, min_y, max_x, max_y = width, height, -1, -1
    for y in range(height):
        row = y * width
        for x in range(width):
            if rgba[(row + x) * 4 + 3] > 8:
                if x < min_x:
                    min_x = x
                if x > max_x:
                    max_x = x
                if y < min_y:
                    min_y = y
                if y > max_y:
                    max_y = y
    if max_x < 0:
        return (0, 0, width, height)
    return (min_x, min_y, max_x + 1, max_y + 1)


def median_box(boxes):
    """取各坐标分量的中位数作基准框，单帧异常不会带偏全部素材。"""
    count = len(boxes)
    return tuple(sorted(box[i] for box in boxes)[count // 2] for i in range(4))


def warp_to_box(rgba, width, height, src, dst):
    """把人物包围盒双线性重采样到统一基准框，消除状态切换时的忽大忽小。

    先预乘 alpha 再插值，避免透明像素残留的白色 RGB 渗进边缘形成白边；
    输出前再反预乘还原颜色。Qwen-Edit 各帧构图可能相差 10~20%，
    靠提示词约束不可靠，因此统一在落包阶段做几何对齐。
    """
    sx0, sy0, sx1, sy1 = src
    dx0, dy0, dx1, dy1 = dst
    src_w, src_h = sx1 - sx0, sy1 - sy0
    dst_w, dst_h = dx1 - dx0, dy1 - dy0
    premul = bytearray(len(rgba))
    for i in range(0, len(rgba), 4):
        alpha = rgba[i + 3]
        premul[i] = rgba[i] * alpha // 255
        premul[i + 1] = rgba[i + 1] * alpha // 255
        premul[i + 2] = rgba[i + 2] * alpha // 255
        premul[i + 3] = alpha
    out = bytearray(width * height * 4)
    for dy in range(dst_h):
        fy = (dy + 0.5) * src_h / dst_h - 0.5 + sy0
        y0 = int(fy // 1)
        wy = fy - y0
        y0 = max(0, min(height - 1, y0))
        y1 = max(0, min(height - 1, y0 + 1))
        w10 = 1 - wy
        for dx in range(dst_w):
            fx = (dx + 0.5) * src_w / dst_w - 0.5 + sx0
            x0 = int(fx // 1)
            wx = fx - x0
            x0 = max(0, min(width - 1, x0))
            x1 = max(0, min(width - 1, x0 + 1))
            i00 = (y0 * width + x0) * 4
            i01 = (y0 * width + x1) * 4
            i10 = (y1 * width + x0) * 4
            i11 = (y1 * width + x1) * 4
            di = ((dy0 + dy) * width + dx0 + dx) * 4
            w00 = (1 - wx) * w10
            w01 = wx * w10
            w_a = (1 - wx) * wy
            w_b = wx * wy
            for channel in range(4):
                out[di + channel] = int(
                    premul[i00 + channel] * w00
                    + premul[i01 + channel] * w01
                    + premul[i10 + channel] * w_a
                    + premul[i11 + channel] * w_b
                )
    for i in range(0, len(out), 4):
        alpha = out[i + 3]
        if alpha > 0:
            for channel in range(3):
                out[i + channel] = min(255, out[i + channel] * 255 // alpha)
    return bytes(out)


def transparent_ratio(rgba):
    total = len(rgba) // 4
    return sum(1 for i in range(3, len(rgba), 4) if rgba[i] == 0) / total


def png_chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def write_rgba_png(path, width, height, rgba):
    """标准库手写 RGBA PNG，避免引入图像库依赖。"""
    scanlines = b"".join(
        b"\x00" + rgba[y * width * 4:(y + 1) * width * 4] for y in range(height)
    )
    payload = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(scanlines, 9))
        + png_chunk(b"IEND", b"")
    )
    with open(path, "wb") as handle:
        handle.write(payload)


def find_source(src_dir, state):
    """在生成目录里找到 pet-<state>_*.png，取文件名排序后的第一个。"""
    hits = sorted(
        name for name in os.listdir(src_dir)
        if name.startswith("pet-%s_" % state) and name.lower().endswith(".png")
    )
    return os.path.join(src_dir, hits[0]) if hits else None


def main():
    parser = argparse.ArgumentParser(description="桌面精灵状态帧转透明 WebP")
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.abspath(os.path.join(here, "..", "..", ".."))
    parser.add_argument("--src", default=os.path.join(root, "docs", "design", "assets", "generated"))
    parser.add_argument("--out", default=os.path.join(root, "src", "assets", "pet"))
    parser.add_argument("--trace", default=os.path.join(root, "docs", "孙测测", "traces"))
    parser.add_argument("--size", default="360x560", help="@2x 目标尺寸，格式 宽x高")
    args = parser.parse_args()

    try:
        width, height = (int(part) for part in args.size.lower().split("x"))
    except Exception:
        print("尺寸格式应为 宽x高，例如 360x560")
        return 2
    if not os.path.isdir(args.src):
        print("源目录不存在:", args.src)
        return 1
    os.makedirs(args.out, exist_ok=True)
    os.makedirs(args.trace, exist_ok=True)

    # 第一阶段：全部帧解码 + 抠图 + 量出人物包围盒
    frames = []
    for state in STATES:
        source = find_source(args.src, state)
        if not source:
            print("缺少状态帧:", state)
            return 1
        rgb = decode_rgb(source, width, height)
        if len(rgb) != width * height * 3:
            print("解码尺寸不符:", state, len(rgb))
            return 1
        rgba, _ratio = cutout(rgb, width, height)
        box = alpha_bbox(rgba, width, height)
        frames.append((state, source, rgba, box))
        print("抠图 %-8s bbox=%s" % (state, box), flush=True)

    # 第二阶段：几何对齐到统一基准框后再编码，保证状态切换不忽大忽小
    base = median_box([frame[3] for frame in frames])
    print("统一基准框:", base, flush=True)

    rows = []
    total_bytes = 0
    for state, source, rgba, box in frames:
        aligned = warp_to_box(rgba, width, height, box, base)
        ratio = transparent_ratio(aligned)
        tmp_png = os.path.join(args.out, ".su-%s-tmp.png" % state)
        target = os.path.join(args.out, "su-%s.webp" % state)
        write_rgba_png(tmp_png, width, height, aligned)
        run([
            "cwebp", "-quiet", "-q", "82", "-m", "6",
            "-alpha_q", "92", "-alpha_method", "1", tmp_png, "-o", target,
        ])
        os.remove(tmp_png)
        size_kb = os.path.getsize(target) / 1024
        total_bytes += os.path.getsize(target)
        if ratio < 0.15:
            print("警告：%s 透明区域仅 %.1f%%，抠底可能失败" % (state, ratio * 100))
        rows.append((state, os.path.basename(source), "%s" % (box,),
                     "%.1f%%" % (ratio * 100), "%.2f KB" % size_kb))
        print("OK  %-8s %-28s 透明 %5.1f%%  %8.2f KB" % (
            state, os.path.basename(source), ratio * 100, size_kb))

    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    trace_path = os.path.join(args.trace, "桌面精灵素材落包-%s.md" % stamp)
    lines = [
        "# 桌面精灵素材落包留痕",
        "",
        "- 执行时间：%s" % datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 脚本：`docs/孙测测/scripts/pet_frames_to_webp.py`",
        "- 来源目录：`%s`" % args.src,
        "- 输出目录：`%s`" % args.out,
        "- 目标尺寸：%dx%d（@2x，CSS 显示 180x280）" % (width, height),
        "- 抠底：纯白严格判据（low>=252, spread<=4）+ 四边泛洪 + 上半身宽松扩散 + 边缘去白；下半身保护白鞋白袜",
        "- 几何对齐：按人物 alpha 包围盒双线性重采样到统一基准框 %s（预乘 alpha，避免白边）" % (base,),
        "- 编码：cwebp q=82 / alpha_q=92 / method=6",
        "",
        "| 状态 | 源文件 | 原始包围盒 | 透明占比 | 产物体积 |",
        "| --- | --- | --- | --- | --- |",
    ]
    lines += ["| %s | %s | %s | %s | %s |" % row for row in rows]
    lines += [
        "",
        "合计：**%.2f KB**（5 帧，WebP 已压缩，安装包内二次压缩收益极小）" % (total_bytes / 1024),
        "",
        "退出码：0（全部成功）",
        "",
    ]
    with open(trace_path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines))
    print("留痕:", trace_path)
    print("合计: %.2f KB" % (total_bytes / 1024))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except subprocess.CalledProcessError as exc:
        print("外部命令失败:", exc)
        sys.exit(1)
    except KeyboardInterrupt:
        print("已中断")
        sys.exit(1)
    except Exception as exc:
        print("解析/运行异常:", repr(exc))
        sys.exit(2)
