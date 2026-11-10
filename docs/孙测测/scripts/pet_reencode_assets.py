#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：按 `src/assets/pet/manifest.json` 重编码桌宠素材——裁掉 alpha 透明边、
         按 frameOrder 物化往返帧（消除 6 帧硬循环跳帧）、调色板量化、
         lossless 动画 WebP 编码。`--check` 用纯标准库 RIFF 校验产物契约，
         不依赖 Pillow，可直接进 CI。
适用项目：SUGT（桌面精灵 小酥）
创建日期：2026-11-09
复用方式：
  - 编码（需 Pillow ≥10 且支持 webp_anim）：
      /usr/local/bin/python3.13 docs/孙测测/scripts/pet_reencode_assets.py \
          --source <原始逐帧素材目录> [--colors 56]（省略则取 manifest.encoding.colors）
  - 校验（纯标准库，CI 可用）：
      python3 docs/孙测测/scripts/pet_reencode_assets.py --check
  - 未来 1000 帧生产：把 ComfyUI 输出的 RGBA 序列帧目录传给 --source，
    manifest 的 frameOrder/frameMs 改为真实关键帧清单即可复用同一条编码链。
退出码约定：0 全部通过 / 1 契约不一致或编码失败 / 2 读取、解析或依赖缺失
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime
from pathlib import Path

MANIFEST_REL = Path("src") / "assets" / "pet" / "manifest.json"
ASSET_DIR_REL = Path("src") / "assets" / "pet"
TRACE_REL = Path("docs") / "孙测测" / "traces"


