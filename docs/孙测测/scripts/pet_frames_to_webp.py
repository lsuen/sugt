#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵素材落包：纯白底状态帧 → 透明 @2x WebP。

编写人员：孙测测
作用用途：把本地 ComfyUI 产出的状态帧抠成透明底、缩放到 @2x 并编码为 WebP；
          采用「从四边泛洪」而非颜色键，避免米白色外套被误抠。
适用项目：SUGT 桌面精灵素材链路
创建日期：2026-11-08
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
    """纯白 / 浅灰棋盘底纹判据。

    关键：米白外套约 (239, 235, 228)，max-min=11，不会被判成背景；
    纯白 (255,255,255) 与浅灰格 (242,242,242) 的 max-min=0，会被判成背景。
    再配合「只从四边泛洪」，角色内部的白色高光也不会被打洞。
    """
    low = min(r, g, b)
    spread = max(r, g, b) - low
    return low >= 224 and spread <= 10


def cutout(rgb, width, height):
    """从四条边泛洪标记背景，返回 RGBA 字节流与透明像素占比。"""
    total = width * height
    transparent = bytearray(total)  # 1 表示背景
    queue = deque()
    for x in range(width):
        queue.append(x)
        queue.append((height - 1) * width + x)
    for y in range(height):
        queue.append(y * width)
        queue.append(y * width + width - 1)

    while queue:
        index = queue.popleft()
        if transparent[index]:
            continue
        offset = index * 3
        if not is_background(rgb[offset], rgb[offset + 1], rgb[offset + 2]):
            continue
        transparent[index] = 1
        x, y = index % width, index // width
        if x > 0:
            queue.append(index - 1)
        if x + 1 < width:
            queue.append(index + 1)
        if y > 0:
            queue.append(index - width)
        if y + 1 < height:
            queue.append(index + width)

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

    rows = []
    total_bytes = 0
    for state in STATES:
        source = find_source(args.src, state)
        if not source:
            print("缺少状态帧:", state)
            return 1
        rgb = decode_rgb(source, width, height)
        if len(rgb) != width * height * 3:
            print("解码尺寸不符:", state, len(rgb))
            return 1
        rgba, ratio = cutout(rgb, width, height)
        tmp_png = os.path.join(args.out, ".su-%s-tmp.png" % state)
        target = os.path.join(args.out, "su-%s.webp" % state)
        write_rgba_png(tmp_png, width, height, rgba)
        run([
            "cwebp", "-quiet", "-q", "82", "-m", "6",
            "-alpha_q", "92", "-alpha_method", "1", tmp_png, "-o", target,
        ])
        os.remove(tmp_png)
        size_kb = os.path.getsize(target) / 1024
        total_bytes += os.path.getsize(target)
        if ratio < 0.15:
            print("警告：%s 透明区域仅 %.1f%%，抠底可能失败" % (state, ratio * 100))
        rows.append((state, os.path.basename(source), "%.1f%%" % (ratio * 100), "%.2f KB" % size_kb))
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
        "- 抠底：四边泛洪 + 中性浅色判据；不依赖颜色键，米白外套不误抠",
        "- 编码：cwebp q=82 / alpha_q=92 / method=6",
        "",
        "| 状态 | 源文件 | 透明占比 | 产物体积 |",
        "| --- | --- | --- | --- |",
    ]
    lines += ["| %s | %s | %s | %s |" % row for row in rows]
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
