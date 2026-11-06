# -*- coding: utf-8 -*-
"""
脚本名称：check_version_consistency.py
编写人员：孙测测
作用用途：校验 SUGT 项目六处文件、七个版本键是否一致，并输出可追溯的
          Markdown 留痕。
          用于上线前评估测试的「配置管理」检查项，防止版本号漂移导致
          测试报告无法追溯到具体构建。
适用项目：SUGT（Tauri 2 + React + Rust/Axum）
创建日期：2026-11-06
复用方式：python3 check_version_consistency.py --root <项目根> [--expect <版本>]
退出码约定：0 一致 / 1 不一致 / 2 读取或解析失败
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import NamedTuple


# 六处文件、七个版本键：展示名、相对项目根的路径、解析方式
# package-lock.json 顶层 version 与 packages[""].version 是两个独立键，
# src/release-notes.ts 的 CURRENT_VERSION 用于界面展示，漏改会造成版本漂移。
VERSION_SOURCES = (
    ("package.json", "package.json", "json"),
    ("package-lock.json (顶层)", "package-lock.json", "json"),
    ("package-lock.json (packages[''])", "package-lock.json", "npm_lock_root"),
    ("src/release-notes.ts", "src/release-notes.ts", "release_notes"),
    ("src-tauri/Cargo.toml", "src-tauri/Cargo.toml", "cargo_toml"),
    ("src-tauri/Cargo.lock", "src-tauri/Cargo.lock", "cargo_lock"),
    ("src-tauri/tauri.conf.json", "src-tauri/tauri.conf.json", "json"),
)


class VersionResult(NamedTuple):
    """单处版本号的读取结果，便于统一渲染与判定。"""

    source: str
    version: str
    error: str


def read_json_version(path: Path) -> str:
    """从 JSON 配置文件读取顶层 version 字段。"""
    with path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    version = payload.get("version")
    if not isinstance(version, str) or not version:
        raise ValueError("顶层 version 字段缺失或不是非空字符串")
    return version


def read_cargo_toml_version(path: Path) -> str:
    """只读取 [package] 段内的 version，避免误取依赖声明的版本。"""
    in_package = False
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            stripped = line.strip()
            # 遇到新段标题时切换状态，段外 version 一律忽略
            if stripped.startswith("[") and stripped.endswith("]"):
                in_package = stripped == "[package]"
                continue
            if in_package and stripped.startswith("version"):
                matched = re.match(r'^version\s*=\s*"([^"]+)"', stripped)
                if matched:
                    return matched.group(1)
    raise ValueError("[package] 段内未找到 version 字段")


def read_cargo_lock_version(path: Path) -> str:
    """按 [[package]] 分块定位 name = "sugt" 的包版本。"""
    content = path.read_text(encoding="utf-8")
    for block in content.split("[[package]]")[1:]:
        name_matched = re.search(
            r'^name\s*=\s*"([^"]+)"', block, re.MULTILINE
        )
        if not name_matched or name_matched.group(1) != "sugt":
            continue
        version_matched = re.search(
            r'^version\s*=\s*"([^"]+)"', block, re.MULTILINE
        )
        if version_matched:
            return version_matched.group(1)
    raise ValueError('Cargo.lock 中未找到 name = "sugt" 的包版本')


def read_npm_lock_root_version(path: Path) -> str:
    """读取 package-lock.json 中 packages[""] 的项目版本。"""
    with path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    version = payload.get("packages", {}).get("", {}).get("version")
    if not isinstance(version, str) or not version:
        raise ValueError('packages[""].version 缺失或不是非空字符串')
    return version


def read_release_notes_version(path: Path) -> str:
    """读取 src/release-notes.ts 中导出的 CURRENT_VERSION。"""
    content = path.read_text(encoding="utf-8")
    matched = re.search(
        r'^export\s+const\s+CURRENT_VERSION\s*=\s*[\'"](\d+\.\d+\.\d+)[\'"]',
        content,
        re.MULTILINE,
    )
    if not matched:
        raise ValueError("未找到 export const CURRENT_VERSION = 'x.y.z'")
    return matched.group(1)


# 解析方式到读取函数的映射，新增来源时只需扩展此表
READERS = {
    "json": read_json_version,
    "npm_lock_root": read_npm_lock_root_version,
    "release_notes": read_release_notes_version,
    "cargo_toml": read_cargo_toml_version,
    "cargo_lock": read_cargo_lock_version,
}


def collect_versions(root: Path) -> list[VersionResult]:
    """逐处读取版本号；单处失败不影响其它来源，便于一次看全问题。"""
    results = []
    for source, relative_path, reader_key in VERSION_SOURCES:
        path = root / relative_path
        try:
            if not path.is_file():
                raise FileNotFoundError(f"文件不存在：{path}")
            version = READERS[reader_key](path)
            results.append(VersionResult(source, version, ""))
        except (OSError, ValueError) as exc:
            results.append(VersionResult(source, "", str(exc)))
    return results


def evaluate(
    results: list[VersionResult], expect: str
) -> tuple[bool, str]:
    """判定一致性；区分读取失败、内部不一致、与期望版本不符三类。"""
    if any(item.error for item in results):
        return False, "存在读取失败项，配置管理检查不通过"
    versions = {item.version for item in results}
    if len(versions) > 1:
        return False, f"{len(results)} 个版本键不一致"
    actual = results[0].version
    if expect and actual != expect:
        return False, f"实际版本 {actual} 与期望版本 {expect} 不符"
    return True, f"{len(results)} 个版本键一致，均为 {actual}"


def render_markdown(
    root: Path,
    expect: str,
    results: list[VersionResult],
    passed: bool,
    conclusion: str,
) -> str:
    """渲染留痕文档，保证任何人可据此独立复核。"""
    lines = [
        "# 版本一致性检查留痕",
        "",
        f"- 执行时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        "- 执行脚本：`scripts/check_version_consistency.py`（SCR-001）",
        "- 编写人员：孙测测",
        f"- 项目根目录：`{root}`",
        f"- 期望版本：{expect or '未指定（仅校验内部一致性）'}",
        "",
        "| 来源 | 版本号 | 读取状态 |",
        "| --- | --- | --- |",
    ]
    for item in results:
        status = "成功" if not item.error else f"失败：{item.error}"
        lines.append(
            f"| `{item.source}` | {item.version or '-'} | {status} |"
        )
    lines += [
        "",
        f"- 结论：**{'通过' if passed else '不通过'}** — {conclusion}",
        "",
        "## 复核命令",
        "",
        "```bash",
        "python3 docs/孙测测/scripts/check_version_consistency.py "
        f"--root {root}",
        "```",
        "",
    ]
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    """解析命令行参数，保证脚本可跨目录、跨版本复用。"""
    parser = argparse.ArgumentParser(
        description="校验 SUGT 六处文件、七个版本键一致性并生成留痕（孙测测）"
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[3],
        help="项目根目录，默认为脚本所在专属目录的上三级",
    )
    parser.add_argument(
        "--expect",
        default="",
        help="期望版本号，例如 1.1.8；不传则只校验内部一致性",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="留痕输出目录，默认写入专属目录 traces/",
    )
    return parser.parse_args()


def main() -> int:
    """脚本入口：读取、判定、落盘留痕、返回退出码。"""
    args = parse_args()
    root = args.root.resolve()
    results = collect_versions(root)
    passed, conclusion = evaluate(results, args.expect)

    # 默认留痕落在专属目录 traces/，不污染项目其它位置
    out_dir = args.out or Path(__file__).resolve().parents[1] / "traces"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_path = out_dir / f"版本一致性检查-{stamp}.md"
    out_path.write_text(
        render_markdown(root, args.expect, results, passed, conclusion),
        encoding="utf-8",
    )

    print(f"结论：{'通过' if passed else '不通过'} — {conclusion}")
    print(f"留痕：{out_path}")
    for item in results:
        print(f"  {item.source}: {item.version or item.error}")

    # 读取失败优先于一致性失败，便于快速定位环境问题
    if any(item.error for item in results):
        return 2
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