def parse_args() -> argparse.Namespace:
    """解析参数；默认项目根由脚本位置推导，不硬编码本机绝对路径。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(description="桌宠素材裁剪 / 往返帧 / 调色板重编码")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--source", type=Path, default=None,
                        help="原始逐帧素材目录（编码模式必填，校验模式忽略）")
    parser.add_argument("--colors", type=int, default=None,
                        help="覆盖 manifest.encoding.colors（默认取清单值）")
    parser.add_argument("--check", action="store_true", help="只校验产物契约，不写文件")
    parser.add_argument("--out", type=Path, default=None, help="自定留痕路径")
    return parser.parse_args()


def load_manifest(root: Path) -> dict:
    """读取并校验 manifest v2 的结构，非法结构直接判为解析失败。"""
    data = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("formatVersion") != 2:
        raise ValueError("manifest.formatVersion 必须为 2")
    base = data.get("baseSize") or {}
    crop = data.get("cropBox")
    enc = data.get("encoding") or {}
    if not isinstance(base.get("w"), int) or not isinstance(base.get("h"), int):
        raise ValueError("manifest.baseSize 缺少整数宽高")
    if not isinstance(crop, list) or len(crop) != 4:
        raise ValueError("manifest.cropBox 必须是 4 元数组")
    if crop[2] - crop[0] != base["w"] or crop[3] - crop[1] != base["h"]:
        raise ValueError("cropBox 尺寸与 baseSize 不一致")
    if enc.get("mode") != "palette-lossless":
        raise ValueError("encoding.mode 必须为 palette-lossless")
    poses = data.get("poses")
    if not isinstance(poses, list) or not poses:
        raise ValueError("manifest.poses 必须是非空数组")
    for pose in poses:
        for key in ("name", "poster", "loop", "frames", "frameOrder", "frameMs"):
            if key not in pose:
                raise ValueError(f"姿势 {pose.get('name')} 缺字段 {key}")
        if not isinstance(pose["frameMs"], list):
            raise ValueError(f"姿势 {pose['name']} frameMs 必须是逐帧数组")
        if len(pose["frameOrder"]) != pose["frames"] or len(pose["frameMs"]) != pose["frames"]:
            raise ValueError(f"姿势 {pose['name']} frameOrder/frameMs 长度与 frames 不符")
        if any(not isinstance(v, int) or v <= 0 for v in pose["frameMs"]):
            raise ValueError(f"姿势 {pose['name']} frameMs 含非法值")
    return data


def probe_webp(path: Path) -> dict:
    """纯标准库解析 RIFF：尺寸、帧数、逐帧时长、alpha/animation 标志。

    支持 VP8X（动画容器）+ ANMF 与单帧 VP8L（lossless）。
    """
    data = path.read_bytes()
    if data[:4] != b"RIFF" or data[8:12] != b"WEBP":
        raise ValueError(f"不是 WebP: {path.name}")
    flags = 0
    width = height = 0
    durations: list[int] = []
    offset = 12
    while offset + 8 <= len(data):
        tag = data[offset:offset + 4]
        size = int.from_bytes(data[offset + 4:offset + 8], "little")
        body = data[offset + 8:offset + 8 + size]
        if tag == b"VP8X" and size >= 10:
            flags = body[0]
            width = 1 + int.from_bytes(body[4:7], "little")
            height = 1 + int.from_bytes(body[7:10], "little")
        elif tag == b"VP8L" and size >= 5 and not width:
            bits = int.from_bytes(body[1:5], "little")
            width = (bits & 0x3FFF) + 1
            height = ((bits >> 14) & 0x3FFF) + 1
            flags |= 0x10 if (bits >> 28) & 1 else 0
        elif tag == b"ANMF" and size >= 16:
            durations.append(int.from_bytes(body[12:15], "little"))
        offset += 8 + size + (size & 1)
    return {
        "w": width,
        "h": height,
        "frames": len(durations) or 1,
        "durations": durations,
        "alpha": bool(flags & 0x10),
        "animation": bool(flags & 0x02),
        "bytes": len(data),
    }


def check_products(root: Path, manifest: dict) -> tuple[list[tuple[str, bool, str]], dict]:
    """校验落包素材与 manifest 的全部契约项；返回逐项结论与体积统计。"""
    rows: list[tuple[str, bool, str]] = []
    asset_dir = root / ASSET_DIR_REL
    w, h = manifest["baseSize"]["w"], manifest["baseSize"]["h"]
    problems: list[str] = []
    total = 0
    frame_total = 0
    for pose in manifest["poses"]:
        name = pose["name"]
        loop = asset_dir / pose["loop"]
        poster = asset_dir / pose["poster"]
        if not loop.is_file() or not poster.is_file():
            problems.append(f"{name} 素材缺失")
            continue
        info = probe_webp(loop)
        if (info["w"], info["h"]) != (w, h):
            problems.append(f"{name} loop 尺寸 {info['w']}x{info['h']} ≠ {w}x{h}")
        if info["frames"] != pose["frames"]:
            problems.append(f"{name} 帧数 {info['frames']} ≠ {pose['frames']}")
        if info["durations"] != pose["frameMs"]:
            problems.append(f"{name} 逐帧时长与 frameMs 不符")
        if not info["alpha"] or not info["animation"]:
            problems.append(f"{name} 缺 alpha/animation 标志")
        pin = probe_webp(poster)
        if (pin["w"], pin["h"]) != (w, h):
            problems.append(f"{name} poster 尺寸 {pin['w']}x{pin['h']} ≠ {w}x{h}")
        total += info["bytes"] + pin["bytes"]
        frame_total += info["frames"]
    for extra in manifest.get("extraFrames", []):
        p = asset_dir / extra
        if not p.is_file():
            problems.append(f"缺附加帧 {extra}")
            continue
        info = probe_webp(p)
        if (info["w"], info["h"]) != (w, h):
            problems.append(f"{extra} 尺寸 {info['w']}x{info['h']} ≠ {w}x{h}")
        total += info["bytes"]
    rows.append(("素材尺寸 / 帧数 / 逐帧时长 / alpha 与 manifest 一致",
                 not problems, "、".join(problems[:4]) if problems else
                 f"{len(manifest['poses'])} 个姿势 + {len(manifest.get('extraFrames', []))} 附加帧全部一致"))
    stats = {
        "total_bytes": total,
        "frame_total": frame_total,
        "per_frame": round(total / frame_total) if frame_total else 0,
        "projected_1000_mb": round(total / frame_total * 1000 / 1e6, 2) if frame_total else 0,
    }
    return rows, stats


def encode_products(root: Path, source: Path, manifest: dict,
                    colors: int) -> list[tuple[str, bool, str]]:
    """按 manifest 裁剪 / 物化往返帧 / 量化 / lossless 编码，覆盖落包素材。"""
    try:
        from PIL import Image, features
    except ImportError as exc:
        raise RuntimeError(
            "编码模式需要 Pillow≥10 且支持 webp_anim；"
            "请改用带 Pillow 的解释器（本机 /usr/local/bin/python3.13），"
            "或只跑 --check（纯标准库）"
        ) from exc
    if not features.check("webp") or not features.check("webp_anim"):
        raise RuntimeError("当前 Pillow 缺少 WebP / 动画 WebP 支持")

    asset_dir = root / ASSET_DIR_REL
    crop = tuple(manifest["cropBox"])
    method = int(manifest["encoding"].get("method", 4))
    rows: list[tuple[str, bool, str]] = []
    before = after = 0

    def load_frames(path: Path) -> list:
        """从源素材读出全部 RGBA 帧并裁到统一框。"""
        im = Image.open(path)
        frames = []
        for index in range(getattr(im, "n_frames", 1)):
            im.seek(index)
            frames.append(im.convert("RGBA").crop(crop))
        return frames

    def quantize(frame):
        return frame.quantize(colors=colors, method=Image.Quantize.FASTOCTREE,
                              dither=Image.Dither.NONE)

    def write_single(frame, dst: Path) -> None:
        quantize(frame).save(dst, format="WEBP", lossless=True, method=method)

    for pose in manifest["poses"]:
        name = pose["name"]
        src_loop = source / pose["loop"]
        src_poster = source / pose["poster"]
        if not src_loop.is_file() or not src_poster.is_file():
            raise FileNotFoundError(f"{name} 源素材缺失于 {source}")
        frames = load_frames(src_loop)
        expected = int(pose.get("sourceFrames", len(frames)))
        if len(frames) != expected:
            raise ValueError(f"{name} 源帧数 {len(frames)} ≠ manifest.sourceFrames {expected}")
        order = pose["frameOrder"]
        if any(i < 0 or i >= len(frames) for i in order):
            raise ValueError(f"{name} frameOrder 越界")
        ordered = [frames[i] for i in order]
        dst_loop = asset_dir / pose["loop"]
        dst_poster = asset_dir / pose["poster"]
        before += src_loop.stat().st_size + src_poster.stat().st_size
        qframes = [quantize(f) for f in ordered]
        qframes[0].save(dst_loop, format="WEBP", save_all=True,
                        append_images=qframes[1:], duration=pose["frameMs"],
                        loop=0, lossless=True, method=method)
        write_single(load_frames(src_poster)[0], dst_poster)
        after += dst_loop.stat().st_size + dst_poster.stat().st_size
        rows.append((f"{name} 重编码", True,
                     f"{len(ordered)} 帧 / {sum(pose['frameMs'])}ms / "
                     f"{dst_loop.stat().st_size} B"))

    for extra in manifest.get("extraFrames", []):
        src = source / extra
        dst = asset_dir / extra
        if not src.is_file():
            raise FileNotFoundError(f"附加帧源素材缺失: {extra}")
        before += src.stat().st_size
        write_single(load_frames(src)[0], dst)
        after += dst.stat().st_size
        rows.append((f"{extra} 重编码", True, f"{dst.stat().st_size} B"))

    rows.insert(0, ("编码参数", True,
                    f"palette-lossless / {colors} 色 / method={method} / "
                    f"crop={crop} / 源目录 {source}"))
    rows.insert(1, ("素材体积变化", True,
                    f"{before} B → {after} B（{100 * after / before:.1f}%）"))
    return rows


def write_trace(root: Path, out: Path | None, rows: list[tuple[str, bool, str]],
                mode: str, stats: dict) -> Path:
    """每次执行写一份 Markdown 留痕，含逐项结论、体积模型与复核命令。"""
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    label = "产物校验" if mode == "--check（只校验）" else "素材重编码"
    path = (out or root / TRACE_REL / f"精灵{label}-{stamp}.md").resolve()
    lines = [
        f"# 精灵素材{label}留痕",
        "",
        f"- 执行时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        f"- 模式：{mode}",
        "- 真相源：`src/assets/pet/manifest.json`（v2）",
        f"- 结论：{'通过' if all(row[1] for row in rows) else '不通过'}",
        "",
        "| 检查项 | 结果 | 证据 |",
        "| --- | --- | --- |",
    ]
    lines += [f"| {name} | {'通过' if ok else '不通过'} | {detail} |"
              for name, ok, detail in rows]
    if stats:
        lines += [
            "",
            "## 体积模型（外推到 1000 帧）",
            "",
            f"- 落包素材合计：{stats['total_bytes']} B（{stats['total_bytes'] / 1024:.1f} KB）",
            f"- 动画帧合计：{stats['frame_total']} 帧；折算单帧 {stats['per_frame']} B",
            f"- 1000 帧线性外推：约 {stats['projected_1000_mb']} MB",
            "- 外推口径：按当前编码策略与画布尺寸的均值，真动作帧差异更大时需留 15% 余量。",
        ]
    lines += [
        "",
        "## 复核命令",
        "",
        "```bash",
        "python3 docs/孙测测/scripts/pet_reencode_assets.py --check",
        "python3 docs/孙测测/scripts/gen_pet_from_manifest.py --check",
        "python3 docs/孙测测/scripts/check_pet_overlay.py",
        "npm test && npm run build",
        "```",
        "",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main() -> int:
    """入口：读清单 → 校验或编码 → 留痕 → 退出码。"""
    args = parse_args()
    root = args.root.resolve()
    mode = "--check（只校验）" if args.check else "编码（写素材）"
    stats: dict = {}
    try:
        manifest = load_manifest(root)
        if args.check:
            rows, stats = check_products(root, manifest)
            rows.insert(0, ("cropBox 与 baseSize 自洽", True,
                            f"{tuple(manifest['cropBox'])} → "
                            f"{manifest['baseSize']['w']}x{manifest['baseSize']['h']}"))
        else:
            if args.source is None:
                print("[失败] 编码模式必须传 --source <原始逐帧素材目录>", file=sys.stderr)
                return 2
            source = args.source.resolve()
            if not source.is_dir():
                print(f"[失败] 源目录不存在: {source}", file=sys.stderr)
                return 2
            colors = args.colors or int(manifest["encoding"]["colors"])
            rows = encode_products(root, source, manifest, colors)
            check_rows, stats = check_products(root, manifest)
            rows.append(("重编码产物自检", check_rows[0][1], check_rows[0][2]))
    except (OSError, ValueError, json.JSONDecodeError, RuntimeError) as exc:
        print(f"[失败] 读取、解析或编码异常: {exc}", file=sys.stderr)
        return 2

    trace = write_trace(root, args.out, rows, mode, stats)
    for name, ok, detail in rows:
        print(f"[{'通过' if ok else '不通过'}] {name} — {detail}")
    failed = [row for row in rows if not row[1]]
    if stats:
        print(f"体积：{stats['total_bytes']} B / {stats['frame_total']} 帧 / "
              f"单帧 {stats['per_frame']} B / 1000 帧约 {stats['projected_1000_mb']} MB")
    print(f"汇总：{len(rows) - len(failed)}/{len(rows)} 通过，留痕 {trace}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
