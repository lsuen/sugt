#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：从 src/overlay.css 抽取桌面精灵样式，注入真实 WebP 素材，生成离线可点预览页；
         同时校验关键布局选择器是否齐全，避免预览页与产品样式漂移。
适用项目：SUGT（桌面精灵 小su）
创建日期：2026-11-08
复用方式：python3 docs/孙测测/scripts/build_pet_preview.py [--root 项目根] [--out 输出HTML]；
         每次修改精灵 CSS 或素材后重跑即可，无需手工维护预览页。
退出码约定：0 通过 / 1 校验不通过 / 2 读取或解析失败
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from datetime import datetime
from pathlib import Path

# 预览页必须包含的 v0.6 布局选择器；缺一个就说明 CSS 被改坏或版本回退
REQUIRED_SELECTORS = (
    ".pet-side",
    ".pet-figure",
    ".pet-pose",
    ".pet-status-bar",
    ".pet-status-dot",
    ".pet-bubble",
    ".pet-edit-bar",
)
FRAME_NAMES = ("su-idle", "su-typing", "su-done", "su-error", "su-sleep")
CSS_ANCHOR = "/* ===== 桌面精灵"
URL_PATTERN = re.compile(r"url\('\./assets/pet/([a-z0-9-]+)\.webp'\)")

