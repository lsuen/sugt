#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵 v0.7 落包：洋红纯色底 → 软边 RGBA → 动画序列帧。

编写人员：孙测测
作用用途：
  取代 v0.5 的「纯白判据 + 四边泛洪」抠图。改为确定性色键：
    1. 从四边取背景中位色，转到色度空间 (R-G, B-G)；
    2. 每个像素按到背景色度的距离求软 alpha（抗锯齿边缘天然线性）；
    3. 反预乘去溢色，去掉洋红描边；
    4. 连通域清除孤立噪点；
    5. 按 alpha 包围盒统一几何对齐（沿用 v0.5 已验证做法）；
    6. 用亚像素仿射生成真正有差异的序列帧（呼吸 / 点头 / 轻摆），
       交给 ComfyUI `SaveAnimatedWEBP` 编码带 alpha 的动画 WebP。
  发丝缝隙是纯背景色，与连通性无关，必然 alpha=0；白鞋白袜、眼白、
  外套高光都不参与背景判据，不会被打洞。
适用项目：SUGT 桌面精灵素材链路 v0.7
创建日期：2026-11-08
复用方式：
  python3 pet_v07_frames.py [--src 洋红底帧目录] [--out 资产目录]
                            [--frames 序列帧输出目录] [--trace 留痕目录]
                            [--size 240x360] [--count 6]
  依赖系统命令 ffmpeg / cwebp；不需要 Pillow、numpy 等第三方库
