#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：以 `src/assets/pet/manifest.json`（v2）为唯一真相源，校验素材契约
         （文件存在、RIFF 尺寸、ANMF 帧数、逐帧时长数组、alpha 标志、逐帧 alpha 包围盒），
         并生成 `src/pet/pet.css` 的单层背景规则段与
         `src/pet/petManifest.generated.ts`。加姿势只需改 manifest 后重跑本脚本。
适用项目：SUGT（桌面精灵 小酥）
创建日期：2026-11-09
复用方式：python3 docs/孙测测/scripts/gen_pet_from_manifest.py [--root 项目根] [--check]；
         默认写模式会更新生成物；`--check` 只比对，生成物过期或素材漂移即失败。
         依赖系统命令 ffmpeg（与 pet_v07_frames.py 同源），不需要第三方库。
退出码约定：0 全部通过 / 1 契约或生成物不一致 / 2 读取、解析或解码失败
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime
from pathlib import Path

MANIFEST_REL = "src/assets/pet/manifest.json"
CSS_REL = "src/pet/pet.css"
TS_REL = "src/pet/petManifest.generated.ts"
CONSUMERS = (
    "docs/孙测测/scripts/build_pet_preview.py",
    "docs/孙测测/scripts/check_pet_overlay.py",
)
CSS_BEGIN = "/* @generated:pet-assets begin —— 由 gen_pet_from_manifest.py 生成，勿手工编辑 */"
CSS_END = "/* @generated:pet-assets end */"
ALPHA_THRESHOLD = 8
VALID_PLAYBACK = ("loop", "pingpong", "once", "blink")
VALID_MOTION = ("breathe", "bob", "float", "tilt", "wiggle", "shake", "wave", "hop")


def parse_args() -> argparse.Namespace:
    """解析参数，默认项目根由脚本位置推导，不硬编码本机绝对路径。"""
    default_root = Path(__file__).resolve().parents[3]
    parser = argparse.ArgumentParser(description="精灵素材契约生成与校验")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--check", action="store_true", help="只校验不写文件")
    parser.add_argument("--out", type=Path, default=None, help="自定留痕路径")
    return parser.parse_args()


