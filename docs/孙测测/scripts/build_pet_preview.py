#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：从 src/overlay.css 抽取桌面精灵样式，内联真实 WebP 素材（含带 alpha 的
         动画 WebP），生成可点、可鼠标交互的离线预览页；同时校验关键选择器齐全，
         避免预览页与产品样式漂移。
适用项目：SUGT（桌面精灵 小su）v0.7
创建日期：2026-11-08（v0.7 于同日重写）
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

# 预览页必须包含的 v0.7 布局选择器；缺一个就说明 CSS 被改坏或版本回退
REQUIRED_SELECTORS = (
    ".pet-side",
    ".pet-figure",
    ".pet-motion",
    ".pet-loops",
    ".pet-loop",
    ".pet-poster",
    ".pet-blink",
    ".pet-gaze",
    ".pet-ambient",
    ".pet-status-bar",
    ".pet-status-dot",
    ".pet-bubble",
    ".pet-edit-bar",
)
# 5 张静态海报 + 5 个带 alpha 的动画 WebP
FRAME_NAMES = (
    "su-idle", "su-typing", "su-done", "su-error", "su-sleep", "su-blink",
    "su-idle-loop", "su-typing-loop", "su-done-loop", "su-error-loop", "su-sleep-loop",
)
CSS_ANCHOR = "/* ===== 桌面精灵"
URL_PATTERN = re.compile(r"url\('\./assets/pet/([a-z0-9-]+)\.webp'\)")

