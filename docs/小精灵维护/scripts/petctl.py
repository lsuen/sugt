#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-021：桌面精灵（小酥）维护目录统一入口。

编写人员：孙测测
作用用途：
  把 v1.3 定版生产链、验收链、环境自检、副本同步、一键回归收敛成一条命令。
  本目录 scripts/ 下的 10 个脚本是 docs/孙测测/scripts/ 定版脚本的只读副本，
  两者目录层级同为 docs/<目录>/scripts/，因此脚本内按 __file__ 推导的项目根、
  留痕目录、master 路径全部保持一致，副本可直接执行。
  doctor 会逐项核对副本与原件 sha256，防止出现「两份脚本不同步」的隐形分叉。
适用项目：SUGT 桌面精灵 v1.3.1（20 动作 / 1000 帧 / 180×297 / 56 色）
创建日期：2026-11-10
复用方式：
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py doctor
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py all
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py produce --phase keys --action lifted
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py gen --check
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py sync
  除 doctor / sync / snapshot / all 外，其余子命令的额外参数原样透传给对应脚本。
退出码约定：0 全部成功 / 1 存在检查或执行不通过 / 2 环境、读取或解析失败
"""
from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parents[2]
_ORIGINAL_SCRIPTS = _ROOT / "docs" / "孙测测" / "scripts"
_CONFIG_SNAPSHOT = _HERE.parent / "config" / "manifest.json"
_MASTER_SNAPSHOT = _HERE.parent / "assets" / "su-cel-good-fixed.png"
_MANIFEST = _ROOT / "src" / "assets" / "pet" / "manifest.json"
_MASTER = _ROOT / "docs" / "孙测测" / "traces" / "assets" / "su-cel-good-fixed.png"
_PET_ASSETS = _ROOT / "src" / "assets" / "pet"
_FINAL_SNAPSHOT = _HERE.parent / "assets" / "final"

# 定版脚本白名单：只收录 v1.3 最终链路，试片 / 废弃脚本一律不进维护目录。
FINAL_SCRIPTS = (
    "comfyui_curl_http.py",
    "comfyui_pet_action_series.py",
    "pet_reencode_assets.py",
    "gen_pet_from_manifest.py",
    "build_pet_preview.py",
    "check_pet_overlay.py",
    "check_pet_final_frames.py",
    "check_pet_pose_series.py",
    "check_pet_keyvisual_cutout.py",
    "check_pet_cutout_asset.py",
)

# 子命令 → 定版脚本。透传型命令的参数由调用方追加。
COMMANDS = {
    "gen": "gen_pet_from_manifest.py",
    "preview": "build_pet_preview.py",
    "reencode": "pet_reencode_assets.py",
    "check": "pet_reencode_assets.py",
    "frames": "check_pet_final_frames.py",
    "gate": "check_pet_overlay.py",
    "series": "check_pet_pose_series.py",
    "cutout": "check_pet_keyvisual_cutout.py",
    "asset": "check_pet_cutout_asset.py",
    "produce": "comfyui_pet_action_series.py",
}

# 一键回归顺序：gate 依赖 dist 产物，必须排在 npm run build 之后。
ALL_STEPS = (
    ("生成 manifest 派生常量", ("gen",)),
    ("生成 v1.3 互动预览页", ("preview",)),
    ("产物契约自检", ("check", "--check")),
    ("落包 1000 帧逐帧验收", ("frames",)),
    ("前端单测", ("@npm", "test")),
    ("生产构建", ("@npm", "run", "build")),
    ("全链路契约门禁", ("gate",)),
)


def sha256(path: Path) -> str:
    """返回文件 sha256；用于副本与原件的一致性核对。"""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_script(name: str, extra: list[str]) -> int:
    """以当前 Python 解释器执行定版脚本副本，工作目录固定为项目根。"""
    target = _HERE / name
    if not target.is_file():
        print("[失败] 缺定版脚本副本：%s（先执行 sync）" % target, file=sys.stderr)
        return 2
    print("\n>>> %s %s" % (name, " ".join(extra)))
    try:
        return subprocess.run(
            [sys.executable, str(target), *extra], cwd=str(_ROOT)
        ).returncode
    except OSError as exc:
        print("[失败] 无法启动脚本：%s" % exc, file=sys.stderr)
        return 2


def do_sync() -> int:
    """单向同步：docs/孙测测/scripts/ 是编辑源，本目录副本只读。"""
    changed = 0
    for name in FINAL_SCRIPTS:
        source = _ORIGINAL_SCRIPTS / name
        target = _HERE / name
        if not source.is_file():
            print("[失败] 原件缺失：%s" % source, file=sys.stderr)
            return 2
        if target.is_file() and sha256(source) == sha256(target):
            continue
        shutil.copy2(source, target)
        print("[同步] %s" % name)
        changed += 1
    print("同步完成：%d 个脚本有变更，%d 个已一致。" % (changed, len(FINAL_SCRIPTS) - changed))
    return 0


def do_snapshot() -> int:
    """把当前进包的 41 张 WebP 复制到维护目录存档，不影响唯一进包路径。"""
    files = sorted(_PET_ASSETS.glob("*.webp"))
    if not files:
        print("[失败] src/assets/pet/ 无 WebP 素材", file=sys.stderr)
        return 2
    _FINAL_SNAPSHOT.mkdir(parents=True, exist_ok=True)
    for path in files:
        shutil.copy2(path, _FINAL_SNAPSHOT / path.name)
    total = sum(path.stat().st_size for path in files)
    print("快照完成：%d 张 / %d B = %.3f MB → %s"
          % (len(files), total, total / 1e6, _FINAL_SNAPSHOT))
    return 0


def do_doctor() -> int:
    """环境、副本一致性、配置快照、素材契约四项自检。"""
    ok = True

    def line(good: bool, label: str, detail: str = "") -> None:
        nonlocal ok
        print("[%s] %s%s" % ("通过" if good else "不通过", label,
                             (" — " + detail) if detail else ""))
        if not good:
            ok = False

    print("项目根：%s" % _ROOT)
    print("Python：%s" % sys.executable)
    try:
        from PIL import Image  # noqa: F401
        line(True, "Pillow 可用")
    except ImportError:
        line(False, "Pillow 可用", "请用 /usr/local/bin/python3.13 执行")
        return 2

    missing = [name for name in FINAL_SCRIPTS
               if not (_HERE / name).is_file()]
    line(not missing, "定版脚本副本齐全（%d 个）" % len(FINAL_SCRIPTS),
         "缺 %s" % ", ".join(missing) if missing else "")
    drift = []
    for name in FINAL_SCRIPTS:
        source, target = _ORIGINAL_SCRIPTS / name, _HERE / name
        if source.is_file() and target.is_file() and sha256(source) != sha256(target):
            drift.append(name)
    line(not drift, "副本与 docs/孙测测/scripts/ 原件 sha256 一致",
         "漂移：%s（执行 sync）" % ", ".join(drift) if drift else "")

    line(_MANIFEST.is_file(), "manifest.json 存在")
    line(_CONFIG_SNAPSHOT.is_file()
         and _MANIFEST.is_file()
         and sha256(_CONFIG_SNAPSHOT) == sha256(_MANIFEST),
         "配置快照与现行 manifest 一致",
         "不一致时重跑：cp src/assets/pet/manifest.json docs/小精灵维护/config/manifest.json")
    line(_MASTER.is_file(), "master 定稿存在（SCR-017 修补版）", str(_MASTER))
    line(_MASTER_SNAPSHOT.is_file()
         and _MASTER.is_file()
         and sha256(_MASTER_SNAPSHOT) == sha256(_MASTER),
         "本目录 master 基准图与生产输入一致")

    files = sorted(_PET_ASSETS.glob("*.webp"))
    total = sum(path.stat().st_size for path in files)
    line(len(files) == 41, "进包素材 41 张", "实际 %d 张" % len(files))
    line(total <= 8 * 1024 * 1024, "整包 ≤ 8 MB",
         "%d B = %.3f MB" % (total, total / 1e6))
    line(bool(shutil.which("npm")), "npm 可用（前端单测 / 构建）")

    work = os.environ.get("SUGT_PET_WORK")
    if work:
        line(Path(work).is_dir(), "生产工作目录 SUGT_PET_WORK 存在", work)
    else:
        print("[提示] 未设置 SUGT_PET_WORK；重生成 1000 帧前请指向生产工作目录")
    base = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
    print("[提示] ComfyUI 地址：%s（仅 produce 需要，doctor 不探测服务）" % base)

    print("\n结论：%s" % ("全部通过，可维护" if ok else "存在不通过项，先修复再维护"))
    return 0 if ok else 1


def do_all(extra: list[str]) -> int:
    """按上线前回归顺序一键执行；任一环节失败立即中断。"""
    for label, step in ALL_STEPS:
        print("\n=== %s ===" % label)
        if step[0] == "@npm":
            if not shutil.which("npm"):
                print("[失败] npm 不在 PATH", file=sys.stderr)
                return 2
            code = subprocess.run(["npm", *step[1:]], cwd=str(_ROOT)).returncode
        else:
            code = run_script(COMMANDS[step[0]], list(step[1:]) + extra)
        if code != 0:
            print("\n[中断] %s 返回 %d，后续步骤未执行。" % (label, code), file=sys.stderr)
            return code
    print("\n=== 一键回归全部通过 ===")
    return 0


def main() -> int:
    """解析子命令并调度；未知参数透传给被调脚本。"""
    parser = argparse.ArgumentParser(
        description="小精灵 v1.3 维护统一入口",
        usage="petctl.py <命令> [原脚本参数 ...]",
        epilog="命令：doctor / gen / preview / reencode / check / frames / gate / "
               "series / cutout / asset / produce / sync / snapshot / all",
    )
    parser.add_argument("command", help="子命令")
    args, extra = parser.parse_known_args()
    cmd = args.command

    if cmd == "doctor":
        return do_doctor()
    if cmd == "sync":
        return do_sync()
    if cmd == "snapshot":
        return do_snapshot()
    if cmd == "all":
        return do_all(extra)
    if cmd == "check":
        # check 固定为纯校验模式，避免误写素材。
        return run_script(COMMANDS[cmd], ["--check", *[a for a in extra if a != "--check"]])
    if cmd in COMMANDS:
        return run_script(COMMANDS[cmd], extra)

    parser.print_usage(sys.stderr)
    print("[失败] 未知子命令：%s" % cmd, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
