#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-018 桌面精灵「姿态序列一致性」验收门禁。

编写人员：孙测测
作用用途：
  1000 帧素材不是「一张好看的定稿复制 1000 次」，而是同一 master 派生
  出的姿态序列。单帧门禁（SCR-016 / 017）看不见序列级缺陷，本脚本补上：
    1. 宽高比与参考一致（防整段构图变形）；
    2. 四角背景均匀（防渐变 / 地面阴影）；
    3. 背景色度满足全局 chroma 色键前提（防自动模式误判成浅色底泛洪）；
    4. 复用 SCR-016 的 chroma 色键 + 五项验收全部通过；
    5. 归一化主体 bbox 与参考偏差在容差内（防角色忽大忽小 / 左右漂移）；
    6. 白 / 紫红 / 炭灰 / 黑四类关键色块不得缺失（防换装、换发色）；
    7. 脚部留白 >= 2% 画布高（防构图漂移裁掉鞋子）；
    8. 无脚下亮青平台 / 聚光椭圆（加留白后 Edit 会画出发光台面，
       且因色距 >95 被色键的近色保护放过，色键五项仍全绿）。
  ``--video`` 额外逐帧验收 H3 / VFI 补间输出的动画 WebP：每帧独立取
  四角背景跑自适应色键（视频模型底色会逐帧漂移），再汇总为五项判定
  （逐帧色键全通过 / chroma 前提 / 漂移可控 / 脚部留白 / 1000 帧体积）。
  诊断项（记录但不判红）：背景 RGB 漂移量、头顶留白。
  2026-11-10 实测：Edit 2509 会把纯青蓝底 (91,145,190) 漂成
  (82,171,187)，B-G 从 47 掉到 16；旧 detect_mode 要求 B-G>=20 会误判
  成浅色底，抠图退化为整图（bbox 100% / 透明 0.3%）。故 SCR-016 的
  chroma 阈值改为按本图背景 score 自适应，本脚本再把「色度前提」列为
  批量生产的显式门禁。

适用项目：SUGT 桌面精灵 v1.2 素材链路（本地执行，不依赖 ComfyUI / 网络）
创建日期：2026-11-10
复用方式：
  python3 check_pet_pose_series.py --reference <蓝底定稿.png> \
      --pose head-turn=/tmp/a.png --pose wave=/tmp/b.png \
      [--video /tmp/h3.webp] [--out /tmp/sugt-pose-series] [--root .]
  需 Pillow（本地素材链路脚本，不进 CI）。产物只落输出目录与 traces，
  **绝不写入 src/assets/pet**。