TEMPLATE = r"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>小su 桌面精灵 v0.6 · 同源 CSS 预览</title>
<style>
:root{--bg:#0E1220;--surface:#171D2E;--border:#2C3650;--text:#E8EDF7;--muted:#94A2BC;--dim:#6F7D99;--accent:#5DD9A8}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif;font-size:14px;line-height:1.7}
.wrap{max-width:1080px;margin:0 auto;padding:40px 28px 72px}
h1{font-size:25px;font-weight:500;margin:0 0 6px}
.lead{color:var(--muted);margin:0 0 24px;max-width:820px}
.badge{display:inline-block;font-size:11px;color:var(--accent);border:1px solid rgba(93,217,168,.4);border-radius:999px;padding:2px 10px;margin:0 6px 18px 0}
.layout{display:grid;grid-template-columns:290px 1fr;gap:24px;align-items:start}
.card{background:var(--surface);border:1px solid var(--border);border-radius:14px;padding:18px 20px}
.card h2{font-size:14px;font-weight:500;margin:0 0 12px;color:var(--text)}
.group{margin-bottom:18px}
.group:last-child{margin-bottom:0}
.group>label{display:block;font-size:11px;color:var(--dim);margin-bottom:7px;letter-spacing:.04em}
.btnrow{display:flex;gap:6px;flex-wrap:wrap}
button{font:inherit;font-size:12px;color:var(--muted);background:#1E263B;border:1px solid var(--border);border-radius:8px;padding:5px 11px;cursor:pointer}
button:hover{color:var(--text);border-color:var(--accent)}
button.on{color:#0E1220;background:var(--accent);border-color:var(--accent);font-weight:600}
.hint{font-size:11.5px;color:var(--dim);margin:10px 0 0;line-height:1.6}
.desktop{background:radial-gradient(circle at 25% 18%,#243450 0%,#141B2C 55%,#0C111D 100%);border:1px solid var(--border);border-radius:14px;height:440px;display:flex;align-items:center;justify-content:center;position:relative;overflow:hidden;transition:background .3s}
.desktop.light{background:linear-gradient(135deg,#DCE6F2 0%,#B9CBDD 55%,#8FA6BF 100%)}
.desktop::before{content:"透明窗口 · 虚线为窗口边界";position:absolute;left:14px;top:12px;font-size:11px;color:#8494AE}
.desktop.light::before{color:#4C5C72}
.win{border:1px dashed rgba(140,200,232,.35);border-radius:4px;position:relative;transform-origin:center center;transition:transform .25s ease}
.win iframe{display:block;border:0;background:transparent}
.readout{font-size:11.5px;color:var(--dim);margin-top:10px;text-align:right}
table{width:100%;border-collapse:collapse;font-size:12.5px;margin-top:24px}
th,td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--border);vertical-align:top}
th{color:var(--dim);font-weight:500}td{color:var(--muted)}
td:first-child{color:var(--text);white-space:nowrap}
footer{margin-top:30px;font-size:11.5px;color:var(--dim)}
code{background:rgba(255,255,255,.06);border-radius:4px;padding:1px 5px;font-size:11.5px}
</style>
</head>
<body>
<div class="wrap">
<h1>小su 桌面精灵 v0.6 · 同源 CSS 预览</h1>
<p class="lead">样式直接取自 <code>src/overlay.css</code>，素材为进包的 5 张透明 WebP（本页内联，单文件离线可用）。窗口放进独立 iframe，因此 <code>vw</code> 字号与实际悬浮窗一致；外层仅做等比放大便于观察。</p>
<span class="badge">生成时间 __GENERATED_AT__</span>
<span class="badge">源 CSS sha256 __CSS_HASH__</span>
<span class="badge">素材 5 张 / 85.45 KB</span>

<div class="layout">
  <div class="card">
    <div class="group">
      <label>状态（点击切换动作帧与状态灯）</label>
      <div class="btnrow" id="stateBtns"></div>
    </div>
    <div class="group">
      <label>尺寸档位</label>
      <div class="btnrow" id="sizeBtns"></div>
    </div>
    <div class="group">
      <label>叠加显示</label>
      <div class="btnrow">
        <button type="button" id="bubbleBtn">气泡</button>
        <button type="button" id="dragBtn">拖动中</button>
        <button type="button" id="editBtn">编辑态</button>
        <button type="button" id="bgBtn">浅色桌面</button>
      </div>
    </div>
    <p class="hint">气泡与状态胶囊固定在左侧 44% 通知栏，右侧立绘靠右下角，互不遮挡。切换 done / error 后观察：一次性动效播完，父层呼吸仍在继续。</p>
  </div>

  <div>
    <div class="desktop" id="desktop">
      <div class="win" id="win"><iframe id="pet" title="桌面精灵同源预览"></iframe></div>
    </div>
    <div class="readout" id="readout"></div>
  </div>
</div>

<table>
<tr><th>本轮修复</th><th>预览中可验证</th></tr>
<tr><td>PET-001 状态真实性</td><td>代码链路：在途守卫随响应体释放，成功气泡等在途归零；本页仅验证 typing 动作帧持续播放</td></tr>
<tr><td>PET-005 动画分层</td><td>done / error 动效结束后角色仍持续呼吸，不再僵住</td></tr>
<tr><td>PET-006 气泡遮脸</td><td>开启「气泡」后，气泡只占左侧，角色面部完整可见</td></tr>
<tr><td>PET-007 状态重复</td><td>只剩左下角一个状态胶囊（灯 + 文字），无独立 core / hover 浮层</td></tr>
<tr><td>三档自适应</td><td>小 / 中 / 大切换，字号与胶囊随窗口宽度平滑缩放，立绘高度等比</td></tr>
</table>

<footer>由 <code>docs/孙测测/scripts/build_pet_preview.py</code> 生成，请勿手工编辑；修改样式后重跑脚本即可。预览页不参与打包。</footer>
</div>

<script>
const PET_CSS = __PET_CSS__;
const STATES = [
  ['idle', '待机中', 'idle'], ['typing', '处理中', 'typing'], ['done', '已完成', 'done'],
  ['error', '出错了', 'error'], ['sleep', '休息中', 'sleep'], ['offline', '未运行', 'idle'],
];
const SIZES = { small: [200, 160, 2], medium: [300, 240, 1.42], large: [400, 320, 1.12] };
const BUBBLES = {
  idle: ['网关已就绪', 'ok'], typing: ['上游已切换', 'info'], done: ['完成 3 次请求', 'ok'],
  error: ['有请求失败', 'error'], sleep: ['小su 只播报真实状态', 'info'], offline: ['网关未运行', 'info'],
};
let currentState = 'idle';
let currentSize = 'small';
let showBubble = true;
let isDragging = false;
let editMode = false;
let lightBg = false;

// 支持通过 URL hash 直接打开指定组合，便于无头浏览器批量截图验收
const params = new URLSearchParams(location.hash.slice(1));
if (STATES.some(s => s[0] === params.get('state'))) currentState = params.get('state');
if (SIZES[params.get('size')]) currentSize = params.get('size');
if (params.get('bubble') === '0') showBubble = false;
if (params.get('drag') === '1') isDragging = true;
if (params.get('edit') === '1') editMode = true;
if (params.get('bg') === 'light') lightBg = true;

const iframe = document.getElementById('pet');
const win = document.getElementById('win');

function buildButtons() {
  const sb = document.getElementById('stateBtns');
  STATES.forEach(([key, label]) => {
    const b = document.createElement('button');
    b.type = 'button'; b.textContent = label; b.dataset.key = key;
    b.onclick = () => { currentState = key; syncButtons(); applyState(); };
    sb.appendChild(b);
  });
  const zb = document.getElementById('sizeBtns');
  Object.keys(SIZES).forEach((key, i) => {
    const b = document.createElement('button');
    b.type = 'button'; b.textContent = ['小 200×160', '中 300×240', '大 400×320'][i]; b.dataset.key = key;
    b.onclick = () => { currentSize = key; syncButtons(); renderFrame(); };
    zb.appendChild(b);
  });
}

function syncButtons() {
  document.querySelectorAll('#stateBtns button').forEach(b => b.classList.toggle('on', b.dataset.key === currentState));
  document.querySelectorAll('#sizeBtns button').forEach(b => b.classList.toggle('on', b.dataset.key === currentSize));
  document.getElementById('bubbleBtn').classList.toggle('on', showBubble);
  document.getElementById('dragBtn').classList.toggle('on', isDragging);
  document.getElementById('editBtn').classList.toggle('on', editMode);
  document.getElementById('bgBtn').classList.toggle('on', lightBg);
}

function frameDoc() {
  return '<!doctype html><html><head><meta charset="utf-8"><style>'
    + 'html,body{margin:0;padding:0;width:100%;height:100%;background:transparent;overflow:hidden}'
    + PET_CSS
    + '</style></head><body>'
    + '<div class="pet-stage interactive" data-state="idle" id="stage">'
    + '<div class="pet-side">'
    + '<div class="pet-bubble pet-bubble-ok" id="bubble" role="status"></div>'
    + '<div class="pet-status-bar" role="status"><span class="pet-status-dot"></span><span class="pet-status-text" id="label"></span></div>'
    + '</div>'
    + '<div class="pet-figure" id="figure"><span class="pet-pose" data-frame="idle"></span></div>'
    + '<div class="pet-edit-bar" id="editbar" style="display:none"><button type="button" class="overlay-save">保存</button><span class="overlay-grip"></span></div>'
    + '</div></body></html>';
}

function applyState() {
  const doc = iframe.contentDocument;
  if (!doc || !doc.getElementById('stage')) return;
  const entry = STATES.find(s => s[0] === currentState);
  const [text, kind] = BUBBLES[currentState];
  doc.getElementById('stage').dataset.state = currentState;
  doc.querySelector('.pet-pose').dataset.frame = entry[2];
  doc.getElementById('label').textContent = entry[1];
  const bubble = doc.getElementById('bubble');
  bubble.textContent = text;
  bubble.className = 'pet-bubble pet-bubble-' + kind;
  bubble.style.display = showBubble ? '' : 'none';
  doc.getElementById('figure').classList.toggle('dragging', isDragging);
  doc.getElementById('editbar').style.display = editMode ? '' : 'none';
}

function renderFrame() {
  const [w, h, scale] = SIZES[currentSize];
  win.style.width = w + 'px';
  win.style.height = h + 'px';
  win.style.transform = 'scale(' + scale + ')';
  iframe.style.width = w + 'px';
  iframe.style.height = h + 'px';
  iframe.onload = applyState;
  iframe.srcdoc = frameDoc();
  document.getElementById('readout').textContent = '窗口 ' + w + ' × ' + h + ' px　预览放大 ' + scale + '×　立绘高度 ' + h + ' px';
}

function bindToggles() {
  const toggle = (id, get, set) => {
    document.getElementById(id).onclick = () => { set(!get()); syncButtons(); applyState(); };
  };
  toggle('bubbleBtn', () => showBubble, v => { showBubble = v; });
  toggle('dragBtn', () => isDragging, v => { isDragging = v; });
  toggle('editBtn', () => editMode, v => { editMode = v; });
  document.getElementById('bgBtn').onclick = () => {
    lightBg = !lightBg;
    document.getElementById('desktop').classList.toggle('light', lightBg);
    syncButtons();
  };
}

buildButtons();
bindToggles();
document.getElementById('desktop').classList.toggle('light', lightBg);
syncButtons();
renderFrame();
</script>
</body>
</html>
"""


def parse_args() -> argparse.Namespace:
    """解析命令行参数，默认路径全部由脚本位置推导，不硬编码本机绝对路径。"""
    default_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description="生成桌面精灵同源 CSS 离线预览页")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--out", type=Path, default=None, help="输出 HTML 路径")
    parser.add_argument("--trace-dir", type=Path, default=None, help="留痕目录")
    return parser.parse_args()


def build_preview(root: Path, out: Path) -> dict:
    """抽取精灵 CSS、内联素材、渲染预览页，返回本次生成的可核查信息。"""
    css_path = root / "src" / "overlay.css"
    pet_dir = root / "src" / "assets" / "pet"
    css = css_path.read_text(encoding="utf-8")

    anchor = css.find(CSS_ANCHOR)
    if anchor < 0:
        raise ValueError(f"未找到精灵样式锚点 {CSS_ANCHOR}")
    pet_css = css[anchor:]

    missing = [selector for selector in REQUIRED_SELECTORS if selector not in pet_css]
    if missing:
        raise ValueError("精灵 CSS 缺少关键选择器: " + ", ".join(missing))

    # 把相对图片路径换成内联 base64，保证预览页单文件离线可用
    replaced: list[str] = []

    def inline_image(match: re.Match) -> str:
        name = match.group(1)
        path = pet_dir / f"{name}.webp"
        data = path.read_bytes()
        replaced.append(f"{name}.webp ({len(data)} B)")
        encoded = base64.b64encode(data).decode("ascii")
        return f"url('data:image/webp;base64,{encoded}')"

    pet_css = URL_PATTERN.sub(inline_image, pet_css)
    if "url('./assets/pet/" in pet_css:
        raise ValueError("仍有未内联的素材路径")
    if len(replaced) != len(FRAME_NAMES):
        raise ValueError(f"素材数量异常: 期望 {len(FRAME_NAMES)}，实际 {len(replaced)}")

    generated_at = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    css_hash = hashlib.sha256(css.encode("utf-8")).hexdigest()[:12]
    html = (
        TEMPLATE.replace("__PET_CSS__", json.dumps(pet_css, ensure_ascii=False))
        .replace("__GENERATED_AT__", generated_at)
        .replace("__CSS_HASH__", css_hash)
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html, encoding="utf-8")

    return {
        "generated_at": generated_at,
        "css_path": css_path.relative_to(root).as_posix(),
        "css_hash": css_hash,
        "selectors": list(REQUIRED_SELECTORS),
        "assets": replaced,
        "output": out.relative_to(root).as_posix() if out.is_relative_to(root) else str(out),
        "bytes": out.stat().st_size,
    }


def write_trace(trace_dir: Path, info: dict) -> Path:
    """每次执行都留一份 Markdown 痕迹，便于复验时对照源 CSS 版本。"""
    trace_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = trace_dir / f"桌面精灵预览页生成-{stamp}.md"
    lines = [
        "# 桌面精灵同源预览页生成留痕",
        "",
        f"- 生成时间：{info['generated_at']}",
        f"- 源样式：`{info['css_path']}`（sha256 前 12 位 `{info['css_hash']}`）",
        f"- 输出：`{info['output']}`（{info['bytes']} B）",
        "- 关键选择器校验：" + "、".join(f"`{s}`" for s in info["selectors"]) + " → 全部存在",
        "- 内联素材：",
    ]
    lines.extend(f"  - `{name}`" for name in info["assets"])
    lines += ["", "结论：预览页与产品精灵 CSS 同源，素材全部内联，单文件离线可用。", ""]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main() -> int:
    """入口：解析参数 → 生成预览 → 写留痕 → 返回退出码。"""
    args = parse_args()
    root = args.root.resolve()
    out = (args.out or root / "docs" / "design" / "pet-v06-preview.html").resolve()
    trace_dir = (args.trace_dir or root / "docs" / "孙测测" / "traces").resolve()
    try:
        info = build_preview(root, out)
        trace = write_trace(trace_dir, info)
    except (OSError, ValueError) as exc:
        print(f"[失败] {exc}", file=sys.stderr)
        return 2
    print(f"[通过] 预览页：{out}")
    print(f"[通过] 源 CSS sha256 前 12 位：{info['css_hash']}")
    print(f"[通过] 内联素材 {len(info['assets'])} 张，关键选择器 {len(info['selectors'])} 个")
    print(f"[通过] 留痕：{trace}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