def load_manifest(root: Path) -> dict:
    """读取并校验 manifest v2 的基本结构，非法结构直接判为解析失败。"""
    data = json.loads((root / MANIFEST_REL).read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("formatVersion") != 2:
        raise ValueError("manifest.formatVersion 必须为 2")
    base = data.get("baseSize") or {}
    if not isinstance(base.get("w"), int) or not isinstance(base.get("h"), int):
        raise ValueError("manifest.baseSize 缺少整数宽高")
    crop = data.get("cropBox")
    if not isinstance(crop, list) or len(crop) != 4:
        raise ValueError("manifest.cropBox 必须是 4 元数组")
    if crop[2] - crop[0] != base["w"] or crop[3] - crop[1] != base["h"]:
        raise ValueError("cropBox 尺寸与 baseSize 不一致")
    enc = data.get("encoding") or {}
    if enc.get("mode") != "palette-lossless":
        raise ValueError("encoding.mode 必须为 palette-lossless")
    poses = data.get("poses")
    if not isinstance(poses, list) or not poses:
        raise ValueError("manifest.poses 必须是非空数组")
    names = set()
    for pose in poses:
        for key in ("name", "poster", "loop", "frames", "sourceFrames",
                    "frameOrder", "frameMs", "playback", "motion", "hitBox"):
            if key not in pose:
                raise ValueError(f"姿势 {pose.get('name')} 缺字段 {key}")
        if pose["name"] in names:
            raise ValueError(f"姿势重复: {pose['name']}")
        names.add(pose["name"])
        if pose["playback"] not in VALID_PLAYBACK:
            raise ValueError(f"姿势 {pose['name']} playback 非法: {pose['playback']}")
        if pose["motion"] not in VALID_MOTION:
            raise ValueError(f"姿势 {pose['name']} motion 非法: {pose['motion']}")
        if not isinstance(pose["frameMs"], list):
            raise ValueError(f"姿势 {pose['name']} frameMs 必须是逐帧数组")
        if len(pose["frameOrder"]) != pose["frames"] or len(pose["frameMs"]) != pose["frames"]:
            raise ValueError(f"姿势 {pose['name']} frameOrder/frameMs 长度与 frames 不符")
        if any(i < 0 or i >= pose["sourceFrames"] for i in pose["frameOrder"]):
            raise ValueError(f"姿势 {pose['name']} frameOrder 索引越界")
        if any(not isinstance(v, int) or v <= 0 for v in pose["frameMs"]):
            raise ValueError(f"姿势 {pose['name']} frameMs 含非法值")
        box = pose["hitBox"]
        if not isinstance(box, list) or len(box) != 4:
            raise ValueError(f"姿势 {pose['name']} hitBox 必须是 4 元数组")
    return data


def read_riff(path: Path) -> dict:
    """解析动画 WebP 顶层 RIFF，返回尺寸、帧数、逐帧时长、alpha/animation 标志。"""
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
        elif tag == b"ANMF" and size >= 16:
            durations.append(int.from_bytes(body[12:15], "little"))
        offset += 8 + size + (size & 1)
    return {
        "w": width,
        "h": height,
        "frames": len(durations),
        "durations": durations,
        "alpha": bool(flags & 0x10),
        "animation": bool(flags & 0x02),
    }


def alpha_bbox(path: Path, width: int, height: int) -> list[float]:
    """逐帧解码动画 WebP 的 alpha，取并集包围盒并归一化到 [0,1]。

    离线预计算：运行时不碰 getImageData，透明边距不会变成「透明点击墙」。
    并集而非首帧，避免动作幅度大的姿势在某些帧里被判定为穿透。
    """
    result = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-vf", "alphaextract",
         "-f", "rawvideo", "-pix_fmt", "gray", "-"],
        capture_output=True, check=False,
    )
    if result.returncode != 0:
        raise ValueError(f"ffmpeg 解码失败 {path.name}: "
                         f"{result.stderr.decode('utf-8', 'replace')[:160]}")
    frame_size = width * height
    raw = result.stdout
    if len(raw) < frame_size or len(raw) % frame_size:
        raise ValueError(f"alpha 数据长度异常 {path.name}: {len(raw)} B")
    x0, y0, x1, y1 = width, height, 0, 0
    for index in range(len(raw) // frame_size):
        frame = raw[index * frame_size:(index + 1) * frame_size]
        rows = [y for y in range(height)
                if max(frame[y * width:(y + 1) * width]) > ALPHA_THRESHOLD]
        if not rows:
            continue
        top, bottom = rows[0], rows[-1]
        cols = [x for x in range(width)
                if any(frame[y * width + x] > ALPHA_THRESHOLD
                       for y in range(top, bottom + 1))]
        if not cols:
            continue
        x0, y0 = min(x0, cols[0]), min(y0, top)
        x1, y1 = max(x1, cols[-1] + 1), max(y1, bottom + 1)
    if x1 <= x0 or y1 <= y0:
        raise ValueError(f"{path.name} 全帧无可见像素")
    return [round(value, 4) for value in
            (x0 / width, y0 / height, x1 / width, y1 / height)]


def inspect_assets(root: Path, manifest: dict) -> tuple[list[str], dict, list[str]]:
    """逐姿势核对素材文件、尺寸、帧数、逐帧时长与 hitBox。

    返回 (硬性问题, 实测包围盒, hitBox 漂移的姿势名)。写模式允许回填包围盒，
    但帧数、时长、alpha 这类素材本身的契约不允许被脚本静默改写。
    """
    problems: list[str] = []
    boxes: dict[str, list[float]] = {}
    box_diffs: list[str] = []
    asset_dir = root / "src" / "assets" / "pet"
    width, height = manifest["baseSize"]["w"], manifest["baseSize"]["h"]
    for pose in manifest["poses"]:
        name = pose["name"]
        for key in ("poster", "loop"):
            if not (asset_dir / pose[key]).is_file():
                problems.append(f"{name} 缺素材 {pose[key]}")
        loop_path = asset_dir / pose["loop"]
        if not loop_path.is_file():
            continue
        riff = read_riff(loop_path)
        if (riff["w"], riff["h"]) != (width, height):
            problems.append(f"{name} 尺寸 {riff['w']}x{riff['h']} ≠ {width}x{height}")
        if riff["frames"] != pose["frames"]:
            problems.append(f"{name} 帧数 {riff['frames']} ≠ manifest {pose['frames']}")
        if not riff["alpha"] or not riff["animation"]:
            problems.append(f"{name} 缺 alpha/animation 标志")
        if riff["durations"] != pose["frameMs"]:
            problems.append(f"{name} 逐帧时长与 frameMs 数组不符")
        boxes[name] = alpha_bbox(loop_path, width, height)
        if boxes[name] != [round(float(v), 4) for v in pose["hitBox"]]:
            box_diffs.append(name)
    for extra in manifest.get("extraFrames", []):
        if not (asset_dir / extra).is_file():
            problems.append(f"缺附加帧 {extra}")
    return problems, boxes, box_diffs


def build_css_block(poses: list[dict]) -> str:
    """生成每张图恰好一个 url 的单层背景规则（C17 契约不变）。"""
    lines = [CSS_BEGIN]
    for pose in poses:
        lines.append(
            f'.pet-loop[data-loop="{pose["name"]}"] '
            f'{{ background-image: url(\'../assets/pet/{pose["loop"]}\'); }}'
        )
    for pose in poses:
        lines.append(
            f'.pet-poster[data-frame="{pose["name"]}"] '
            f'{{ background-image: url(\'../assets/pet/{pose["poster"]}\'); }}'
        )
    lines.append(CSS_END)
    return "\n".join(lines) + "\n"


def js_object(value) -> str:
    """把 Python 值序列化为合法 TS 对象字面量（JSON 语法子集）。"""
    return json.dumps(value, ensure_ascii=False, separators=(", ", ": "))


def build_ts(manifest: dict) -> str:
    """生成前端可直接 import 的常量，避免运行时读 JSON 或硬编码姿势名。"""
    names = [pose["name"] for pose in manifest["poses"]]
    lines = [
        "// 由 docs/孙测测/scripts/gen_pet_from_manifest.py 从 manifest.json 生成，勿手工编辑。",
        "// 单一真相源：姿势清单、素材文件名、逐帧时长、播放模式、motion 与 alpha 命中框。",
        "export const PET_POSE_NAMES = [",
    ]
    lines.extend(f"  '{name}'," for name in names)
    lines += [
        "] as const;",
        "export type PetPoseName = (typeof PET_POSE_NAMES)[number];",
        "",
        f"export const PET_IMAGE_SIZE = "
        f"{{ w: {manifest['baseSize']['w']}, h: {manifest['baseSize']['h']} }} as const;",
        "",
        "export const PET_HITBOXES: "
        "Record<PetPoseName, readonly [number, number, number, number]> = {",
    ]
    for pose in manifest["poses"]:
        box = ", ".join(str(value) for value in pose["hitBox"])
        lines.append(f"  {pose['name']}: [{box}],")
    lines += [
        "};",
        "",
        "export const PET_ASSET_FILES: "
        "Record<PetPoseName, { poster: string; loop: string }> = {",
    ]
    for pose in manifest["poses"]:
        lines.append(f"  {pose['name']}: "
                     f"{{ poster: '{pose['poster']}', loop: '{pose['loop']}' }},")
    lines += [
        "};",
        "",
        "// 逐帧停留时长（ms）：支持非匀速节奏，长度与动画 WebP 的 ANMF 帧一一对应。",
        "export const PET_FRAME_MS: Record<PetPoseName, readonly number[]> = {",
    ]
    for pose in manifest["poses"]:
        lines.append(f"  {pose['name']}: {js_object(pose['frameMs'])},")
    lines += [
        "};",
        "",
        "export const PET_PLAYBACK: Record<PetPoseName, string> = {",
    ]
    for pose in manifest["poses"]:
        lines.append(f"  {pose['name']}: '{pose['playback']}',")
    lines += [
        "};",
        "",
        "export const PET_MOTION: Record<PetPoseName, string> = {",
    ]
    for pose in manifest["poses"]:
        lines.append(f"  {pose['name']}: '{pose['motion']}',")
    lines += [
        "};",
        "",
        "// 自动交互节奏：语义层常量，代码级固定，不进设置页（参考 whale-girl 零打扰原则）。",
        f"export const PET_AUTO_ACTIONS = {js_object(manifest.get('autoActions', {}))} as const;",
        "",
        f"export const PET_STATE_PRIORITY = "
        f"{js_object(manifest.get('statePriority', []))} as const;",
        "",
        f"export const PET_POSE_PRIORITY = "
        f"{js_object(manifest.get('posePriority', []))} as const;",
        "",
    ]
    return "\n".join(lines)


def replace_css_block(css: str, block: str) -> str:
    """只替换生成标记之间的内容，手写样式一律保留。"""
    start = css.find(CSS_BEGIN)
    end = css.find(CSS_END)
    if start < 0 or end < 0 or end < start:
        raise ValueError("pet.css 缺少生成标记，无法安全替换")
    end += len(CSS_END)
    return css[:start] + block.rstrip("\n") + css[end:]


def check_consumers(root: Path) -> list[str]:
    """校验两个下游脚本确实从 manifest 取常量，杜绝「又硬编码一份」。"""
    problems = []
    for rel in CONSUMERS:
        text = (root / rel).read_text(encoding="utf-8")
        if "load_pet_manifest" not in text:
            problems.append(f"{rel} 未从 manifest 读取常量")
    return problems


def expected_outputs(root: Path, manifest: dict, boxes: dict) -> dict:
    """返回所有生成物的目标路径与期望内容。"""
    css = (root / CSS_REL).read_text(encoding="utf-8")
    return {
        root / CSS_REL: replace_css_block(css, build_css_block(manifest["poses"])),
        root / TS_REL: build_ts(manifest),
    }


def write_trace(root: Path, out: Path | None, rows: list[tuple[str, bool, str]],
                mode: str) -> Path:
    """每次执行写一份 Markdown 留痕，含逐项结论与复核命令。"""
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = (out or root / "docs" / "孙测测" / "traces"
            / f"精灵Manifest生成物校验-{stamp}.md").resolve()
    lines = [
        "# 精灵 Manifest 生成物校验留痕",
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
    lines += [
        "",
        "## 复核命令",
        "",
        "```bash",
        "python3 docs/孙测测/scripts/pet_reencode_assets.py --check",
        "python3 docs/孙测测/scripts/gen_pet_from_manifest.py --check",
        "python3 docs/孙测测/scripts/build_pet_preview.py",
        "python3 docs/孙测测/scripts/check_pet_overlay.py",
        "npm test && npm run build",
        "```",
        "",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main() -> int:
    """入口：读 manifest → 核对素材 → 比对/写生成物 → 留痕 → 退出码。"""
    args = parse_args()
    root = args.root.resolve()
    mode = "--check（只校验）" if args.check else "写模式（更新生成物）"
    rows: list[tuple[str, bool, str]] = []
    try:
        manifest = load_manifest(root)
        problems, boxes, box_diffs = inspect_assets(root, manifest)
        consumer_problems = check_consumers(root)
        targets = expected_outputs(root, manifest, boxes)
    except (OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        print(f"[失败] 读取或解析异常: {exc}", file=sys.stderr)
        return 2

    check_problems = problems + [f"{name} hitBox 漂移" for name in box_diffs]
    rows.append(("素材尺寸/帧数/逐帧时长/alpha/文件与 manifest 一致",
                 not problems, "、".join(problems[:4]) if problems else
                 f"{len(manifest['poses'])} 个姿势全部一致"))
    if args.check:
        rows.append(("manifest hitBox 与素材实测一致",
                     not box_diffs,
                     "、".join(box_diffs) if box_diffs else "全部一致"))
    rows.append(("下游脚本从 manifest 取常量",
                 not consumer_problems,
                 "、".join(consumer_problems) if consumer_problems else "2/2 命中"))

    if args.check:
        for path, expected in targets.items():
            actual = path.read_text(encoding="utf-8")
            ok = actual == expected
            rows.append((f"生成物未过期 {path.relative_to(root).as_posix()}",
                         ok, "内容一致" if ok else "与 manifest 不一致，请重跑写模式"))
    else:
        # 写模式先把实测 hitBox 回填，再按回填后的 manifest 重算生成物。
        hitbox_changed = False
        for pose in manifest["poses"]:
            if pose["hitBox"] != boxes[pose["name"]]:
                pose["hitBox"] = boxes[pose["name"]]
                hitbox_changed = True
        if hitbox_changed:
            (root / MANIFEST_REL).write_text(
                json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8")
        targets = expected_outputs(root, manifest, boxes)
        for path, expected in targets.items():
            actual = path.read_text(encoding="utf-8") if path.exists() else ""
            if actual != expected:
                path.write_text(expected, encoding="utf-8")
            rows.append((f"生成物已同步 {path.relative_to(root).as_posix()}",
                         True, "已写入" if actual != expected else "无需改动"))
        rows.insert(0, ("manifest hitBox 回填实测值", True,
                        "已更新" if hitbox_changed else "原本一致"))

    trace = write_trace(root, args.out, rows, mode)
    for name, ok, detail in rows:
        print(f"[{'通过' if ok else '不通过'}] {name} — {detail}")
    failed = [row for row in rows if not row[1]]
    print(f"汇总：{len(rows) - len(failed)}/{len(rows)} 通过，留痕 {trace}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