退出码约定：0 全部成功 / 1 源文件或外部命令失败 / 2 参数或解析异常
"""
import argparse
import datetime
import math
import os
import struct
import subprocess
import sys
import zlib
from collections import deque

# idle 直接用洋红底定稿；其余由 Qwen-Edit 产出，文件名带 v7-chroma-<state> 前缀。
# 不再生成 blink 帧：Qwen-Edit 2509 在本角色上无论如何都保留睁眼（v0.5 同样问题），
# 闭眼改由运行时眼睑层统一实现（眨眼 + 睡眠共用），更可控也更自然。
STATES = ("idle", "typing", "done", "error", "sleep")
LOOP_STATES = ("idle", "typing", "done", "error", "sleep")

# 色键参数：背景色度噪声 p99≈15，故 14 以下全清；180 以上视为完全前景
CHROMA_FLOOR = 14.0
CHROMA_FULL = 180.0
# 连通域下限：清掉背景噪点，也清掉 Qwen-Edit 偶发的游离呆毛/孤立粒子；
# 人物主体是单一大连通域，不会误伤。
SPECK_MIN_AREA = 48


def run(cmd, **kwargs):
    """执行外部命令并返回 stdout 二进制。"""
    return subprocess.run(cmd, check=True, stdout=subprocess.PIPE, **kwargs).stdout


def decode_rgb(png_path, width, height):
    """ffmpeg 一次完成缩放与 RGB24 解码。"""
    return run([
        "ffmpeg", "-v", "error", "-y", "-i", png_path,
        "-vf", "scale=%d:%d:flags=lanczos" % (width, height),
        "-pix_fmt", "rgb24", "-f", "rawvideo", "-",
    ])


def background_chroma(rgb, width, height):
    """取四边样本各通道中位数，返回 (背景 RGB, 背景色度)。"""
    samples = []
    for x in range(0, width, max(1, width // 32)):
        for y in (0, 1, height - 2, height - 1):
            i = (y * width + x) * 3
            samples.append((rgb[i], rgb[i + 1], rgb[i + 2]))
    for y in range(0, height, max(1, height // 32)):
        for x in (0, 1, width - 2, width - 1):
            i = (y * width + x) * 3
            samples.append((rgb[i], rgb[i + 1], rgb[i + 2]))
    channels = tuple(sorted(s[c] for s in samples)[len(samples) // 2] for c in range(3))
    return channels, (channels[0] - channels[1], channels[2] - channels[1])


def key_chroma(rgb, width, height):
    """色度空间软色键 + 去溢色，返回 RGBA 字节流与透明占比。"""
    bg, bg_chroma = background_chroma(rgb, width, height)
    total = width * height
    rgba = bytearray(total * 4)
    span = CHROMA_FULL - CHROMA_FLOOR
    for index in range(total):
        src = index * 3
        red, green, blue = rgb[src], rgb[src + 1], rgb[src + 2]
        dr = (red - green) - bg_chroma[0]
        db = (blue - green) - bg_chroma[1]
        distance = math.sqrt(dr * dr + db * db)
        alpha = (distance - CHROMA_FLOOR) / span
        # 轻收紧：抹掉背景噪声带来的 0~0.08 伪 alpha，同时保留抗锯齿
        alpha = 0.0 if alpha <= 0.08 else (alpha - 0.08) / 0.92
        if alpha > 1.0:
            alpha = 1.0
        out = index * 4
        if alpha >= 0.999:
            rgba[out], rgba[out + 1], rgba[out + 2] = red, green, blue
            rgba[out + 3] = 255
        elif alpha <= 0.0:
            rgba[out + 3] = 0
        else:
            # 反预乘去溢色：rgb = a*前景 + (1-a)*背景，解出前景
            effective = max(alpha, 0.25)
            inv = 1.0 - alpha
            rgba[out] = min(255, max(0, int((red - inv * bg[0]) / effective)))
            rgba[out + 1] = min(255, max(0, int((green - inv * bg[1]) / effective)))
            rgba[out + 2] = min(255, max(0, int((blue - inv * bg[2]) / effective)))
            rgba[out + 3] = int(alpha * 255 + 0.5)
    return bytes(rgba), bg


def clean_specks(rgba, width, height):
    """清除孤立的小块伪前景（背景噪声），保留发丝等细连通结构。"""
    data = bytearray(rgba)
    total = width * height
    seen = bytearray(total)
    removed = 0
    for start in range(total):
        if seen[start] or data[start * 4 + 3] < 24:
            continue
        queue = deque([start])
        seen[start] = 1
        component = []
        while queue:
            index = queue.popleft()
            component.append(index)
            x, y = index % width, index // width
            neighbours = []
            if x > 0:
                neighbours.append(index - 1)
            if x + 1 < width:
                neighbours.append(index + 1)
            if y > 0:
                neighbours.append(index - width)
            if y + 1 < height:
                neighbours.append(index + width)
            for neighbour in neighbours:
                if seen[neighbour] or data[neighbour * 4 + 3] < 24:
                    continue
                seen[neighbour] = 1
                queue.append(neighbour)
        if len(component) < SPECK_MIN_AREA:
            for index in component:
                data[index * 4 + 3] = 0
            removed += 1
    return bytes(data), removed


def alpha_bbox(rgba, width, height):
    """非透明像素包围盒。"""
    min_x, min_y, max_x, max_y = width, height, -1, -1
    for y in range(height):
        row = y * width
        for x in range(width):
            if rgba[(row + x) * 4 + 3] > 8:
                min_x = min(min_x, x)
                max_x = max(max_x, x)
                min_y = min(min_y, y)
                max_y = max(max_y, y)
    if max_x < 0:
        return (0, 0, width, height)
    return (min_x, min_y, max_x + 1, max_y + 1)


def median_box(boxes):
    """各坐标分量取中位数，单帧异常不拖偏全部素材。"""
    count = len(boxes)
    return tuple(sorted(box[i] for box in boxes)[count // 2] for i in range(4))


def resample(rgba, width, height, mapper):
    """通用亚像素重采样：mapper(x, y) -> (sx, sy) 或 None（落在画外）。

    先预乘 alpha 再双线性插值，避免透明像素残留的洋红 RGB 渗进边缘；
    输出前反预乘还原颜色。
    """
    premul = bytearray(len(rgba))
    for i in range(0, len(rgba), 4):
        a = rgba[i + 3]
        premul[i] = rgba[i] * a // 255
        premul[i + 1] = rgba[i + 1] * a // 255
        premul[i + 2] = rgba[i + 2] * a // 255
        premul[i + 3] = a
    out = bytearray(len(rgba))
    for y in range(height):
        for x in range(width):
            point = mapper(x, y)
            if point is None:
                continue
            fx, fy = point
            if fx < 0 or fy < 0 or fx >= width - 1 or fy >= height - 1:
                continue
            x0, y0 = int(fx), int(fy)
            wx, wy = fx - x0, fy - y0
            i00 = (y0 * width + x0) * 4
            i01 = i00 + 4
            i10 = i00 + width * 4
            i11 = i10 + 4
            w00, w01 = (1 - wx) * (1 - wy), wx * (1 - wy)
            w10, w11 = (1 - wx) * wy, wx * wy
            di = (y * width + x) * 4
            for c in range(4):
                out[di + c] = int(
                    premul[i00 + c] * w00 + premul[i01 + c] * w01
                    + premul[i10 + c] * w10 + premul[i11 + c] * w11
                )
    for i in range(0, len(out), 4):
        a = out[i + 3]
        if a > 0:
            for c in range(3):
                out[i + c] = min(255, out[i + c] * 255 // a)
    return bytes(out)


def warp_to_box(rgba, width, height, src, dst):
    """把人物包围盒重采样到统一基准框，消除状态切换忽大忽小。"""
    sx0, sy0, sx1, sy1 = src
    dx0, dy0, dx1, dy1 = dst
    src_w, src_h = sx1 - sx0, sy1 - sy0
    dst_w, dst_h = dx1 - dx0, dy1 - dy0

    def mapper(x, y):
        if x < dx0 or x >= dx1 or y < dy0 or y >= dy1:
            return None
        return (sx0 + (x - dx0 + 0.5) * src_w / dst_w - 0.5,
                sy0 + (y - dy0 + 0.5) * src_h / dst_h - 0.5)

    return resample(rgba, width, height, mapper)


def motion_frames(rgba, width, height, state, count):
    """生成真正有差异的序列帧：缓慢呼吸 / 点头 / 轻摆，首尾严格闭合。"""
    pivot_x, pivot_y = width / 2.0, height * 0.96
    frames = []
    for k in range(count):
        phase = 2.0 * math.pi * k / count
        breath = (1.0 - math.cos(phase)) * 0.5  # 0 → 1 → 0，首尾为 0
        if state == "idle":
            dx = 0.35 * math.sin(phase)
            dy = -1.35 * breath
            rotation = 0.0035 * math.sin(phase)
            scale = 1.0 + 0.0035 * math.cos(phase)
        elif state == "typing":
            dx = 0.18 * math.sin(phase)
            dy = -0.75 * breath
            rotation = 0.0022 * math.sin(2 * phase)
            scale = 1.0 + 0.0018 * math.cos(phase)
        elif state == "done":
            dx = 0.25 * math.sin(phase)
            dy = -1.7 * breath
            rotation = 0.0030 * math.sin(phase)
            scale = 1.0 + 0.0045 * math.cos(phase)
        elif state == "error":
            dx = 0.75 * math.sin(phase)
            dy = -0.35 * breath
            rotation = 0.0050 * math.sin(phase)
            scale = 1.0
        else:  # sleep：幅度最小，配合更慢的 CSS 呼吸
            dx = 0.10 * math.sin(phase)
            dy = -0.55 * breath
            rotation = 0.0010 * math.sin(phase)
            scale = 1.0 + 0.0020 * math.cos(phase)
        cos_r, sin_r = math.cos(rotation), math.sin(rotation)

        def mapper(x, y, dx=dx, dy=dy, cos_r=cos_r, sin_r=sin_r, scale=scale):
            vx, vy = x - pivot_x, y - pivot_y
            sx = pivot_x + (vx * cos_r + vy * sin_r) / scale - dx
            sy = pivot_y + (-vx * sin_r + vy * cos_r) / scale - dy
            return (sx, sy)

        frames.append(resample(rgba, width, height, mapper))
    return frames


# 对齐后 240×360 帧中两只眼睛的实测位置（放大 6 倍目视标定）。
# 只在这两个椭圆区域内替换像素，绝不碰到中间的刘海与脸颊。
EYE_BOXES = ((102, 83, 117, 97), (124, 83, 139, 97))
SKIN_COLOR = (249, 222, 211)
LASH_COLOR = (43, 40, 44)


def make_closed_eyes(rgba, width, height):
    """程序化闭眼帧：羽化椭圆填肤色 + 一道向下弯的睫毛线。

    Qwen-Edit 2509 在本角色上无法稳定执行「闭眼」（v0.5 已记录），
    因此在落包阶段做确定性局部重绘：比 CSS 贴片更贴合线稿，也不会盖住刘海。
    """
    data = bytearray(rgba)
    for x0, y0, x1, y1 in EYE_BOXES:
        cx, cy = (x0 + x1) / 2.0, (y0 + y1) / 2.0
        rx, ry = (x1 - x0) / 2.0, (y1 - y0) / 2.0
        # 第一步：椭圆内用肤色覆盖虹膜/眼白/高光，边缘 2px 羽化避免硬方块
        for y in range(y0, y1):
            for x in range(x0, x1):
                nx, ny = (x - cx) / rx, (y - cy) / ry
                # 椭圆内部完全覆盖（含上睫毛线），只在边界外 1~2px 羽化，
                # 否则上睫毛的深色像素会在额头残留成一小块脏斑。
                coverage = max(0.0, min(1.0, (1.34 - nx * nx - ny * ny) / 0.34))
                if coverage <= 0:
                    continue
                index = (y * width + x) * 4
                if data[index + 3] < 32:
                    continue
                for channel in range(3):
                    old = data[index + channel]
                    data[index + channel] = int(old + (SKIN_COLOR[channel] - old) * coverage)
        # 第二步：向下弯弧线（两端高、中间低），两像素软边模拟线稿抗锯齿
        for x in range(x0 + 2, x1 - 1):
            nx = (x - cx) / rx
            yc = cy + 0.5 + 2.1 * nx * nx
            for offset, weight in ((0, 0.88), (1, 0.32)):
                y = int(yc) + offset
                if y >= height:
                    continue
                index = (y * width + x) * 4
                if data[index + 3] < 32:
                    continue
                for channel in range(3):
                    old = data[index + channel]
                    data[index + channel] = int(old + (LASH_COLOR[channel] - old) * weight)
    return bytes(data)


def png_chunk(tag, data):
    return (
        struct.pack(">I", len(data)) + tag + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def write_rgba_png(path, width, height, rgba):
    """标准库手写 RGBA PNG，不引入图像库依赖。"""
    scanlines = b"".join(
        b"\x00" + rgba[y * width * 4:(y + 1) * width * 4] for y in range(height)
    )
    payload = (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + png_chunk(b"IDAT", zlib.compress(scanlines, 6))
        + png_chunk(b"IEND", b"")
    )
    with open(path, "wb") as handle:
        handle.write(payload)


def encode_webp(tmp_png, target, quality=86):
    run([
        "cwebp", "-quiet", "-q", str(quality), "-m", "6",
        "-alpha_q", "94", "-alpha_method", "1", tmp_png, "-o", target,
    ])


def find_source(src_dir, state):
    """idle 直接取洋红底定稿；其余取 v7-chroma-<state>_*.png。"""
    if state == "idle":
        for name in ("pet-chroma-base.png", "v7-chroma-idle_00001_.png"):
            path = os.path.join(src_dir, name)
            if os.path.isfile(path):
                return path
    hits = sorted(
        name for name in os.listdir(src_dir)
        if name.startswith("v7-chroma-%s_" % state) and name.lower().endswith(".png")
    )
    return os.path.join(src_dir, hits[0]) if hits else None


def main():
    parser = argparse.ArgumentParser(description="桌面精灵 v0.7 洋红底色键与动画序列帧")
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.abspath(os.path.join(here, "..", "..", ".."))
    parser.add_argument(
        "--src", default=os.path.join(root, "docs", "design", "assets", "generated", "v4"))
    parser.add_argument("--out", default=os.path.join(root, "src", "assets", "pet"))
    parser.add_argument(
        "--frames",
        default=os.path.join(root, "docs", "design", "assets", "generated", "v4", "frames"))
    parser.add_argument("--trace", default=os.path.join(root, "docs", "孙测测", "traces"))
    parser.add_argument("--size", default="240x360", help="@2x 目标尺寸，格式 宽x高")
    parser.add_argument("--count", type=int, default=6, help="每个循环的差异帧数")
    args = parser.parse_args()
    try:
        width, height = (int(part) for part in args.size.lower().split("x"))
    except Exception:
        print("尺寸格式应为 宽x高，例如 240x360")
        return 2
    if args.count < 2:
        print("循环帧数至少 2")
        return 2
    if not os.path.isdir(args.src):
        print("源目录不存在:", args.src)
        return 1
    os.makedirs(args.out, exist_ok=True)
    os.makedirs(args.frames, exist_ok=True)
    os.makedirs(args.trace, exist_ok=True)

    # 第一阶段：全部帧色键 + 去溢色 + 量出人物包围盒
    keyed = []
    rows = []
    for state in STATES:
        source = find_source(args.src, state)
        if not source:
            print("缺少洋红底状态帧:", state)
            return 1
        rgb = decode_rgb(source, width, height)
        if len(rgb) != width * height * 3:
            print("解码尺寸不符:", state, len(rgb))
            return 1
        rgba, bg = key_chroma(rgb, width, height)
        rgba, specks = clean_specks(rgba, width, height)
        box = alpha_bbox(rgba, width, height)
        opaque = sum(1 for i in range(3, len(rgba), 4) if rgba[i] > 8)
        keyed.append((state, source, rgba, box))
        rows.append((state, os.path.basename(source), str(bg), str(box),
                     "%.1f%%" % (100.0 * opaque / (width * height)), str(specks)))
        print("色键 %-7s 背景=%s bbox=%s 前景=%.1f%% 清除噪点=%d" % (
            state, bg, box, 100.0 * opaque / (width * height), specks), flush=True)

    base = median_box([item[3] for item in keyed])
    print("统一基准框:", base, flush=True)
    aligned = {
        state: warp_to_box(rgba, width, height, box, base)
        for state, _src, rgba, box in keyed
    }
    # 睡眠帧闭眼（Qwen-Edit 画不出来）；另存一份待机闭眼帧供运行时眨眼叠加。
    aligned["sleep"] = make_closed_eyes(aligned["sleep"], width, height)
    closed_idle = make_closed_eyes(aligned["idle"], width, height)

    # 第二阶段：输出静态海报 + 逐帧动画序列（供 ComfyUI 编码动画 WebP）
    total_bytes = 0
    for state, _src, _rgba, _box in keyed:
        poster = os.path.join(args.out, "su-%s.webp" % state)
        tmp = os.path.join(args.out, ".v07-%s-tmp.png" % state)
        write_rgba_png(tmp, width, height, aligned[state])
        encode_webp(tmp, poster)
        os.remove(tmp)
        total_bytes += os.path.getsize(poster)
        print("OK  海报 su-%-7s %.2f KB" % (
            state + ".webp", os.path.getsize(poster) / 1024), flush=True)

    blink_poster = os.path.join(args.out, "su-blink.webp")
    blink_tmp = os.path.join(args.out, ".v07-blink-tmp.png")
    write_rgba_png(blink_tmp, width, height, closed_idle)
    encode_webp(blink_tmp, blink_poster)
    os.remove(blink_tmp)
    total_bytes += os.path.getsize(blink_poster)
    print("OK  闭眼帧 su-blink.webp %.2f KB" % (os.path.getsize(blink_poster) / 1024), flush=True)

    for state in LOOP_STATES:
        folder = os.path.join(args.frames, state)
        os.makedirs(folder, exist_ok=True)
        for name in os.listdir(folder):
            if name.startswith("frame-") and name.endswith(".png"):
                os.remove(os.path.join(folder, name))
        for index, frame in enumerate(motion_frames(aligned[state], width, height,
                                                    state, args.count)):
            write_rgba_png(os.path.join(folder, "frame-%02d.png" % index),
                           width, height, frame)
        print("OK  序列 %-7s %d 帧 -> %s" % (state, args.count, folder), flush=True)

    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    trace_path = os.path.join(args.trace, "桌面精灵v0.7色键落包-%s.md" % stamp)
    lines = [
        "# 桌面精灵 v0.7 色键落包留痕",
        "",
        "- 执行时间：%s" % datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 脚本：`docs/孙测测/scripts/pet_v07_frames.py`",
        "- 来源目录：`%s`" % args.src,
        "- 资产目录：`%s`" % args.out,
        "- 序列帧目录：`%s`" % args.frames,
        "- 目标尺寸：%dx%d（@2x）｜每循环 %d 帧｜fps=8" % (width, height, args.count),
        "- 抠底：洋红纯色底 → 色度空间 (R-G, B-G) 软色键"
        "（floor=%.0f / full=%.0f）→ 反预乘去溢色 → 连通域清噪"
        % (CHROMA_FLOOR, CHROMA_FULL),
        "- 几何对齐：alpha 包围盒重采样到统一基准框 %s" % (base,),
        "- 编码：静态海报 cwebp q=86 / alpha_q=94；序列帧为 RGBA PNG，"
        "交 ComfyUI `SaveAnimatedWEBP` 编码带 alpha 的动画 WebP",
        "",
        "| 状态 | 源文件 | 背景中位色 | 原始包围盒 | 前景占比 | 清除噪点块 |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    lines += ["| %s | %s | %s | %s | %s | %s |" % row for row in rows]
    lines += [
        "",
        "静态海报合计：**%.2f KB**（%d 张，动画 WebP 体积在 ComfyUI 编码后另计）"
        % (total_bytes / 1024, len(STATES)),
        "",
        "退出码：0（全部成功）",
        "",
    ]
    with open(trace_path, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines))
    print("留痕:", trace_path)
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