TEMPLATE = r"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>小su 桌面精灵 v0.7 · 同源 CSS 预览</title>
<style>
:root{--bg:#0E1220;--surface:#171D2E;--border:#2C3650;--text:#E8EDF7;--muted:#94A2BC;--dim:#6F7D99;--accent:#5DD9A8}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font-family:system-ui,-apple-system,"PingFang SC","Microsoft YaHei",sans-serif;font-size:14px;line-height:1.7}
.wrap{max-width:1080px;margin:0 auto;padding:40px 28px 72px}
h1{font-size:25px;font-weight:500;margin:0 0 6px}
.lead{color:var(--muted);margin:0 0 24px;max-width:860px}
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
.desktop{background:radial-gradient(circle at 25% 18%,#243450 0%,#141B2C 55%,#0C111D 100%);border:1px solid var(--border);border-radius:14px;height:460px;display:flex;align-items:center;justify-content:center;position:relative;overflow:hidden;transition:background .3s}
.desktop.light{background:linear-gradient(135deg,#DCE6F2 0%,#B9CBDD 55%,#8FA6BF 100%)}
.desktop::before{content:"移动鼠标：小su 会转头看你 · 虚线为透明窗口边界";position:absolute;left:14px;top:12px;font-size:11px;color:#8494AE}
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
<h1>小su 桌面精灵 v0.7 · 同源 CSS 预览</h1>
<p class="lead">样式直接取自 <code>src/overlay.css</code>；素材为进包的 6 张静态海报（含程序化闭眼帧）+ 5 个 <strong>带透明通道的动画 WebP</strong>（ComfyUI <code>SaveAnimatedWEBP</code> 编码，6 帧 / 8fps）。窗口放进独立 iframe，因此 <code>vw</code> 字号与实际悬浮窗一致；外层仅做等比放大便于观察。</p>
<span class="badge">生成时间 __GENERATED_AT__</span>
<span class="badge">源 CSS sha256 __CSS_HASH__</span>
<span class="badge">素材 __ASSET_COUNT__ 张 / __ASSET_KB__ KB（含 5 个带 alpha 动画 WebP）</span>

<div class="layout">
  <div class="card">
    <div class="group">
      <label>状态（切换真实逐帧动画）</label>
      <div class="btnrow" id="stateBtns"></div>
    </div>
    <div class="group">
      <label>尺寸档位</label>
      <div class="btnrow" id="sizeBtns"></div>
    </div>
    <div class="group">
      <label>交互叠加</label>
      <div class="btnrow">
        <button type="button" id="bubbleBtn">气泡</button>
        <button type="button" id="blinkBtn">眨眼帧</button>
        <button type="button" id="pokeBtn">轻碰</button>
        <button type="button" id="dragBtn">拖动中</button>
        <button type="button" id="editBtn">编辑态</button>
        <button type="button" id="stillBtn">静态帧</button>
        <button type="button" id="bgBtn">浅色桌面</button>
      </div>
    </div>
    <p class="hint">在右侧桌面区域移动鼠标可看到转头、眼神高光与靠近光晕；「眨眼帧」用来核对闭眼立绘；「静态帧」模拟 reduced-motion / 窗口隐藏时的降级。动画 WebP 在 iframe 内真实播放，不是截图。</p>
  </div>

  <div>
    <div class="desktop" id="desktop">
      <div class="win" id="win"><iframe id="pet" title="桌面精灵同源预览"></iframe></div>
    </div>
    <div class="readout" id="readout"></div>
  </div>
</div>

<table>
<tr><th>v0.7 改进</th><th>预览中可验证</th></tr>
<tr><td>发丝缝隙抠图</td><td>洋红纯色底 + 色度空间软色键 + 去溢色；深/浅桌面下发丝间均无白块、无粉边</td></tr>
<tr><td>真实逐帧动画</td><td>切状态后立绘持续缓慢呼吸/点头，不再是 0.42s 高频旋转的「奇怪摇晃」</td></tr>
<tr><td>视线跟随</td><td>鼠标靠近时头部轻微 3D 转向、眼神高光位移、光晕渐显；静止数秒会自主张望</td></tr>
<tr><td>眨眼 / 睡眠</td><td>「眨眼帧」显示程序化闭眼立绘；睡眠动画序列本身由闭眼帧生成，呼吸放慢</td></tr>
<tr><td>轻碰 / 拖动反馈</td><td>「轻碰」播放一次弹跳；「拖动中」显示抬起、投影与倾斜</td></tr>
<tr><td>状态一次性动效</td><td>done 一次小跳 + 火花，error 一次轻颤，结束后呼吸继续，不会僵住</td></tr>
</table>

<footer>由 <code>docs/孙测测/scripts/build_pet_preview.py</code> 生成，请勿手工编辑；修改样式后重跑脚本即可。预览页不参与打包。</footer>
</div>

<script>
const PET_CSS = __PET_CSS__;
const STATES = [
  ['idle', '待机中', 'idle'], ['typing', '处理中', 'typing'], ['done', '已完成', 'done'],
  ['error', '出错了', 'error'], ['sleep', '休息中', 'sleep'], ['offline', '未运行', 'idle'],
];
const LOOPS = ['idle', 'typing', 'done', 'error', 'sleep'];
const SIZES = { small: [200, 160, 2], medium: [300, 240, 1.42], large: [400, 320, 1.12] };
const BUBBLES = {
  idle: ['网关已就绪', 'ok'], typing: ['上游已切换', 'info'], done: ['完成 3 次请求', 'ok'],
  error: ['有请求失败', 'error'], sleep: ['小su 只播报真实状态', 'info'], offline: ['网关未运行', 'info'],
  poke: ['嗯？我在呢～', 'info'],
};
let currentState = 'idle';
let currentSize = 'small';
let showBubble = true;
let isDragging = false;
let isBlinking = false;
let isPoked = false;
let isStill = false;
let editMode = false;
let lightBg = false;

const params = new URLSearchParams(location.hash.slice(1));
if (STATES.some(s => s[0] === params.get('state'))) currentState = params.get('state');
if (SIZES[params.get('size')]) currentSize = params.get('size');
if (params.get('bubble') === '0') showBubble = false;
if (params.get('drag') === '1') isDragging = true;
if (params.get('blink') === '1') isBlinking = true;
if (params.get('poke') === '1') isPoked = true;
if (params.get('still') === '1') isStill = true;
if (params.get('edit') === '1') editMode = true;
if (params.get('bg') === 'light') lightBg = true;
// 无头浏览器无法移动鼠标，用 hash 直接注入视线变量做可复现截图
const gx = parseFloat(params.get('gx') || '0');
const gy = parseFloat(params.get('gy') || '0');
const gn = parseFloat(params.get('near') || '0');
const gazePreset = Number.isFinite(gx) && Number.isFinite(gy) && (gx || gy || gn);

const iframe = document.getElementById('pet');
const win = document.getElementById('win');
const desktop = document.getElementById('desktop');

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
  const flags = { bubbleBtn: showBubble, blinkBtn: isBlinking, pokeBtn: isPoked,
                  dragBtn: isDragging, editBtn: editMode, stillBtn: isStill, bgBtn: lightBg };
  Object.entries(flags).forEach(([id, on]) => document.getElementById(id).classList.toggle('on', on));
}

function frameDoc() {
  const loops = LOOPS.map(k => '<span class="pet-loop" data-loop="' + k + '" aria-hidden="true"></span>').join('');
  const motes = [1,2,3,4,5,6].map(() => '<i class="pet-mote"></i>').join('');
  return '<!doctype html><html><head><meta charset="utf-8"><style>'
    + 'html,body{margin:0;padding:0;width:100%;height:100%;background:transparent;overflow:hidden}'
    + PET_CSS
    + '</style></head><body>'
    + '<div class="pet-stage interactive" data-state="idle" id="stage">'
    + '<div class="pet-ambient" aria-hidden="true"><span class="pet-halo"></span><span class="pet-ground"></span>'
    + motes + '<i class="pet-spark s1"></i><i class="pet-spark s2"></i><i class="pet-spark s3"></i></div>'
    + '<div class="pet-side">'
    + '<div class="pet-bubble pet-bubble-ok" id="bubble" role="status"></div>'
    + '<div class="pet-status-bar" role="status"><span class="pet-status-dot"></span><span class="pet-status-text" id="label"></span></div>'
    + '</div>'
    + '<div class="pet-figure" id="figure"><div class="pet-motion"><div class="pet-loops">'
    + loops + '<span class="pet-poster" id="poster" data-frame="idle" aria-hidden="true"></span>'
    + '<span class="pet-blink" aria-hidden="true"></span></div>'
    + '<span class="pet-gaze"></span></div></div>'
    + '<div class="pet-edit-bar" id="editbar" style="display:none"><button type="button" class="overlay-save">保存</button><span class="overlay-grip"></span></div>'
    + '</div><script>'
    // iframe 内的指针事件不会冒泡到父页面，这里复刻产品侧视线映射，
    // 保证鼠标移到角色身上时也能看到转头效果。
    + 'document.addEventListener("pointermove",function(e){'
    + 'var s=document.getElementById("stage"),f=document.getElementById("figure");'
    + 'if(!s||!f)return;var r=f.getBoundingClientRect();'
    + 'var cx=r.left+r.width*0.55,cy=r.top+r.height*0.32;'
    + 'var d=Math.hypot(e.clientX-cx,e.clientY-cy);'
    + 'var n=Math.max(0,Math.min(1,1-d/170));'
    + 's.style.setProperty("--gaze-x",Math.max(-1,Math.min(1,(e.clientX-cx)/(r.width*0.9))).toFixed(3));'
    + 's.style.setProperty("--gaze-y",Math.max(-1,Math.min(1,(e.clientY-cy)/(r.height*0.6))).toFixed(3));'
    + 's.style.setProperty("--near",n.toFixed(3));'
    + 's.classList.toggle("nearby",n>0.12);});'
    // 父页脚本内不能直接写 script 结束标签：单反斜杠转义可避免 HTML 解析器提前截断，
    // JS 求值后 srcdoc 里会还原成合法标签。若误写成两个反斜杠，求值后仍是转义串，
    // iframe 内脚本整段不执行，「鼠标移到角色身上会转头」会静默失效。
    + '<\/script></body></html>';
}

function applyState() {
  const doc = iframe.contentDocument;
  if (!doc || !doc.getElementById('stage')) return;
  const entry = STATES.find(s => s[0] === currentState);
  const stage = doc.getElementById('stage');
  stage.dataset.state = currentState;
  if (gazePreset) {
    stage.style.setProperty('--gaze-x', String(gx));
    stage.style.setProperty('--gaze-y', String(gy));
    stage.style.setProperty('--near', String(gn));
    stage.classList.toggle('nearby', gn > 0.12);
  }
  stage.classList.toggle('blinking', isBlinking);
  stage.classList.toggle('still', isStill);
  const frame = isPoked ? BUBBLES.poke : BUBBLES[currentState];
  doc.querySelectorAll('.pet-loop').forEach(el => el.classList.toggle('is-active', el.dataset.loop === entry[2]));
  const poster = doc.getElementById('poster');
  poster.dataset.frame = entry[2];
  poster.classList.toggle('is-active', isStill);
  doc.getElementById('label').textContent = entry[1];
  const bubble = doc.getElementById('bubble');
  bubble.textContent = frame[0];
  bubble.className = 'pet-bubble pet-bubble-' + frame[1];
  bubble.style.display = showBubble ? '' : 'none';
  const figure = doc.getElementById('figure');
  figure.classList.toggle('dragging', isDragging);
  figure.classList.toggle('poked', isPoked);
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
  document.getElementById('readout').textContent =
    '窗口 ' + w + ' × ' + h + ' px　预览放大 ' + scale + '×　素材 240×360 @2x';
}

// 预览页把桌面区域的鼠标位置映射成产品的视线变量，可直接验收转头效果
desktop.addEventListener('pointermove', (event) => {
  const doc = iframe.contentDocument;
  const rect = win.getBoundingClientRect();
  if (!doc || !doc.getElementById('stage')) return;
  const cx = rect.left + rect.width * 0.72;
  const cy = rect.top + rect.height * 0.34;
  const distance = Math.hypot(event.clientX - cx, event.clientY - cy);
  const near = Math.max(0, Math.min(1, 1 - distance / 260));
  const x = Math.max(-1, Math.min(1, (event.clientX - cx) / (rect.width * 0.8)));
  const y = Math.max(-1, Math.min(1, (event.clientY - cy) / (rect.height * 0.7)));
  const stage = doc.getElementById('stage');
  stage.style.setProperty('--gaze-x', x.toFixed(3));
  stage.style.setProperty('--gaze-y', y.toFixed(3));
  stage.style.setProperty('--near', near.toFixed(3));
  stage.classList.toggle('nearby', near > 0.12);
});
desktop.addEventListener('pointerleave', () => {
  const doc = iframe.contentDocument;
  if (!doc || !doc.getElementById('stage')) return;
  const stage = doc.getElementById('stage');
  stage.style.setProperty('--gaze-x', '0');
  stage.style.setProperty('--gaze-y', '0');
  stage.style.setProperty('--near', '0');
});

function bindToggles() {
  const toggle = (id, get, set) => {
    document.getElementById(id).onclick = () => { set(!get()); syncButtons(); applyState(); };
  };
  toggle('bubbleBtn', () => showBubble, v => { showBubble = v; });
  toggle('blinkBtn', () => isBlinking, v => { isBlinking = v; });
  toggle('pokeBtn', () => isPoked, v => { isPoked = v; });
  toggle('dragBtn', () => isDragging, v => { isDragging = v; });
  toggle('editBtn', () => editMode, v => { editMode = v; });
  toggle('stillBtn', () => isStill, v => { isStill = v; });
  document.getElementById('bgBtn').onclick = () => {
    lightBg = !lightBg;
    desktop.classList.toggle('light', lightBg);
    syncButtons();
  };
}

buildButtons();
bindToggles();
desktop.classList.toggle('light', lightBg);
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


def validate_frame_script(html: str) -> None:
    """校验 iframe 内脚本闭合，防止「转头交互静默失效」回归。

    父页脚本里必须是单反斜杠转义的结束标签：JS 求值后 srcdoc 得到合法的
    script 结束标签。若误写成两个反斜杠，求值后仍是转义串，iframe 内的
    script 不会闭合，整段 JS 不执行，预览页的视线跟随会无声失效。
    """
    if "<\\\\/script>" in html:
        raise ValueError("iframe 内 script 结束标签被双重转义，srcdoc 脚本不会执行")
    if "<\\/script>" not in html:
        raise ValueError("iframe 内 script 结束标签缺失，父页脚本可能被截断")
    if 'document.addEventListener("pointermove"' not in html:
        raise ValueError("预览页缺少 iframe 内视线映射复刻")


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

    # 把相对图片路径换成内联 base64，保证预览页单文件离线可用；
    # 动画 WebP 同样整体内联，iframe 里会真实播放。
    replaced: list[str] = []
    total_kb = 0.0

    def inline_image(match: re.Match) -> str:
        nonlocal total_kb
        name = match.group(1)
        path = pet_dir / f"{name}.webp"
        data = path.read_bytes()
        replaced.append(f"{name}.webp ({len(data)} B)")
        total_kb += len(data) / 1024
        encoded = base64.b64encode(data).decode("ascii")
        return f"url('data:image/webp;base64,{encoded}')"

    pet_css = URL_PATTERN.sub(inline_image, pet_css)
    if "url('./assets/pet/" in pet_css:
        raise ValueError("仍有未内联的素材路径")
    names = {item.split(" ")[0] for item in replaced}
    expected = {f"{name}.webp" for name in FRAME_NAMES}
    if names != expected:
        raise ValueError(f"素材集合异常: 缺 {sorted(expected - names)} / 多 {sorted(names - expected)}")

    generated_at = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    css_hash = hashlib.sha256(css.encode("utf-8")).hexdigest()[:12]
    html = (
        TEMPLATE.replace("__PET_CSS__", json.dumps(pet_css, ensure_ascii=False))
        .replace("__GENERATED_AT__", generated_at)
        .replace("__CSS_HASH__", css_hash)
        .replace("__ASSET_COUNT__", str(len(FRAME_NAMES)))
        .replace("__ASSET_KB__", f"{total_kb:.1f}")
    )
    validate_frame_script(html)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html, encoding="utf-8")

    return {
        "generated_at": generated_at,
        "css_path": css_path.relative_to(root).as_posix(),
        "css_hash": css_hash,
        "selectors": list(REQUIRED_SELECTORS),
        "assets": replaced,
        "asset_kb": total_kb,
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
        f"- 内联素材 {len(info['assets'])} 张 / {info['asset_kb']:.1f} KB（含 5 个带 alpha 的动画 WebP）：",
    ]
    lines.extend(f"  - `{name}`" for name in info["assets"])
    lines += ["", "结论：预览页与产品精灵 CSS 同源，素材全部内联，单文件离线可用。", ""]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main() -> int:
    """入口：解析参数 → 生成预览 → 写留痕 → 返回退出码。"""
    args = parse_args()
    root = args.root.resolve()
    out = (args.out or root / "docs" / "design" / "pet-v07-preview.html").resolve()
    trace_dir = (args.trace_dir or root / "docs" / "孙测测" / "traces").resolve()
    try:
        info = build_preview(root, out)
        trace = write_trace(trace_dir, info)
    except (OSError, ValueError) as exc:
        print(f"[失败] {exc}", file=sys.stderr)
        return 2
    print(f"[通过] 预览页：{out}")
    print(f"[通过] 源 CSS sha256 前 12 位：{info['css_hash']}")
    print(f"[通过] 内联素材 {len(info['assets'])} 张 / {info['asset_kb']:.1f} KB，"
          f"关键选择器 {len(info['selectors'])} 个")
    print(f"[通过] 留痕：{trace}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
