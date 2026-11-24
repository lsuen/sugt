# -*- coding: utf-8 -*-
"""
脚本名称：check_no_fake_data.py
编写人员：孙测测
作用用途：SUGT 生产代码「禁止模拟/假数据、禁止硬绑数据（必须参数化）」门禁。
          用七条可判定规则扫描前端 TS/TSX 与后端 Rust 生产代码，任何命中
          即不通过，并输出可独立复核的 Markdown 留痕。服务于用户明确要求：
          「不允许再有模拟或假数据；不允许硬绑数据，做参数化」。
          R7 为日志文件名单点化漂移门禁（WB-016）：sugt.log 字面量只允许
          出现在 logging.rs 的 LOG_FILE_NAME 常量定义行。
适用项目：SUGT（Tauri 2 + React + Rust/Axum）
创建日期：2026-11-22
复用方式：python3 docs/孙测测/scripts/check_no_fake_data.py --root <项目根>
          [--out <留痕目录>]；请从项目根执行（--root 默认当前工作目录）。
退出码约定：0 通过（七条规则均无命中）/ 1 不通过（存在命中）/ 2 读取或解析失败。
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Iterator

# 扫描范围：前端源码 + 后端源码；测试相关目录 / 文件名一律排除
SCAN_GLOBS = ("src/**/*.ts", "src/**/*.tsx", "src-tauri/src/**/*.rs")
EXCLUDED_DIRS = frozenset(
    {"node_modules", "dist", "target", "tests", "test", "fixtures", "__tests__"}
)

# R1：假数据标识（作为返回值 / 展示值时才命中）
FAKE_DATA_PATTERN = re.compile(
    r"mock|fake|dummy|模拟数据|假数据|示例数据", re.IGNORECASE
)
# 合法常量定义（如 DUMMY_LOCAL_KEY）与 prompt 文本属白名单
CONST_DECL_PATTERN = re.compile(
    r"\b(?:pub\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\b"
)
UPPER_FAKE_ID_PATTERN = re.compile(
    r"\b[A-Z0-9_]*(?:DUMMY|MOCK|FAKE)[A-Z0-9_]*\b"
)
TEST_FN_PATTERN = re.compile(r"\bfn\s+\w*(?:mock|fake|dummy)\w*\s*\(", re.IGNORECASE)
# prompt 文本特征：命中这些片段的行是提示词正文，不是假数据
PROMPT_MARKERS = (
    "你是小酥",
    "You are Xiaosu",
    "Never use tool-call",
    "<|tool_call_start|>",
    "fake functions",
    "fake remembering",
)

# R2：写死品牌 / 占位 key（精确字面量，大小写敏感）
HARDCODED_LITERAL_PATTERN = re.compile(r"(['\"])(sutai|sugt-local-key)\1")
HARDCODED_JSX_PATTERN = re.compile(r">\s*(sutai|sugt-local-key)\s*<")
CONST_WITH_LITERAL_PATTERN = re.compile(
    r"\b(?:pub\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\b[^=]*="
    r"[^;\n]*?['\"](?:sutai|sugt-local-key)['\"]"
)
# R3：token 估算冒充实测
ESTIMATE_PATTERN = re.compile(
    r"estimate_[a-z_]*tokens|chars\(\)\.count\(\)|估算|估计", re.IGNORECASE
)
TOKEN_WRITE_PATTERN = re.compile(
    r"[\"']total_tokens[\"']\s*:|[\"']totalTokens[\"']\s*:"
    r"|\[[\"']total_tokens[\"']\]\s*=",
    re.IGNORECASE,
)
ESTIMATED_FLAG_PATTERN = re.compile(
    r"estimated|isEstimated|is_estimated|usage_estimated|tokens_estimated",
    re.IGNORECASE,
)
# R3：写入点局部判定窗口（前后各取若干代码行，注释 / 测试块已剔除）
R3_WINDOW_LINES = 8

# R6：默认主密钥（DUMMY_LOCAL_KEY / 字面量 sugt-local-key）的语义特征。
# 旧规则以注释里的 DUMMY_LOCAL_KEY 为锚点，注释被剔除后恒 0 命中（假绿）。
# 现在直接判定「默认主密钥是否出现在统计入口的排除条件 + 丢弃动作中」。
DEFAULT_LOCAL_KEY_PATTERN = re.compile(r"DUMMY_LOCAL_KEY|sugt-local-key", re.IGNORECASE)
EXCLUSION_CONTEXT_PATTERN = re.compile(
    r"\bif\b|\bmatch\b|!=|\.retain\s*\(|\.filter\s*\(|\.reject\s*\(|\bexclude\b|\bskip\b",
    re.IGNORECASE,
)
DISCARD_PATTERN = re.compile(r"\breturn\s+None\b|\bcontinue\b", re.IGNORECASE)
NEGATED_DEFAULT_PATTERN = re.compile(
    r"!=\s*(?:DUMMY_LOCAL_KEY|['\"]sugt-local-key['\"])"
    r"|(?:DUMMY_LOCAL_KEY|['\"]sugt-local-key['\"])\s*!=",
    re.IGNORECASE,
)
# R6 的运行时语义守卫用例（cargo test 会在 CI 中执行该用例）
E2E_TEST_REL = "src-tauri/tests/client_keys_e2e.rs"
E2E_TEST_NAME = "default_local_key_usage_is_recorded"

# R7：日志文件名单点化漂移门禁（WB-016）。sugt.log 字面量只允许出现在
# logging.rs 的 LOG_FILE_NAME 常量定义行；其它生产文件必须引用该常量。
# 这样改日志文件名时只有一处真相源，避免「日志面板空、磁盘有文件」。
LOGGING_REL = "src-tauri/src/logging.rs"
LOG_FILE_LITERAL_PATTERN = re.compile(r"sugt\.log")
# 常量声明（值无关）：用于守卫「单一真相源必须存在」，防止整段删掉后字面量
# 一起消失造成假绿；同时用于识别唯一允许出现字面量的那一行。
LOG_FILE_CONST_DECL_PATTERN = re.compile(
    r"(?:pub\s+)?const\s+LOG_FILE_NAME\s*:\s*&str\s*="
)

# 单一真相源 / 例外文件（相对项目根）
# R2 白名单最终口径（team-lead 确认）：gateway_auth.rs 的 DUMMY_LOCAL_KEY
# 定义处、model.rs 的 CLIENT_MODEL_ID 定义处、src/constants.ts 整文件、
# src/i18n.ts 整文件（文案不算硬绑）。其余文件引用常量，不得再写死字面量。
I18N_REL = "src/i18n.ts"
CONSTANTS_REL = "src/constants.ts"
GATEWAY_AUTH_REL = "src-tauri/src/gateway_auth.rs"
MODEL_REL = "src-tauri/src/model.rs"
PROVIDER_PRESETS_REL = "src/providerPresets.ts"
GATEWAY_REL = "src-tauri/src/gateway.rs"
PROVIDER_CATALOG_REL = "src-tauri/src/provider_catalog.rs"
DEEPSEEK_DOC_URL_AUTHORITATIVE = "https://api-docs.deepseek.com/"
# R2 允许的「文件 + 常量名」组合：只有这两个常量定义处可出现品牌 / 占位 key
# 字面量；其它文件必须引用它们，不得复制字面量。
ALLOWED_CONST_DEFS = frozenset(
    {
        (GATEWAY_AUTH_REL, "DUMMY_LOCAL_KEY"),
        (MODEL_REL, "CLIENT_MODEL_ID"),
    }
)
OVERLAY_SCRIPT_RELS = (
    "docs/孙测测/scripts/check_pet_overlay.py",
    "docs/小精灵维护/scripts/check_pet_overlay.py",
)

RULE_TITLES = {
    "R1": "假数据标识出现在生产代码返回值 / 展示值",
    "R2": "写死品牌 / 占位 key 参与业务逻辑（未参数化）",
    "R3": "token 估算写入点附近必须有 estimated 标记（按写入点局部判定）",
    "R4": "两份 check_pet_overlay.py 的 sha256 必须一致",
    "R5": "deepseek 文档地址前端 / 后端必须与权威值一致",
    "R6": "用量统计入口不得排除默认主密钥（语义判定 + E2E 守卫）",
    "R7": "日志文件名 sugt.log 字面量仅允许 LOG_FILE_NAME 常量定义行（单点化漂移门禁）",
}


@dataclass
class Finding:
    """单条命中记录，带文件与行号，便于逐条复核。"""

    rule: str
    path: str
    line: int
    snippet: str
    reason: str


@dataclass
class RuleResult:
    """单条规则的汇总结果。"""

    rule: str
    title: str
    findings: list[Finding] = field(default_factory=list)
    note: str = ""


def is_test_path(rel_path: str) -> bool:
    """按路径片段与文件名排除测试目录、测试文件与测试夹具。"""
    parts = rel_path.split("/")
    if any(part in EXCLUDED_DIRS for part in parts):
        return True
    name = parts[-1]
    if ".test." in name or ".spec." in name:
        return True
    if name.endswith(("_test.rs", "_tests.rs", "tests.rs")):
        return True
    return False


def collect_source_files(root: Path) -> list[Path]:
    """收集 scan 范围内的生产代码文件，按相对路径排序保证结果稳定。"""
    collected: dict[str, Path] = {}
    for pattern in SCAN_GLOBS:
        for path in root.glob(pattern):
            if not path.is_file():
                continue
            rel_path = path.relative_to(root).as_posix()
            if is_test_path(rel_path):
                continue
            collected[rel_path] = path
    return [collected[key] for key in sorted(collected)]


def iter_code_lines(path: Path) -> Iterator[tuple[int, str]]:
    """产出生产代码行（1 起始行号），剔除注释行与 Rust #[cfg(test)] 块。

    用大括号深度粗略定位 #[cfg(test)] mod tests 的边界；字符串里的成对
    花括号净变化为 0，不影响边界判定。此启发式只为排除测试代码，不参与
    业务规则判定。
    """
    lines = path.read_text(encoding="utf-8").splitlines()
    in_block_comment = False
    in_test_module = False
    test_base_depth = 0
    depth = 0
    for number, raw in enumerate(lines, 1):
        stripped = raw.strip()
        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            continue
        if stripped.startswith("/*"):
            if "*/" not in stripped:
                in_block_comment = True
            continue
        if stripped.startswith(("//", "*")):
            continue
        if not in_test_module and stripped.startswith("#[cfg(test)]"):
            in_test_module = True
            test_base_depth = depth
            continue
        if in_test_module:
            depth += stripped.count("{") - stripped.count("}")
            if depth <= test_base_depth:
                in_test_module = False
            continue
        depth += stripped.count("{") - stripped.count("}")
        yield number, raw


def snippet_of(line: str, limit: int = 160) -> str:
    """压缩代码摘要，避免留痕被长行撑开。"""
    compact = " ".join(line.split())
    return compact if len(compact) <= limit else compact[: limit - 3] + "..."


def is_whitelisted_fake_line(line: str, rel_path: str) -> bool:
    """R1 白名单：合法常量、prompt 正文、测试函数名、i18n 文案。"""
    if rel_path in {I18N_REL, CONSTANTS_REL}:
        return True
    if CONST_DECL_PATTERN.search(line) or UPPER_FAKE_ID_PATTERN.search(line):
        return True
    if TEST_FN_PATTERN.search(line):
        return True
    if any(marker in line for marker in PROMPT_MARKERS):
        return True
    return False


def check_r1(files: list[Path], root: Path) -> RuleResult:
    """R1：假数据标识作为生产代码返回值 / 展示值时命中。"""
    result = RuleResult("R1", RULE_TITLES["R1"])
    for path in files:
        rel_path = path.relative_to(root).as_posix()
        for number, line in iter_code_lines(path):
            if not FAKE_DATA_PATTERN.search(line):
                continue
            if is_whitelisted_fake_line(line, rel_path):
                continue
            result.findings.append(
                Finding("R1", rel_path, number, snippet_of(line), "生产代码出现假数据标识")
            )
    return result


def check_r2(files: list[Path], root: Path) -> RuleResult:
    """R2：写死品牌 / 占位 key 字面量参与业务逻辑时命中。"""
    result = RuleResult("R2", RULE_TITLES["R2"])
    for path in files:
        rel_path = path.relative_to(root).as_posix()
        if rel_path in {I18N_REL, CONSTANTS_REL}:
            continue
        for number, line in iter_code_lines(path):
            const_match = CONST_WITH_LITERAL_PATTERN.search(line)
            if const_match and (rel_path, const_match.group(1)) in ALLOWED_CONST_DEFS:
                # 单一真相源常量定义处：参数化的源头，不算硬绑
                continue
            if not (
                HARDCODED_LITERAL_PATTERN.search(line)
                or HARDCODED_JSX_PATTERN.search(line)
            ):
                continue
            result.findings.append(
                Finding(
                    "R2",
                    rel_path,
                    number,
                    snippet_of(line),
                    "品牌 / 占位 key 写死在业务逻辑中，应从单一真相源参数化引入",
                )
            )
    return result


def check_r3(files: list[Path], root: Path) -> RuleResult:
    """R3：按「每个 total_tokens 写入点」局部判定，不再按整个文件豁免。

    对每个写入点，只在其前后 R3_WINDOW_LINES 行代码窗口内查找：
      1) 是否存在估算来源（ESTIMATE_PATTERN）；
      2) 是否带 estimated 标记（ESTIMATED_FLAG_PATTERN）。
    有估算来源却无标记 → 该写入点命中；没有估算来源的写入点视为真实值，放行。
    这样同一文件里合规的估算写入不会拖累全局，新增的无标记估算写入也逃不掉。
    """
    result = RuleResult("R3", RULE_TITLES["R3"])
    for path in files:
        rel_path = path.relative_to(root).as_posix()
        code_lines = list(iter_code_lines(path))
        write_indexes = [
            index
            for index, (_, line) in enumerate(code_lines)
            if TOKEN_WRITE_PATTERN.search(line)
        ]
        for index in write_indexes:
            number, line = code_lines[index]
            start = max(0, index - R3_WINDOW_LINES)
            end = min(len(code_lines), index + R3_WINDOW_LINES + 1)
            window = "\n".join(text for _, text in code_lines[start:end])
            if not ESTIMATE_PATTERN.search(window):
                # 该写入点附近没有估算来源，按真实值处理
                continue
            if ESTIMATED_FLAG_PATTERN.search(window):
                # 该写入点附近带 estimated 标记，属于合规的估算写入
                continue
            result.findings.append(
                Finding(
                    "R3",
                    rel_path,
                    number,
                    f"写入点 L{number}：{snippet_of(line)}",
                    f"写入点前后 {R3_WINDOW_LINES} 行存在 token 估算来源但无 estimated 标记，"
                    "估算值可能冒充实测",
                )
            )
    result.note = (
        f"按写入点局部判定（前后各 {R3_WINDOW_LINES} 行查找估算来源与 estimated 标记），"
        "不再以整文件出现 estimated 字样作为豁免"
    )
    return result


def sha256_of(path: Path) -> str:
    """计算文件 sha256，用于副本一致性判定。"""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_r4(root: Path) -> RuleResult:
    """R4：两份 check_pet_overlay.py 必须字节一致。"""
    result = RuleResult("R4", RULE_TITLES["R4"])
    digests: dict[str, str] = {}
    for rel_path in OVERLAY_SCRIPT_RELS:
        path = root / rel_path
        if not path.is_file():
            raise FileNotFoundError(f"缺少契约脚本：{rel_path}")
        digests[rel_path] = sha256_of(path)
    result.note = "；".join(f"{rel} = {digest}" for rel, digest in digests.items())
    if len(set(digests.values())) != 1:
        for rel_path, digest in digests.items():
            result.findings.append(
                Finding("R4", rel_path, 0, digest, "两份定版副本 sha256 不一致")
            )
    return result


def line_number_of(text: str, position: int) -> int:
    """把字符偏移换算成 1 起始行号。"""
    return text.count("\n", 0, position) + 1


def extract_doc_url(text: str, key: str) -> tuple[str | None, int]:
    """在 deepseek 条目块内提取文档地址（前端 docUrl / 后端 doc_url）。"""
    pattern = re.compile(
        r"\bid:\s*['\"]deepseek['\"]"  # deepseek 条目起点
        r"[\s\S]{0,800}?"  # 同一条目内向后找，不跨到下一个 id
        + re.escape(key)
        + r":\s*['\"]([^'\"]+)['\"]"
    )
    matched = pattern.search(text)
    if not matched:
        return None, 0
    return matched.group(1), line_number_of(text, matched.start(1))


def check_r5(root: Path) -> RuleResult:
    """R5：前后端 deepseek docUrl 必须与权威值一致（防止两端一起漂移）。"""
    result = RuleResult("R5", RULE_TITLES["R5"])
    frontend_path = root / PROVIDER_PRESETS_REL
    if not frontend_path.is_file():
        raise FileNotFoundError(f"缺少前端预设：{PROVIDER_PRESETS_REL}")
    front_url, front_line = extract_doc_url(
        frontend_path.read_text(encoding="utf-8"), "docUrl"
    )

    # 真相源是 provider_catalog.rs 的 provider 目录条目（当前符号 VENDORS /
    # VendorCatalogEntry，历史上叫 BUILTIN_PROVIDER_DIR；probe.rs 现行版本已不存在）。
    # 不依赖符号名，只找含 deepseek id 与 doc_url 的条目，provider_catalog.rs 优先。
    rust_paths = sorted((root / "src-tauri" / "src").glob("**/*.rs"))
    rust_paths.sort(
        key=lambda p: (
            p.as_posix() != str(root / PROVIDER_CATALOG_REL),
            p.as_posix(),
        )
    )
    back_url: str | None = None
    back_line = 0
    back_rel = ""
    for path in rust_paths:
        if is_test_path(path.relative_to(root).as_posix()):
            continue
        url, number = extract_doc_url(path.read_text(encoding="utf-8"), "doc_url")
        if url is not None:
            back_url, back_line = url, number
            back_rel = path.relative_to(root).as_posix()
            break

    # 与权威真相源比对：两端相等还不够，必须各自等于权威值。
    # 归一化只去掉末尾斜杠，与权威值保持同一比较口径。
    auth_norm = DEEPSEEK_DOC_URL_AUTHORITATIVE.rstrip("/")
    front_norm = front_url.rstrip("/") if front_url is not None else None
    back_norm = back_url.rstrip("/") if back_url is not None else None
    aligned = (
        front_norm is not None
        and back_norm is not None
        and front_norm == auth_norm
        and back_norm == auth_norm
    )
    result.note = (
        f"已与权威值比对（{'一致' if aligned else '不一致'}）；"
        f"权威值 {DEEPSEEK_DOC_URL_AUTHORITATIVE!r}；"
        f"真相源 {PROVIDER_CATALOG_REL}；"
        f"前端 {PROVIDER_PRESETS_REL}:L{front_line} = {front_url!r}；"
        f"后端 {back_rel or '未找到'}:L{back_line} = {back_url!r}"
    )
    if front_url is None or back_url is None:
        result.findings.append(
            Finding("R5", PROVIDER_PRESETS_REL, front_line, result.note, "文档地址字段缺失")
        )
    elif front_norm != back_norm:
        result.findings.append(
            Finding(
                "R5",
                PROVIDER_PRESETS_REL,
                front_line,
                f"前端={front_url} / 后端({back_rel}:L{back_line})={back_url}",
                "deepseek 文档地址前后端漂移（且与权威值比对不一致）",
            )
        )
    elif not aligned:
        # 两端被同时改成同一个错误值：旧规则只比相等会漏判，这里必须失败
        result.findings.append(
            Finding(
                "R5",
                PROVIDER_PRESETS_REL,
                front_line,
                f"前端={front_url} / 后端({back_rel}:L{back_line})={back_url} / "
                f"权威值={DEEPSEEK_DOC_URL_AUTHORITATIVE}",
                f"deepseek 文档地址两端同时偏离权威值 {DEEPSEEK_DOC_URL_AUTHORITATIVE!r}",
            )
        )
    return result


def check_r6(root: Path) -> RuleResult:
    """R6：用量统计不得把默认主密钥排除（语义判定 + E2E 守卫用例在场）。

    旧规则把注释里的 DUMMY_LOCAL_KEY 当锚点，注释被 iter_code_lines 剔除后
    永远 0 命中（假绿）。现在直接按语义判定：凡默认主密钥 token 出现在
    「排除条件 + 丢弃动作（return None / continue / 从计数集合剔除）」的
    同一语句窗口内即命中；同时要求 E2E 守卫用例仍在场，双重保险。
    """
    result = RuleResult("R6", RULE_TITLES["R6"])
    path = root / GATEWAY_REL
    if not path.is_file():
        raise FileNotFoundError(f"缺少网关源码：{GATEWAY_REL}")
    code_lines = list(iter_code_lines(path))
    for index, (number, line) in enumerate(code_lines):
        if not DEFAULT_LOCAL_KEY_PATTERN.search(line):
            continue
        # 窗口含当前行：单行 `if presented == DUMMY_LOCAL_KEY { return None }`
        # 与跨行 `if ... DUMMY {` + `return None` 两种写法都要拦住。
        end = min(len(code_lines), index + 5)
        window = "\n".join(text for _, text in code_lines[index:end])
        excluded = EXCLUSION_CONTEXT_PATTERN.search(window) is not None and (
            DISCARD_PATTERN.search(window) is not None
            or NEGATED_DEFAULT_PATTERN.search(window) is not None
        )
        if excluded:
            result.findings.append(
                Finding(
                    "R6",
                    GATEWAY_REL,
                    number,
                    snippet_of(line),
                    "默认主密钥出现在统计入口的排除条件中（return None / continue / 从计数集合剔除），"
                    "其用量将恒为 0",
                )
            )

    # 运行时语义守卫：E2E 用例必须存在，删除 / 改名同样判失败
    e2e_path = root / E2E_TEST_REL
    e2e_text = ""
    if e2e_path.is_file():
        e2e_text = e2e_path.read_text(encoding="utf-8")
    if not e2e_text:
        result.findings.append(
            Finding("R6", E2E_TEST_REL, 0, E2E_TEST_REL, "缺少用量统计 E2E 守卫用例文件")
        )
    elif E2E_TEST_NAME not in e2e_text:
        result.findings.append(
            Finding("R6", E2E_TEST_REL, 0, E2E_TEST_NAME, "用量统计 E2E 守卫用例缺失或改名")
        )

    result.note = (
        f"语义扫描 {GATEWAY_REL} 代码行（注释 / 测试块已剔除，窗口为当前行起 5 行）："
        "判定「默认主密钥 token + 排除条件 + 丢弃动作」；"
        f"E2E 守卫 {E2E_TEST_REL}::{E2E_TEST_NAME}"
        f"（{'在场' if E2E_TEST_NAME in e2e_text else '缺失'}）"
    )
    return result


def check_r7(files: list[Path], root: Path) -> RuleResult:
    """R7：日志文件名单点化漂移门禁（WB-016）。

    生产代码（注释 / #[cfg(test)] 块 / 测试目录已由 iter_code_lines 与
    collect_source_files 剔除）里，字面量 sugt.log 只允许出现在 logging.rs
    的 LOG_FILE_NAME 常量定义行；其它任何位置出现即命中。另设正向守卫：
    该常量声明必须存在，防止整段删除后字面量一起消失造成假绿（同 R6 思路）。
    """
    result = RuleResult("R7", RULE_TITLES["R7"])
    const_decl_found = False
    for path in files:
        rel_path = path.relative_to(root).as_posix()
        for number, line in iter_code_lines(path):
            is_const_decl = LOG_FILE_CONST_DECL_PATTERN.search(line) is not None
            if rel_path == LOGGING_REL and is_const_decl:
                const_decl_found = True
            if not LOG_FILE_LITERAL_PATTERN.search(line):
                continue
            if rel_path == LOGGING_REL and is_const_decl:
                # 唯一允许：单一真相源的常量定义行
                continue
            result.findings.append(
                Finding(
                    "R7",
                    rel_path,
                    number,
                    snippet_of(line),
                    "生产代码出现 sugt.log 字面量，应引用 crate::logging::LOG_FILE_NAME（单点化）",
                )
            )

    if not const_decl_found:
        result.findings.append(
            Finding(
                "R7",
                LOGGING_REL,
                0,
                LOG_FILE_CONST_DECL_PATTERN.pattern,
                "缺少 LOG_FILE_NAME 常量声明（单一真相源被删除，改名时无法统一）",
            )
        )

    result.note = (
        f"唯一允许字面量的位置：{LOGGING_REL} 的 LOG_FILE_NAME 常量声明行；"
        f"常量声明{'在场' if const_decl_found else '缺失'}；"
        "扫描已排除注释、#[cfg(test)] 块与测试目录"
    )
    return result


def render_markdown(
    root: Path,
    results: list[RuleResult],
    passed: bool,
    conclusion: str,
) -> str:
    """渲染留痕文档，任何人可据此独立复核。"""
    total = sum(len(item.findings) for item in results)
    lines = [
        "# 禁止假数据 / 硬绑数据门禁检查留痕",
        "",
        f"- 执行时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        "- 执行脚本：`scripts/check_no_fake_data.py`（SCR-025）",
        "- 编写人员：孙测测",
        f"- 项目根目录：`{root}`",
        f"- 扫描范围：{'、'.join(SCAN_GLOBS)}（排除测试目录 / 测试文件 / 注释 / #[cfg(test)] 块）",
        "",
        "## 一、规则清单与结论",
        "",
        "| 规则 | 检查内容 | 命中数 | 结论 |",
        "| --- | --- | --- | --- |",
    ]
    for item in results:
        verdict = "通过" if not item.findings else "不通过"
        lines.append(
            f"| {item.rule} | {item.title} | {len(item.findings)} | {verdict} |"
        )
    lines += [
        "",
        f"- 汇总命中：**{total}** 项",
        f"- 结论：**{'通过' if passed else '不通过'}** — {conclusion}",
        "",
        "## 二、逐规则命中明细",
        "",
    ]
    for item in results:
        lines += [f"### {item.rule} {item.title}", ""]
        if item.note:
            lines += [f"- 校验数据：{item.note}", ""]
        if not item.findings:
            lines += ["- 无命中。", ""]
            continue
        lines += [
            "| 规则 | 文件 | 行号 | 代码摘要 / 校验值 | 命中原因 |",
            "| --- | --- | --- | --- | --- |",
        ]
        for entry in item.findings:
            snippet = entry.snippet.replace("|", "\\|")
            reason = entry.reason.replace("|", "\\|")
            lines.append(
                f"| {entry.rule} | `{entry.path}` | {entry.line or '-'} | "
                f"{snippet} | {reason} |"
            )
        lines.append("")
    lines += [
        "## 三、复核命令",
        "",
        "```bash",
        "python3 docs/孙测测/scripts/check_no_fake_data.py " f"--root {root}",
        "```",
        "",
        "> 命中项不要求修复前为 0；前端 / 后端按单一真相源参数化修复后复跑，",
        "> 七条规则全绿即退出码 0。退出码：0 通过 / 1 不通过 / 2 读取解析失败。",
        "",
    ]
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    """解析命令行参数，保证脚本可跨环境复用。"""
    parser = argparse.ArgumentParser(
        description="SUGT 生产代码禁止假数据 / 硬绑数据门禁并生成留痕（孙测测）"
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="项目根目录，默认当前工作目录（请从项目根执行）",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="留痕输出目录，默认写入脚本所属专属目录的 traces/",
    )
    return parser.parse_args()


def main() -> int:
    """脚本入口：扫描、判定、落盘留痕、返回退出码。"""
    args = parse_args()
    root = args.root.resolve()
    results: list[RuleResult] = []
    parse_error = ""

    if not root.is_dir():
        parse_error = f"项目根目录不存在：{root}"
    else:
        try:
            files = collect_source_files(root)
            results = [
                check_r1(files, root),
                check_r2(files, root),
                check_r3(files, root),
                check_r4(root),
                check_r5(root),
                check_r6(root),
                check_r7(files, root),
            ]
        except (OSError, UnicodeError, ValueError) as exc:
            # 读取失败优先于检查不通过，便于快速定位环境问题
            parse_error = f"读取或解析失败：{exc}"

    total = sum(len(item.findings) for item in results)
    passed = not parse_error and total == 0
    if parse_error:
        conclusion = parse_error
    elif passed:
        conclusion = "七条规则均无命中，生产代码未发现假数据 / 硬绑数据"
    else:
        conclusion = f"发现 {total} 项命中，需按单一真相源参数化后复跑"

    # 无论成功失败都生成留痕，保证每次执行可追溯
    out_dir = args.out or Path(__file__).resolve().parents[1] / "traces"
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_path = out_dir / f"禁止假数据硬绑数据检查-{stamp}.md"
    out_path.write_text(render_markdown(root, results, passed, conclusion), encoding="utf-8")

    print(f"结论：{'通过' if passed else '不通过'} — {conclusion}")
    print(f"留痕：{out_path}")
    for item in results:
        print(f"  {item.rule}: {len(item.findings)} 项命中 — {item.title}")
        for entry in item.findings:
            print(f"    - {entry.path}:{entry.line or '-'} {entry.snippet}")

    if parse_error:
        return 2
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
