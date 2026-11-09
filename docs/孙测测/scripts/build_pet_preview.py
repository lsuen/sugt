#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""编写人员：孙测测
作用用途：从 src/pet/pet.css 抽取桌面精灵样式，内联真实 WebP 素材（含带 alpha 的
         动画 WebP），生成可点、可鼠标交互的离线演示页；同时校验关键选择器齐全，
         避免演示页与产品样式漂移。
适用项目：SUGT（桌面精灵 小su）v1.0
创建日期：2026-11-08（v0.7 于同日重写；2026-11-09 同步 v0.9 十动作 / v1.0 互动层）
复用方式：python3 docs/孙测测/scripts/build_pet_preview.py [--root 项目根] [--out 输出HTML]；
          每次修改精灵 CSS 或素材后重跑即可，无需手工维护演示页。
退出码约定：0 通过 / 1 校验不通过 / 2 读取或解析失败
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import re
import sys
from datetime import datetime
from pathlib import Path

# 预览页必须包含的 v1.0 布局与互动反馈选择器；缺一个就说明 CSS 被改坏或版本回退
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
    ".pet-thought",
    ".pet-stage.petted .pet-figure::after",
    ".pet-stage.roaming .pet-loops",
    ".pet-stage.pet-landed .pet-motion",
)
# v1.0 素材清单未变：10 张静态海报 + 1 张程序化闭眼帧 + 10 个带 alpha 的动画 WebP
FRAME_NAMES = (
    "su-idle", "su-typing", "su-done", "su-error", "su-sleep", "su-blink",
    "su-wave", "su-peek", "su-shy", "su-celebrate", "su-walk",
    "su-idle-loop", "su-typing-loop", "su-done-loop", "su-error-loop", "su-sleep-loop",
    "su-wave-loop", "su-peek-loop", "su-shy-loop", "su-celebrate-loop", "su-walk-loop",
)
CSS_ANCHOR = "/* ===== 桌面精灵"
# pet.css 相对路径为 ../assets/pet/；兼容旧版 overlay.css 的 ./assets/pet/
URL_PATTERN = re.compile(r"url\('(?:\.\./)*assets/pet/([a-z0-9-]+)\.webp'\)")