退出码约定：0 全部通过 / 1 存在验收不通过 / 2 参数、依赖或解析异常
"""
import argparse
import importlib.util
import io
import math
import sys
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

_MAX_ASPECT_DELTA = 0.02
_MAX_CORNER_SPREAD = 24
_MIN_CHROMA_SCORE = 8.0
_MAX_BBOX_DELTA = 0.05
_MIN_COLOUR_KEEP = 0.50
_MIN_BOTTOM_MARGIN = 0.02
_MAX_VIDEO_DRIFT = 40
_MAX_1000_FRAME_MB = 8.0
_MAX_PLATFORM_RATIO = 0.002
_SOLID_ALPHA = 32
_BOX_DENSITY = 48
_SAMPLE_STEP = 2


def load_cutout_module():
    """动态载入 SCR-016，复用其色键与五项验收，不复制第二套实现。"""
    path = _HERE / "check_pet_keyvisual_cutout.py"
    spec = importlib.util.spec_from_file_location("sugt_pet_cutout", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def colour_profile(image):
    """统计四类关键色块占「非背景像素」的比例，用于抓换装 / 换发色。

    分类刻意只依赖 RGB 关系，不依赖固定色值，避免背景漂移影响判定。
    """
    width, height = image.size
    pixels = image.convert("RGB")
    counts = {"白袜鞋挂件": 0, "紫红发尾": 0, "炭灰卫衣": 0, "黑发黑裤": 0}
    total = 0
    for y in range(0, height, _SAMPLE_STEP):
        for x in range(0, width, _SAMPLE_STEP):
            red, green, blue = pixels.getpixel((x, y))
            spread = max(red, green, blue) - min(red, green, blue)
            # 背景青蓝：两通道同时为正，直接排除。
            if min(green - red, blue - green) >= 8:
                continue
            total += 1
            if min(red, green, blue) >= 225 and spread <= 22:
                counts["白袜鞋挂件"] += 1
            if ((red > green + 20 and blue > green + 15 and red > 70)
                    or (red > green + 50 and red > blue + 40 and red > 110)):
                counts["紫红发尾"] += 1
            if 100 <= red <= 190 and spread <= 18 and blue - red <= 10:
                counts["炭灰卫衣"] += 1
            if max(red, green, blue) <= 70:
                counts["黑发黑裤"] += 1
    return {key: value / max(1, total) for key, value in counts.items()}


def solid_box(rgba):
    """按「实心 alpha + 4×4 密度」求稳健主体 bbox。

    SCR-016 的 measure 用 alpha>0 统计全部占用像素；色键边缘会留下
    alpha=1~3 的半透明毛边，甚至零星 alpha>=32 的孤立列，足以把 bbox
    撑到画布边界，造成序列级「角色漂移」的假阳性（实测 wave 左边界被
    撑到 x=0、偏差 0.130）。BOX 下采样把稀疏噪点列稀释掉，只保留有成片
    实心像素的行列，精度 ±4px 对归一化判定足够。
    """
    mask = rgba.getchannel("A").point(
        lambda value: 255 if value >= _SOLID_ALPHA else 0)
    factor = 4
    small = mask.resize((max(1, mask.width // factor),
                         max(1, mask.height // factor)), Image.BOX)
    dense = small.point(lambda value: 255 if value >= _BOX_DENSITY else 0)
    box = dense.getbbox()
    if not box:
        return (0, 0, 0, 0)
    return (box[0] * factor, box[1] * factor,
            min(mask.width - 1, (box[2] + 1) * factor - 1),
            min(mask.height - 1, (box[3] + 1) * factor - 1))


def platform_ratio(image):
    """检测「脚下亮青色聚光平台 / 椭圆台面」占全图比例。

    给输入画布加底部留白后，Edit 2509 会把留白画成一个亮青椭圆台面
    （实测 ~(90,230,245)）。它到背景色距 >95，会被 SCR-016 的
    ``_CHROMA_NEAR`` 当成「远离底色的青色配饰」保护下来，色键五项
    仍然全绿但深底上能看到一块发光地板。角色的薄荷绿发夹 B 通道
    明显低于 G，不会被这个判据误伤。
    """
    width, height = image.size
    pixels = image.convert("RGB")
    hits = 0
    samples = 0
    for y in range(0, height, _SAMPLE_STEP):
        for x in range(0, width, _SAMPLE_STEP):
            red, green, blue = pixels.getpixel((x, y))
            samples += 1
            if (green > 195 and blue > 195 and red < 150
                    and blue > green + 5):
                hits += 1
    return hits / max(1, samples)


def normalised_box(box, size):
    """把像素 bbox 归一到 0~1，消除不同输出分辨率的影响。"""
    width, height = size
    return (box[0] / width, box[1] / height,
            box[2] / width, box[3] / height)


def corner_stats(image, module):
    """复用 SCR-016 四角统计。"""
    return module.corner_stats(image)


def evaluate(reference, tag, path, module, out_dir):
    """对单张姿态图执行六项判定，返回 (判定列表, 诊断字典, 抠图 RGBA)。"""
    image = Image.open(path).convert("RGB")
    ref_rgb = Image.open(reference).convert("RGB")
    ref_background, _, _ = corner_stats(ref_rgb, module)
    background, spans, corners = corner_stats(image, module)
    score = min(background[1] - background[0],
                background[2] - background[1])
    drift = math.dist(background, ref_background)

    rgba = module.chroma_key(image, background)
    cut_checks, metrics = module.measure(image, rgba, "chroma", background)
    cut_passed = sum(1 for _, passed, _ in cut_checks if passed)

    box = solid_box(rgba)
    norm = normalised_box(box, image.size)
    ref_rgba = module.chroma_key(ref_rgb, ref_background)
    ref_norm = normalised_box(solid_box(ref_rgba), ref_rgb.size)
    bbox_delta = max(abs(norm[i] - ref_norm[i]) for i in range(4))
    bottom_margin = image.height - 1 - box[3]
    bottom_ratio = bottom_margin / image.height
    platform = platform_ratio(image)

    aspect = image.width / image.height
    ref_aspect = ref_rgb.width / ref_rgb.height

    profile = colour_profile(image)
    ref_profile = colour_profile(ref_rgb)
    keeps = {key: profile[key] / max(1e-6, ref_profile[key])
             for key in profile}
    missing = [key for key, value in keeps.items()
               if value < _MIN_COLOUR_KEEP]

    checks = [
        ("宽高比一致", abs(aspect - ref_aspect) <= _MAX_ASPECT_DELTA,
         "%.4f vs 参考 %.4f（容差 %.2f）" % (
             aspect, ref_aspect, _MAX_ASPECT_DELTA)),
        ("四角背景均匀", max(spans) <= _MAX_CORNER_SPREAD,
         "RGB 极差 %d/%d/%d（阈值 <=%d）" % (
             spans[0], spans[1], spans[2], _MAX_CORNER_SPREAD)),
        ("背景色度满足 chroma 前提", score >= _MIN_CHROMA_SCORE,
         "min(G-R,B-G)=%d（阈值 >=%d）；背景 %s" % (
             score, _MIN_CHROMA_SCORE, str(background))),
        ("色键五项全通过", cut_passed == len(cut_checks),
         "%d/%d，残留 %.2f%%，白色衣物 %.2f%%，透明 %.1f%%" % (
             cut_passed, len(cut_checks), metrics["residue_ratio"] * 100,
             metrics["white_keep"] * 100,
             metrics["transparent_ratio"] * 100)),
        ("主体 bbox 与参考一致", bbox_delta <= _MAX_BBOX_DELTA,
         "最大偏差 %.3f（阈值 <=%.2f）；归一化 bbox %s" % (
             bbox_delta, _MAX_BBOX_DELTA,
             str(tuple(round(value, 3) for value in norm)))),
        ("关键色块未缺失", not missing,
         "缺失=%s；保留倍数 %s" % (
             str(missing or "无"),
             str({key: round(value, 2) for key, value in keeps.items()}))),
        ("脚部留白充足", bottom_ratio >= _MIN_BOTTOM_MARGIN,
         "底边距 %d px / %.2f%%（阈值 >=%.0f%%；贴底会被构图漂移裁鞋）" % (
             bottom_margin, bottom_ratio * 100,
             _MIN_BOTTOM_MARGIN * 100)),
        ("无脚下亮青平台", platform <= _MAX_PLATFORM_RATIO,
         "亮青平台像素 %.3f%%（阈值 <=%.2f%%）" % (
             platform * 100, _MAX_PLATFORM_RATIO * 100)),
    ]

    diagnostics = {
        "background": background,
        "reference_background": ref_background,
        "background_drift": drift,
        "chroma_score": score,
        "bottom_margin_px": bottom_margin,
        "top_margin_px": box[1],
        "bottom_margin_ratio": bottom_ratio,
        "bbox_delta": bbox_delta,
        "platform_ratio": platform,
        "colour_keeps": keeps,
        "cut_checks": cut_checks,
        "metrics": metrics,
    }

    cutout = out_dir / ("series-%s-cutout.png" % tag)
    compare = out_dir / ("series-%s-compare.png" % tag)
    rgba.save(cutout)
    module.build_compare(rgba, metrics["box"], compare)
    return checks, diagnostics, cutout, compare


def evaluate_video(reference, video_path, module, out_dir):
    """逐帧验收 H3 补间输出的动画 WebP，返回 (判定, 逐帧明细, 抽样体积)。

    H3 视频模型的底色会逐帧漂移（实测 G 通道 142→170），单帧色卡写死
    必然失败，故每帧独立取四角背景跑自适应色键，并统计漂移区间。
    """
    video = Image.open(video_path)
    count = getattr(video, "n_frames", 1)
    ref_rgb = Image.open(reference).convert("RGB")
    ref_background, _, _ = corner_stats(ref_rgb, module)
    rows = []
    sizes = []
    sample_at = {0, max(0, count // 2), count - 1}
    for index in range(count):
        video.seek(index)
        frame = video.convert("RGB")
        background, spans, corners = corner_stats(frame, module)
        score = min(background[1] - background[0],
                    background[2] - background[1])
        rgba = module.chroma_key(frame, background)
        checks, metrics = module.measure(frame, rgba, "chroma", background)
        passed = sum(1 for _, ok, _ in checks if ok)
        bottom = frame.height - 1 - solid_box(rgba)[3]
        rows.append({
            "index": index, "background": background, "score": score,
            "spread": max(spans), "passed": passed, "total": len(checks),
            "residue": metrics["residue_ratio"],
            "white_keep": metrics["white_keep"],
            "bottom_ratio": bottom / frame.height,
            "platform": platform_ratio(frame),
        })
        if index in sample_at:
            box = rgba.getbbox()
            crop = rgba.crop(box) if box else rgba
            canvas = Image.new("RGBA", (105, 297), (0, 0, 0, 0))
            fit = crop.copy()
            fit.thumbnail((105, 297), Image.LANCZOS)
            canvas.alpha_composite(
                fit, ((105 - fit.width) // 2, (297 - fit.height) // 2))
            quantised = canvas.quantize(
                colors=96, method=Image.Quantize.FASTOCTREE).convert("RGBA")
            buffer = io.BytesIO()
            quantised.save(buffer, format="WEBP", lossless=True, method=4)
            sizes.append(buffer.tell())
            rgba.save(out_dir / ("series-video-f%02d.png" % index))
    greens = [row["background"][1] for row in rows]
    drift = max(greens) - min(greens)
    average = sum(sizes) / max(1, len(sizes))
    checks = [
        ("视频逐帧色键全通过",
         all(row["passed"] == row["total"] for row in rows),
         "%d/%d 帧五项全通过" % (
             sum(1 for row in rows if row["passed"] == row["total"]),
             count)),
        ("视频逐帧满足 chroma 前提",
         all(row["score"] >= _MIN_CHROMA_SCORE for row in rows),
         "背景 score 最小 %d（阈值 >=%d）" % (
             min(row["score"] for row in rows), _MIN_CHROMA_SCORE)),
        ("背景逐帧漂移可控", drift <= _MAX_VIDEO_DRIFT,
         "G 通道极差 %d（阈值 <=%d）" % (drift, _MAX_VIDEO_DRIFT)),
        ("视频逐帧脚部留白充足",
         all(row["bottom_ratio"] >= _MIN_BOTTOM_MARGIN for row in rows),
         "最小 %.2f%%（阈值 >=%.0f%%）" % (
             min(row["bottom_ratio"] for row in rows) * 100,
             _MIN_BOTTOM_MARGIN * 100)),
        ("视频逐帧无脚下亮青平台",
         all(row["platform"] <= _MAX_PLATFORM_RATIO for row in rows),
         "最大 %.3f%%（阈值 <=%.2f%%）" % (
             max(row["platform"] for row in rows) * 100,
             _MAX_PLATFORM_RATIO * 100)),
        ("1000 帧体积在预算内",
         average * 1000 / 1024 / 1024 <= _MAX_1000_FRAME_MB,
         "抽样均值 %d B / 帧 → %.2f MB（阈值 <=%.0f MB）" % (
             average, average * 1000 / 1024 / 1024,
             _MAX_1000_FRAME_MB)),
    ]
    return checks, rows, sizes


def build_overview(reference, poses, out_dir):
    """参考 + 各姿态原图 + 各姿态抠图，横向并排总览。"""
    panels = [("参考", Image.open(reference).convert("RGB"))]
    for tag, source, cutout in poses:
        panels.append((tag + " 原图", Image.open(source).convert("RGB")))
        panels.append((tag + " 抠图", Image.open(cutout).convert("RGBA")))
    thumb_height = 460
    prepared = []
    for label, image in panels:
        scale = thumb_height / image.height
        size = (max(1, int(image.width * scale)), thumb_height)
        prepared.append((label, image.resize(size, Image.LANCZOS)))
    gap, label_height = 12, 26
    width = sum(image.width for _, image in prepared) + gap * len(prepared)
    canvas = Image.new("RGB", (width, thumb_height + label_height),
                       (245, 245, 247))
    draw = ImageDraw.Draw(canvas)
    x = 0
    for label, image in prepared:
        if image.mode == "RGBA":
            base = Image.new("RGBA", image.size, (225, 228, 233, 255))
            base.alpha_composite(image)
            image = base.convert("RGB")
        canvas.paste(image, (x, label_height))
        draw.text((x + 4, 6), label, fill=(30, 34, 44))
        x += image.width + gap
    target = out_dir / "series-overview.png"
    canvas.save(target)
    return target


def write_trace(trace, started, reference, results, video=None):
    """按孙测测规范落 Markdown 留痕。"""
    lines = [
        "# 精灵姿态序列验收留痕",
        "",
        "- 生成时间：%s" % started,
        "- 脚本：SCR-018 `check_pet_pose_series.py`",
        "- 参考定稿：`%s`" % reference,
        "- 退出码约定：0 全通过 / 1 存在不通过 / 2 解析异常",
        "",
        "## 判定汇总",
        "",
        "| 姿态 | 通过/总数 | 结论 |",
        "| --- | --- | --- |",
    ]
    passed_total = 0
    check_total = 0
    for item in results:
        passed = sum(1 for _, ok, _ in item["checks"] if ok)
        passed_total += passed
        check_total += len(item["checks"])
        lines.append("| %s | %d/%d | %s |" % (
            item["tag"], passed, len(item["checks"]),
            "通过" if passed == len(item["checks"]) else "不通过"))
    if video:
        video_checks, video_rows, _, video_path = video
        passed = sum(1 for _, ok, _ in video_checks if ok)
        passed_total += passed
        check_total += len(video_checks)
        lines.append("| 视频补间 `%s`（%d 帧） | %d/%d | %s |" % (
            Path(video_path).name, len(video_rows), passed,
            len(video_checks),
            "通过" if passed == len(video_checks) else "不通过"))
    lines += ["", "总计 **%d/%d** 通过。" % (passed_total, check_total),
              "", "## 逐项明细", ""]
    for item in results:
        lines.append("### %s" % item["tag"])
        lines.append("")
        lines.append("- 源图：`%s`" % item["source"])
        for title, ok, detail in item["checks"]:
            lines.append("- [%s] %s — %s" % (
                "通过" if ok else "不通过", title, detail))
        diag = item["diagnostics"]
        lines += [
            "",
            "诊断（不计入通过率）：",
            "",
            "- 背景 %s，参考 %s，色距 %.1f，min(G-R,B-G)=%d" % (
                diag["background"], diag["reference_background"],
                diag["background_drift"], diag["chroma_score"]),
            "- 脚部底边距 %d px（%.2f%%），头顶留白 %d px" % (
                diag["bottom_margin_px"],
                diag["bottom_margin_ratio"] * 100,
                diag["top_margin_px"]),
            "- 抠图产物：`%s`；四联对照：`%s`" % (
                item["cutout"], item["compare"]),
            "",
        ]
    if video:
        video_checks, video_rows, sizes, video_path = video
        lines += [
            "### 视频补间逐帧",
            "",
            "- 来源：`%s`，共 %d 帧" % (video_path, len(video_rows)),
            "- 抽样 105×297 / 96 色 lossless 单帧：%s B" % (
                " / ".join(str(value) for value in sizes)),
            "",
            "| 帧 | 背景 | score | 极差 | 五项 | 残留% | 白% | 底边距% | 平台% |",
            "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
        ]
        for row in video_rows:
            lines.append(
                "| %d | %s | %d | %d | %d/%d | %.2f | %.1f | %.2f | %.3f |" % (
                    row["index"], row["background"], row["score"],
                    row["spread"], row["passed"], row["total"],
                    row["residue"] * 100, row["white_keep"] * 100,
                    row["bottom_ratio"] * 100, row["platform"] * 100))
        lines += ["", "视频判定：", ""]
        for title, ok, detail in video_checks:
            lines.append("- [%s] %s — %s" % (
                "通过" if ok else "不通过", title, detail))
        lines.append("")
    lines += [
        "## 结论与硬约束",
        "",
        "- 本门禁只验收「同一 master 派生的姿态序列」是否可批量生产；",
        "  通过不等于素材已落包，落包还需 SCR-017 + manifest v2 + 全套替换。",
        "- 脚部底边距过小是生产前置风险：送 Edit 前必须给输入图底部留白，",
        "  否则任何构图漂移都会裁掉鞋子。",
        "- 背景色逐张漂移是常态，色键必须按本图背景 score 自适应，",
        "  禁止写死单一背景色。",
        "",
    ]
    trace.write_text("\n".join(lines), encoding="utf-8")
    return passed_total, check_total


def main():
    parser = argparse.ArgumentParser(description="精灵姿态序列一致性验收")
    parser.add_argument("--reference", required=True,
                        help="蓝底 master 定稿（序列基准）")
    parser.add_argument("--pose", action="append", default=[],
                        metavar="TAG=PATH", help="姿态图，可重复传入")
    parser.add_argument("--video", help="H3 / VFI 补间输出的动画 WebP，逐帧验收")
    parser.add_argument("--out", default="/tmp/sugt-pose-series",
                        help="产物目录，禁止指向 src/assets/pet")
    parser.add_argument("--root", default=str(_ROOT), help="项目根目录")
    args = parser.parse_args()

    reference = Path(args.reference).resolve()
    if not reference.is_file():
        print("参考定稿不存在：%s" % reference)
        return 2
    out_dir = Path(args.out).resolve()
    protected = Path(args.root).resolve() / "src" / "assets" / "pet"
    if out_dir == protected or protected in out_dir.parents:
        print("输出目录不得位于 src/assets/pet")
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)

    poses = []
    for raw in args.pose:
        if "=" not in raw:
            print("姿态参数格式应为 TAG=PATH：%s" % raw)
            return 2
        tag, raw_path = raw.split("=", 1)
        path = Path(raw_path).resolve()
        if not path.is_file():
            print("姿态图不存在：%s" % path)
            return 2
        poses.append((tag.strip(), path))
    if not poses and not args.video:
        print("至少需要一张姿态图或一个视频序列")
        return 2
    video_path = Path(args.video).resolve() if args.video else None
    if video_path and not video_path.is_file():
        print("视频序列不存在：%s" % video_path)
        return 2

    module = load_cutout_module()
    started = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    results = []
    overview_poses = []
    for tag, path in poses:
        checks, diagnostics, cutout, compare = evaluate(
            str(reference), tag, path, module, out_dir)
        print("%s | %s" % (tag, path.name))
        for title, ok, detail in checks:
            print("  [%s] %s — %s" % (
                "通过" if ok else "不通过", title, detail))
        print("  诊断：背景色距 %.1f，脚部底边距 %d px" % (
            diagnostics["background_drift"],
            diagnostics["bottom_margin_px"]))
        results.append({
            "tag": tag, "source": str(path), "checks": checks,
            "diagnostics": diagnostics, "cutout": str(cutout),
            "compare": str(compare),
        })
        overview_poses.append((tag, path, cutout))

    if overview_poses:
        overview = build_overview(str(reference), overview_poses, out_dir)
        print("总览：%s" % overview)

    video = None
    if video_path:
        video_checks, video_rows, sizes = evaluate_video(
            str(reference), str(video_path), module, out_dir)
        print("视频补间 | %s（%d 帧）" % (video_path.name, len(video_rows)))
        for title, ok, detail in video_checks:
            print("  [%s] %s — %s" % (
                "通过" if ok else "不通过", title, detail))
        video = (video_checks, video_rows, sizes, str(video_path))

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    trace = _TRACES / ("精灵姿态序列验收-%s.md" % stamp)
    passed, total = write_trace(trace, started, str(reference),
                                results, video)
    print("汇总：%d/%d 通过，留痕 %s" % (passed, total, trace))
    return 0 if passed == total else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as exc:
        print("解析/运行异常：%r" % exc)
        sys.exit(2)