TEMPLATE = r"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>小su 桌面精灵 v1.0 · 互动演示</title>
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
.now{margin-top:14px;padding:9px 11px;border:1px solid rgba(93,217,168,.35);border-radius:9px;background:rgba(93,217,168,.08);font-size:12.5px;color:var(--accent)}
/* 580px：最小档 148×168 放大 3.2× 后高 537px，旧值 460px 会裁掉脚底状态胶囊。 */
.desktop{background:radial-gradient(circle at 25% 18%,#243450 0%,#141B2C 55%,#0C111D 100%);border:1px solid var(--border);border-radius:14px;height:580px;display:flex;align-items:center;justify-content:center;position:relative;overflow:hidden;transition:background .3s}
.desktop.light{background:linear-gradient(135deg,#DCE6F2 0%,#B9CBDD 55%,#8FA6BF 100%)}
.desktop::before{content:"移动鼠标：小su 会转头看你 · 点角色头部/身体也有反应 · 虚线为透明窗口边界";position:absolute;left:14px;top:12px;font-size:11px;color:#8494AE}
.desktop.light::before{color:#4C5C72}
.win{border:1px dashed rgba(140,200,232,.35);border-radius:4px;position:relative;transform-origin:center center;transition:transform .42s cubic-bezier(.22,.8,.3,1)}
.win .pet-stage{width:100%;height:100%}
.readout{font-size:11.5px;color:var(--dim);margin-top:10px;text-align:right}
table{width:100%;border-collapse:collapse;font-size:12.5px;margin-top:24px}
th,td{text-align:left;padding:8px 10px;border-bottom:1px solid var(--border);vertical-align:top}
th{color:var(--dim);font-weight:500}td{color:var(--muted)}
td:first-child{color:var(--text);white-space:nowrap}
footer{margin-top:30px;font-size:11.5px;color:var(--dim)}
code{background:rgba(255,255,255,.06);border-radius:4px;padding:1px 5px;font-size:11.5px}
__PET_CSS_RAW__
</style>
</head>
<body>
<div class="wrap">
<h1>小su 桌面精灵 v1.0 · 互动演示</h1>
<p class="lead">样式与素材直接取自 <code>src/pet/pet.css</code>：11 张静态海报 + 10 个带透明通道的动画 WebP 全部内联，单文件离线可用。v1.0 演示页与产品精灵在<strong>同一文档</strong>中渲染（不再用 srcdoc iframe），每个互动按钮点一次就播一段 1.5~2.6s 的完整动作：换姿势、出气泡、改脚底状态文字，走动还会真的位移，播完自动回到当前状态；再点一次可重播。「自动演示」会依次播放全部动作。</p>
<span class="badge">生成时间 __GENERATED_AT__</span>
<span class="badge">源 CSS sha256 __CSS_HASH__</span>
<span class="badge">素材 __ASSET_COUNT__ 张 / __ASSET_KB__ KB（含 10 个带 alpha 动画 WebP）</span>

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
      <label>显示开关（点一下保持）</label>
      <div class="btnrow">
        <button type="button" id="bubbleBtn">气泡</button>
        <button type="button" id="dragBtn">拖动中</button>
        <button type="button" id="editBtn">编辑态</button>
        <button type="button" id="stillBtn">静态帧</button>
        <button type="button" id="bgBtn">浅色桌面</button>
      </div>
    </div>
    <div class="group">
      <label>互动演示（点一下播一段）</label>
      <div class="btnrow">
        <button type="button" id="helloBtn" data-action="hello">开场招呼</button>
        <button type="button" id="patBtn" data-action="pat">摸头</button>
        <button type="button" id="pokeBtn" data-action="poke">轻碰招手</button>
        <button type="button" id="blinkBtn" data-action="blink">眨眼</button>
        <button type="button" id="thinkBtn" data-action="think">思考云</button>
        <button type="button" id="roamBtn" data-action="roam">自主走动</button>
        <button type="button" id="landBtn" data-action="land">落地回弹</button>
      </div>
    </div>
    <div class="group">
      <label>批量</label>
      <div class="btnrow">
        <button type="button" id="demoBtn">▶ 自动演示全部</button>
        <button type="button" id="resetBtn">回到待机</button>
      </div>
    </div>
    <div class="now" id="now">当前：待机中</div>
    <p class="hint">鼠标在右侧桌面移动可看到以头部为轴的转头、眼神高光与靠近光晕；<strong>直接点小酥的头会害羞冒心形，点身体会招手</strong>。状态胶囊固定在角色脚底，气泡在左上方自适应换行。产品侧的真实计时、打断与显示器夹边逻辑以 <code>src/overlay.tsx</code> / <code>src-tauri/src/overlay.rs</code> 为准。</p>
  </div>

  <div>
    <div class="desktop" id="desktop">
      <div class="win" id="win">
        <div class="pet-stage interactive" id="stage" data-state="idle">
          <div class="pet-ambient" aria-hidden="true">
            <span class="pet-halo"></span><span class="pet-ground"></span>
            <i class="pet-mote"></i><i class="pet-mote"></i><i class="pet-mote"></i>
            <i class="pet-mote"></i><i class="pet-mote"></i><i class="pet-mote"></i>
            <i class="pet-spark s1"></i><i class="pet-spark s2"></i><i class="pet-spark s3"></i>
          </div>
          <div class="pet-side">
            <div class="pet-bubble pet-bubble-ok" id="bubble" role="status"></div>
            <div class="pet-status-bar" role="status"><span class="pet-status-dot"></span><span class="pet-status-text" id="label"></span></div>
          </div>
          <div class="pet-thought" id="thought" aria-hidden="true" style="display:none"><i></i><i></i><i></i></div>
          <div class="pet-figure" id="figure">
            <div class="pet-motion">
              <div class="pet-loops">
                <span class="pet-loop" data-loop="idle" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="typing" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="done" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="error" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="sleep" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="wave" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="peek" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="shy" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="celebrate" aria-hidden="true"></span>
                <span class="pet-loop" data-loop="walk" aria-hidden="true"></span>
                <span class="pet-poster" id="poster" data-frame="idle" aria-hidden="true"></span>
                <span class="pet-blink" aria-hidden="true"></span>
              </div>
              <span class="pet-gaze"></span>
            </div>
          </div>
          <div class="pet-edit-bar" id="editbar" style="display:none"><button type="button" class="overlay-save">保存</button><span class="overlay-grip"></span></div>
        </div>
      </div>
    </div>
    <div class="readout" id="readout"></div>
  </div>
</div>

<table>
<tr><th>v1.0 互动</th><th>演示中可验证</th></tr>
<tr><td>触发式动作</td><td>每个按钮点一次播 1.5~2.6s：换姿势 + 气泡 + 脚底状态文字 + 一次性动效，播完自动复位；再点可重播，不会「闪一下就没了」</td></tr>
<tr><td>自动演示</td><td>「▶ 自动演示全部」依次播放招呼 → 摸头 → 轻碰 → 眨眼 → 思考云 → 散步 → 落地，无需逐个点</td></tr>
<tr><td>点角色本体</td><td>点头部害羞并冒心形，点身体招手；与产品侧轻碰/摸头同一套差分素材</td></tr>
<tr><td>头转向鼠标</td><td>transform-origin 在脸中心（55% / 32%），幅度 9deg；rAF 插值，鼠标停下不会出现「人还在慢慢追」的双重缓动</td></tr>
<tr><td>气泡与状态胶囊</td><td>气泡左上 <code>max-content</code> 自适应、长文案自然换行；状态胶囊居中固定在角色脚底</td></tr>
<tr><td>自主走动</td><td>演示里窗口真的左右位移并播 walk 逐帧动画；产品侧只在 idle 且久无操作时小范围缓动，后端按当前显示器硬夹边</td></tr>
<tr><td>请求级思考云</td><td>typing 时只有三个装饰点、不吐文案；状态仍以后端真实在途请求为准，结果级事件才出泡</td></tr>
<tr><td>单层背景红线</td><td>10 个姿势各只挂一张动画 WebP + 一张静态海报，带 alpha 的透明区域不会露出第二层立绘</td></tr>
</table>

<footer>由 <code>docs/孙测测/scripts/build_pet_preview.py</code> 生成，请勿手工编辑；修改精灵 CSS 或互动层后重跑脚本即可。预览页不参与打包。</footer>
</div>

<script>
const STATES = [
  ['idle', '待机中', 'idle'], ['typing', '处理中', 'typing'], ['done', '已完成', 'done'],
  ['error', '出错了', 'error'], ['sleep', '休息中', 'sleep'], ['offline', '未运行', 'idle'],
  ['wave', '轻碰招手', 'wave'], ['peek', '靠近倾听', 'peek'], ['shy', '害羞', 'shy'],
  ['celebrate', '庆祝', 'celebrate'], ['walk', '走近', 'walk'],
];
const SIZES = { small: [148, 168, 3.2], medium: [180, 204, 2.6], large: [220, 250, 2.1] };
const BUBBLES = {
  idle: ['网关已就绪', 'ok'], typing: ['上游已切换', 'info'], done: ['完成 3 次请求', 'ok'],
  error: ['有请求失败', 'error'], sleep: ['小su 只播报真实状态', 'info'], offline: ['网关未运行', 'info'],
  wave: ['嗯？我在呢～', 'info'], peek: ['在听在听～', 'info'], shy: ['别、别盯着看…', 'info'],
  celebrate: ['完成啦！', 'ok'], walk: ['我过来啦～', 'info'],
  poke: ['嗯？我在呢～', 'info'], hello: ['我来啦～', 'info'], wake: ['嗯…醒啦～', 'info'],
  idle1: ['在呢，陪着你', 'info'], idle2: ['要不要摸摸头？', 'info'],
  idle3: ['伸个懒腰～', 'info'], pat: ['嘿嘿…', 'info'],
  land: ['放好啦', 'info'], roam: ['出去溜达一圈～', 'info'],
};
// 每个动作都是「一段可观察的表演」，不是常驻开关：pose / 气泡 / 状态文字 / CSS 反馈类
// / 持续时间 / 是否真的移动窗口。pose 为 null 表示沿用当前状态的立绘。
const ACTIONS = {
  hello: { label: '挥手打招呼', pose: 'wave', bubble: 'hello', ms: 2200 },
  pat:   { label: '被摸头啦', pose: 'shy', bubble: 'pat', cls: 'petted', ms: 2000 },
  poke:  { label: '轻碰招手', pose: 'wave', bubble: 'poke', fig: 'poked', ms: 1800 },
  blink: { label: '眨了下眼', pose: null, bubble: null, cls: 'blinking', ms: 320 },
  think: { label: '在想事情…', pose: null, bubble: null, thought: true, ms: 2200 },
  roam:  { label: '散步中', pose: 'walk', bubble: 'roam', cls: 'roaming', ms: 2600, move: true },
  land:  { label: '放下啦', pose: null, bubble: 'land', cls: 'pet-landed', ms: 1500 },
};

let currentState = 'idle';
let currentSize = 'small';
let showBubble = true;
let isDragging = false;
let isStill = false;
let editMode = false;
let lightBg = false;
let currentAction = null;
let actionTimer = 0;
let moveTimers = [];
let demoTimers = [];

const win = document.getElementById('win');
const desktop = document.getElementById('desktop');
const stage = document.getElementById('stage');
const figure = document.getElementById('figure');
const bubble = document.getElementById('bubble');
const label = document.getElementById('label');
const thought = document.getElementById('thought');
const poster = document.getElementById('poster');
const editbar = document.getElementById('editbar');
const now = document.getElementById('now');
const readout = document.getElementById('readout');

const params = new URLSearchParams(location.hash.slice(1));
if (STATES.some(s => s[0] === params.get('state'))) currentState = params.get('state');
if (SIZES[params.get('size')]) currentSize = params.get('size');
if (params.get('bubble') === '0') showBubble = false;
if (params.get('drag') === '1') isDragging = true;
if (params.get('still') === '1') isStill = true;
if (params.get('edit') === '1') editMode = true;
if (params.get('bg') === 'light') lightBg = true;
const gx = parseFloat(params.get('gx') || '0');
const gy = parseFloat(params.get('gy') || '0');
const gn = parseFloat(params.get('near') || '0');
const gazePreset = Number.isFinite(gx) && Number.isFinite(gy) && (gx || gy || gn);

function stateEntry() { return STATES.find(s => s[0] === currentState); }
function clamp(v, lo, hi) { return Math.max(lo, Math.min(hi, v)); }

function setPose(pose) {
  document.querySelectorAll('.pet-loop').forEach(
    el => el.classList.toggle('is-active', el.dataset.loop === pose));
  poster.dataset.frame = pose || stateEntry()[2];
}

function setBubble(key) {
  if (!key) { bubble.style.display = 'none'; return; }
  const frame = BUBBLES[key];
  if (!frame) { bubble.style.display = 'none'; return; }
  bubble.textContent = frame[0];
  bubble.className = 'pet-bubble pet-bubble-' + frame[1];
  bubble.style.display = showBubble ? '' : 'none';
  // 每次出泡都重播入场动画，连续点同一个动作也有可见反馈
  bubble.style.animation = 'none';
  void bubble.offsetWidth;
  bubble.style.animation = '';
}

function updateNow(text) {
  const [w, h, s] = SIZES[currentSize];
  now.textContent = text || ('当前：' + stateEntry()[1]);
  readout.textContent = '窗口 ' + w + ' × ' + h + ' px　预览放大 ' + s + '×　素材 240×360 @2x';
}

function syncButtons() {
  document.querySelectorAll('#stateBtns button').forEach(
    b => b.classList.toggle('on', b.dataset.key === currentState));
  document.querySelectorAll('#sizeBtns button').forEach(
    b => b.classList.toggle('on', b.dataset.key === currentSize));
  const toggles = { bubbleBtn: showBubble, dragBtn: isDragging, editBtn: editMode,
                    stillBtn: isStill, bgBtn: lightBg };
  Object.entries(toggles).forEach(
    ([id, on]) => document.getElementById(id).classList.toggle('on', on));
  document.querySelectorAll('[data-action]').forEach(
    b => b.classList.toggle('on', b.dataset.action === currentAction));
  document.getElementById('demoBtn').classList.toggle('on', demoTimers.length > 0);
}

function applyBase() {
  stage.dataset.state = currentState;
  stage.classList.toggle('still', isStill);
  figure.classList.toggle('dragging', isDragging);
  editbar.style.display = editMode ? '' : 'none';
  setPose(stateEntry()[2]);
  // 静态降级时显示对应姿势的海报（reduced-motion / 窗口隐藏）
  poster.classList.toggle('is-active', isStill);
  label.textContent = stateEntry()[1];
  setBubble(currentState);
  thought.style.display = 'none';
  updateNow();
  syncButtons();
}

function stopDemo() {
  demoTimers.forEach(clearTimeout);
  demoTimers = [];
}

function clearAction() {
  window.clearTimeout(actionTimer);
  actionTimer = 0;
  moveTimers.forEach(clearTimeout);
  moveTimers = [];
  stage.classList.remove('petted', 'roaming', 'pet-landed', 'blinking');
  figure.classList.remove('poked');
  thought.style.display = 'none';
  win.style.transform = 'scale(' + SIZES[currentSize][2] + ')';
  currentAction = null;
}

function resetAction() {
  clearAction();
  applyBase();
}

function startWalk(ms) {
  const desktopRect = desktop.getBoundingClientRect();
  const winRect = win.getBoundingClientRect();
  const limit = clamp(desktopRect.width / 2 - winRect.width / 2 - 14, 24, 96);
  const steps = [-limit * 0.65, limit * 0.55, -limit * 0.2, 0];
  steps.forEach((dx, i) => {
    moveTimers.push(window.setTimeout(() => {
      win.style.transform = 'translateX(' + dx.toFixed(1) + 'px) scale(' + SIZES[currentSize][2] + ')';
    }, 120 + i * (ms - 240) / steps.length));
  });
}

// 一次性 CSS 动画只在 class 首次添加时播放；先移除 → 强制回流 → 再加，
// 保证同一个按钮连点、不同动作切换都能重播，不会「闪一下就静止」。
function playAction(name, fromDemo, hold) {
  const action = ACTIONS[name];
  if (!action) return;
  if (!fromDemo) stopDemo();
  clearAction();
  currentAction = name;
  void stage.offsetWidth;
  if (action.cls) stage.classList.add(action.cls);
  if (action.fig) figure.classList.add(action.fig);
  thought.style.display = action.thought ? '' : 'none';
  setPose(action.pose || stateEntry()[2]);
  label.textContent = action.label;
  setBubble(action.bubble || null);
  updateNow('正在演示：' + action.label + '（' + (action.ms / 1000).toFixed(1) + 's 后自动回到「' + stateEntry()[1] + '」）');
  if (action.move) startWalk(action.ms);
  syncButtons();
  if (!hold) actionTimer = window.setTimeout(resetAction, action.ms);
}

function playAll() {
  stopDemo();
  resetAction();
  const sequence = ['hello', 'pat', 'poke', 'blink', 'think', 'roam', 'land'];
  sequence.forEach((name, i) => {
    demoTimers.push(window.setTimeout(() => playAction(name, true), i * 2400));
  });
  demoTimers.push(window.setTimeout(() => {
    demoTimers = [];
    syncButtons();
  }, sequence.length * 2400));
  updateNow('自动演示中：依次播放 7 段互动…');
  syncButtons();
}

function renderFrame() {
  const [w, h, s] = SIZES[currentSize];
  win.style.width = w + 'px';
  win.style.height = h + 'px';
  win.style.transform = 'scale(' + s + ')';
  updateNow();
}

// 视线跟随：与产品侧一致用 rAF 插值写 CSS 变量，CSS 不再叠加 transition，
// 避免「鼠标停了角色还在慢慢追」的双重缓动。
let gazeTarget = { x: 0, y: 0, near: 0 };
let gazeNow = { x: 0, y: 0, near: 0 };
let gazeRaf = 0;
function writeGaze() {
  stage.style.setProperty('--gaze-x', gazeNow.x.toFixed(3));
  stage.style.setProperty('--gaze-y', gazeNow.y.toFixed(3));
  stage.style.setProperty('--near', gazeNow.near.toFixed(3));
  stage.classList.toggle('nearby', gazeNow.near > 0.12);
  const listening = !currentAction && currentState === 'idle' && gazeNow.near > 0.45;
  const walking = !currentAction && currentState === 'idle'
    && gazeNow.near > 0.18 && gazeNow.near <= 0.45;
  stage.classList.toggle('pet-listening', listening);
  stage.classList.toggle('pet-walking', walking);
}
function gazeTick() {
  const lerp = 0.18;
  gazeNow.x += (gazeTarget.x - gazeNow.x) * lerp;
  gazeNow.y += (gazeTarget.y - gazeNow.y) * lerp;
  gazeNow.near += (gazeTarget.near - gazeNow.near) * lerp;
  writeGaze();
  const delta = Math.abs(gazeTarget.x - gazeNow.x) + Math.abs(gazeTarget.y - gazeNow.y)
    + Math.abs(gazeTarget.near - gazeNow.near);
  gazeRaf = delta > 0.004 ? requestAnimationFrame(gazeTick) : 0;
}
function aim(x, y, near) {
  gazeTarget = { x, y, near };
  if (!gazeRaf) gazeRaf = requestAnimationFrame(gazeTick);
}

desktop.addEventListener('pointermove', (event) => {
  const rect = win.getBoundingClientRect();
  const cx = rect.left + rect.width * 0.72;
  const cy = rect.top + rect.height * 0.34;
  const distance = Math.hypot(event.clientX - cx, event.clientY - cy);
  const near = clamp(1 - distance / 280, 0, 1);
  aim(clamp((event.clientX - cx) / (rect.width * 0.9), -1, 1),
      clamp((event.clientY - cy) / (rect.height * 0.6), -1, 1), near);
});
desktop.addEventListener('pointerleave', () => aim(0, 0, 0));

// 直接点角色本体：头部害羞冒心形，身体招手。同文档渲染，无需 iframe 消息桥。
figure.addEventListener('pointerdown', (event) => {
  const rect = figure.getBoundingClientRect();
  const isHead = (event.clientY - rect.top) < rect.height * 0.42;
  playAction(isHead ? 'pat' : 'poke');
});

function buildButtons() {
  const sb = document.getElementById('stateBtns');
  STATES.forEach(([key, label]) => {
    const b = document.createElement('button');
    b.type = 'button'; b.textContent = label; b.dataset.key = key;
    b.onclick = () => { stopDemo(); resetAction(); currentState = key; applyBase(); };
    sb.appendChild(b);
  });
  const zb = document.getElementById('sizeBtns');
  Object.keys(SIZES).forEach((key, i) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = ['小 148×168', '中 180×204', '大 220×250'][i];
    b.dataset.key = key;
    b.onclick = () => { currentSize = key; resetAction(); renderFrame(); };
    zb.appendChild(b);
  });
}

function bindControls() {
  document.querySelectorAll('[data-action]').forEach(
    b => { b.onclick = () => playAction(b.dataset.action); });
  const toggle = (id, get, set) => {
    document.getElementById(id).onclick = () => {
      set(!get());
      applyBase();
    };
  };
  toggle('bubbleBtn', () => showBubble, v => { showBubble = v; });
  toggle('dragBtn', () => isDragging, v => { isDragging = v; });
  toggle('editBtn', () => editMode, v => { editMode = v; });
  toggle('stillBtn', () => isStill, v => { isStill = v; });
  document.getElementById('bgBtn').onclick = () => {
    lightBg = !lightBg;
    desktop.classList.toggle('light', lightBg);
    syncButtons();
  };
  document.getElementById('demoBtn').onclick = playAll;
  document.getElementById('resetBtn').onclick = () => { stopDemo(); resetAction(); };
}

buildButtons();
bindControls();
desktop.classList.toggle('light', lightBg);
renderFrame();
applyBase();
if (gazePreset) {
  gazeTarget = { x: gx, y: gy, near: gn };
  gazeNow = { x: gx, y: gy, near: gn };
  writeGaze();
}
const initialAction = params.get('action');
if (initialAction && ACTIONS[initialAction]) playAction(initialAction, false, true);
</script>
</body>
</html>
"""


def parse_args() -> argparse.Namespace:
    """解析命令行参数，默认路径全部由脚本位置推导，不硬编码本机绝对路径。"""
    default_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description="生成桌面精灵同源 CSS 离线互动演示页")
    parser.add_argument("--root", type=Path, default=default_root, help="项目根目录")
    parser.add_argument("--out", type=Path, default=None, help="输出 HTML 路径")
    parser.add_argument("--trace-dir", type=Path, default=None, help="留痕目录")
    return parser.parse_args()


def validate_interaction(html: str) -> None:
    """校验演示页是同文档渲染 + 触发式动作引擎，防止「按钮只闪一下」回归。

    旧版把精灵塞进 srcdoc iframe，父页靠 contentDocument 跨文档访问；
    预览面板若给 iframe 一个 opaque origin 或拦截内联脚本，按钮就会只亮一下
    而角色毫无变化。现在精灵 DOM 与按钮同文档，且每个动作必须能重播、复位。
    """
    if "<iframe" in html:
        raise ValueError("演示页不得使用 iframe：跨文档访问会让按钮静默失效")
    needles = (
        'id="stage"',
        'data-action="hello"',
        "function playAction(",
        "function resetAction(",
        "function playAll(",
        "function startWalk(",
        "pointermove",
        "pointerdown",
        "--gaze-x",
    )
    missing = [item for item in needles if item not in html]
    if missing:
        raise ValueError("演示页缺少互动链路: " + ", ".join(missing))


def build_preview(root: Path, out: Path) -> dict:
    """抽取精灵 CSS、内联素材、渲染演示页，返回本次生成的可核查信息。"""
    css_path = root / "src" / "pet" / "pet.css"
    pet_dir = root / "src" / "assets" / "pet"
    css = css_path.read_text(encoding="utf-8")

    anchor = css.find(CSS_ANCHOR)
    if anchor < 0:
        raise ValueError(f"未找到精灵样式锚点 {CSS_ANCHOR}")
    pet_css = css[anchor:]

    missing = [selector for selector in REQUIRED_SELECTORS if selector not in pet_css]
    if missing:
        raise ValueError("精灵 CSS 缺少关键选择器: " + ", ".join(missing))

    # 把相对图片路径换成内联 base64，保证演示页单文件离线可用；
    # 动画 WebP 同样整体内联，页面里会真实播放。
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
    leftover = re.findall(r"url\('(?:\.\./)*assets/pet/[^']+'\)", pet_css)
    if leftover:
        raise ValueError("仍有未内联的素材路径: " + ", ".join(leftover[:3]))
    names = {item.split(" ")[0] for item in replaced}
    expected = {f"{name}.webp" for name in FRAME_NAMES}
    if names != expected:
        raise ValueError(f"素材集合异常: 缺 {sorted(expected - names)} / 多 {sorted(names - expected)}")

    generated_at = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    css_hash = hashlib.sha256(css.encode("utf-8")).hexdigest()[:12]
    html = (
        TEMPLATE.replace("__PET_CSS_RAW__", pet_css)
        .replace("__GENERATED_AT__", generated_at)
        .replace("__CSS_HASH__", css_hash)
        .replace("__ASSET_COUNT__", str(len(FRAME_NAMES)))
        .replace("__ASSET_KB__", f"{total_kb:.1f}")
    )
    validate_interaction(html)
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
        "# 桌面精灵同源互动演示页生成留痕",
        "",
        f"- 生成时间：{info['generated_at']}",
        f"- 源样式：`{info['css_path']}`（sha256 前 12 位 `{info['css_hash']}`）",
        f"- 输出：`{info['output']}`（{info['bytes']} B）",
        "- 关键选择器校验：" + "、".join(f"`{s}`" for s in info["selectors"]) + " → 全部存在",
        "- 互动链路校验：同文档渲染（无 iframe）、触发式动作引擎、重播/复位、自动演示、走动位移、点击角色 → 全部存在",
        f"- 内联素材 {len(info['assets'])} 张 / {info['asset_kb']:.1f} KB（含 10 个带 alpha 的动画 WebP）：",
    ]
    lines.extend(f"  - `{name}`" for name in info["assets"])
    lines += ["", "结论：演示页与产品精灵 v1.0 CSS 同源，按钮点一次播一段动作并自动复位，素材全部内联，单文件离线可用。", ""]
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
    print(f"[通过] 演示页：{out}")
    print(f"[通过] 源 CSS sha256 前 12 位：{info['css_hash']}")
    print(f"[通过] 内联素材 {len(info['assets'])} 张 / {info['asset_kb']:.1f} KB，"
          f"关键选择器 {len(info['selectors'])} 个")
    print(f"[通过] 留痕：{trace}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
