#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵 v0.7 浏览器冒烟：真实时间验证动画与交互。

编写人员：孙测测
作用用途：无头 Chrome 的 --virtual-time-budget 会冻结图像动画时间线，
         静态截图无法证明「动画 WebP 真的在播」。本脚本用 Playwright 的
         真实等待逐帧采样，并把视线跟随、眨眼、静态降级、状态切换、
         轻碰反馈做成可判定断言，结果落 Markdown 留痕。
适用项目：SUGT 桌面精灵 v0.7
创建日期：2026-11-08
复用方式：python3 docs/孙测测/scripts/pet_v07_browser_smoke.py [--root 项目根]
          环境变量 SUGT_PLAYWRIGHT_MODULE 指向 playwright 的 index.mjs，
          SUGT_CHROME_PATH 指向 Chrome 可执行文件（默认本机 macOS 路径）
退出码约定：0 全部通过 / 1 存在不通过项 / 2 环境或读取解析失败
"""
import argparse
import datetime
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# 浏览器侧脚本：输出 JSON 数组，每项 (名称, 通过, 证据)
BROWSER_JS = r"""
import { chromium } from '__PLAYWRIGHT__';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

const args = JSON.parse(process.argv[2]);
const rows = [];
const check = (name, ok, detail) => rows.push([name, Boolean(ok), String(detail)]);
const hash = (buffer) => createHash('sha1').update(buffer).digest('hex').slice(0, 10);

let browser;
try {
  browser = await chromium.launch({
    headless: true,
    executablePath: args.chrome,
    args: ['--no-sandbox', '--disable-gpu'],
  });
  // 1) 动画 WebP 作为 CSS 背景，真实时间下必须出现多个不同帧。
  //    用只含背景图的极简页整页截图，避开 Playwright 对「元素稳定」的要求，
  //    也排除呼吸动画对帧差异的干扰。
  const animPage = await browser.newPage({ viewport: { width: 240, height: 360 } });
  await animPage.goto(pathToFileURL(args.animPage).href);
  const samples = [];
  for (let i = 0; i < 7; i += 1) {
    await animPage.waitForTimeout(170);
    samples.push(hash(await animPage.screenshot()));
  }
  const unique = new Set(samples).size;
  check('动画 WebP 逐帧播放', unique >= 3,
        `7 次采样得到 ${unique} 个不同帧：${samples.join(' ')}`);
  await animPage.close();

  const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  await page.goto(pathToFileURL(args.preview).href);
  await page.waitForTimeout(500);
  const frame = page.frameLocator('#pet');
  const stage = frame.locator('#stage');

  // 2) 视线跟随：两次移动都落在 iframe 内（产品侧监听 document.pointermove，
  //    预览页在 iframe 内复刻同一映射），验证 gaze 变量随鼠标变化。
  //    先预热一次移动，规避「首次进入 iframe 只触发 pointerenter」的时序抖动。
  const petBox = await page.locator('#pet').boundingBox();
  await page.mouse.move(petBox.x + petBox.width * 0.20, petBox.y + petBox.height * 0.24);
  await page.waitForTimeout(120);
  await page.mouse.move(petBox.x + petBox.width * 0.30, petBox.y + petBox.height * 0.32, { steps: 3 });
  await page.waitForTimeout(240);
  const first = await stage.evaluate(el => el.style.getPropertyValue('--gaze-x'));
  await page.mouse.move(petBox.x + petBox.width * 0.86, petBox.y + petBox.height * 0.68, { steps: 3 });
  await page.waitForTimeout(240);
  const second = await stage.evaluate(el => ({
    x: el.style.getPropertyValue('--gaze-x'),
    y: el.style.getPropertyValue('--gaze-y'),
    near: el.style.getPropertyValue('--near'),
  }));
  // 鼠标贴近头部时靠近光晕必须点亮
  const figBox = await frame.locator('#figure').boundingBox();
  await page.mouse.move(figBox.x + figBox.width * 0.55, figBox.y + figBox.height * 0.32, { steps: 3 });
  await page.waitForTimeout(220);
  const halo = await stage.evaluate(el => ({
    near: el.style.getPropertyValue('--near'),
    on: el.classList.contains('nearby'),
  }));
  check('视线跟随鼠标与靠近光晕',
        first !== '' && second.x !== '' && first !== second.x
        && Math.abs(Number(second.x) - Number(first)) > 0.05
        && halo.on && Number(halo.near) > 0.12,
        `gaze-x ${first} → ${second.x}，gaze-y ${second.y}，near ${second.near}；贴头 near ${halo.near} nearby=${halo.on}`);

  // 3) 眨眼帧叠加层
  await page.locator('#blinkBtn').click();
  await page.waitForTimeout(120);
  const blinkOpacity = await frame.locator('.pet-blink').evaluate(
    el => getComputedStyle(el).opacity);
  check('眨眼帧可显示', Number(blinkOpacity) > 0.9, `opacity=${blinkOpacity}`);
  await page.locator('#blinkBtn').click();

  // 4) 静态帧降级（reduced-motion / 窗口隐藏）
  await page.locator('#stillBtn').click();
  await page.waitForTimeout(120);
  const stillState = await frame.locator('#stage').evaluate(el => ({
    loop: getComputedStyle(el.querySelector('.pet-loop')).display,
    poster: getComputedStyle(el.querySelector('.pet-poster')).opacity,
  }));
  check('静态帧降级', stillState.loop === 'none' && Number(stillState.poster) > 0.9,
        `loop.display=${stillState.loop}, poster.opacity=${stillState.poster}`);
  await page.locator('#stillBtn').click();

  // 5) 状态切换只让对应动画层可见
  await page.locator('#stateBtns button[data-key="error"]').click();
  await page.waitForTimeout(120);
  const stateInfo = await frame.locator('#stage').evaluate(el => ({
    state: el.dataset.state,
    active: [...el.querySelectorAll('.pet-loop.is-active')].map(n => n.dataset.loop),
  }));
  check('状态切换命中动画层', stateInfo.state === 'error'
        && stateInfo.active.length === 1 && stateInfo.active[0] === 'error',
        `state=${stateInfo.state}, active=${stateInfo.active.join(',')}`);

  // 6) 轻碰反馈：一次性弹跳 + 轻声回应
  await page.locator('#pokeBtn').click();
  await page.waitForTimeout(120);
  const poke = await frame.locator('#stage').evaluate(el => ({
    poked: el.querySelector('#figure').classList.contains('poked'),
    bubble: el.querySelector('#bubble').textContent,
  }));
  check('轻碰反馈', poke.poked && poke.bubble.includes('嗯'),
        `poked=${poke.poked}, bubble=${poke.bubble}`);

  await page.screenshot({ path: args.screenshot });
  check('验收截图产出', true, args.screenshot);
} catch (error) {
  check('浏览器执行', false, String(error && error.stack || error));
} finally {
  if (browser) await browser.close();
}
console.log(JSON.stringify(rows));
"""


def parse_args() -> argparse.Namespace:
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description="桌面精灵 v0.7 浏览器冒烟")
    parser.add_argument("--root", type=Path, default=root)
    parser.add_argument("--out", type=Path, default=None, help="留痕路径")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    preview = root / "docs" / "design" / "pet-v07-preview.html"
    if not preview.is_file():
        print("预览页不存在，请先运行 build_pet_preview.py", file=sys.stderr)
        return 2
    playwright = os.environ.get(
        "SUGT_PLAYWRIGHT_MODULE",
        str(Path.home() / "node_modules" / "playwright" / "index.mjs"),
    )
    if not Path(playwright).is_file():
        print("未找到 Playwright: %s" % playwright, file=sys.stderr)
        return 2
    chrome = os.environ.get(
        "SUGT_CHROME_PATH",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    )
    if not Path(chrome).is_file():
        print("未找到 Chrome: %s" % chrome, file=sys.stderr)
        return 2
    screenshot = root / "docs" / "孙测测" / "traces" / "桌面精灵v0.7浏览器冒烟.png"
    screenshot.parent.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="sugt-pet-smoke-") as tmp:
        import base64
        loop_asset = root / "src" / "assets" / "pet" / "su-idle-loop.webp"
        encoded = base64.b64encode(loop_asset.read_bytes()).decode("ascii")
        anim_page = Path(tmp) / "anim-only.html"
        anim_page.write_text(
            "<!doctype html><html><head><meta charset='utf-8'><style>"
            "html,body{margin:0;background:#222;width:240px;height:360px}"
            "#a{width:240px;height:360px;background-image:url(data:image/webp;base64,"
            + encoded + ");background-size:contain;background-repeat:no-repeat}"
            "</style></head><body><div id='a'></div></body></html>",
            encoding="utf-8",
        )
        script = Path(tmp) / "smoke.mjs"
        script.write_text(BROWSER_JS.replace("__PLAYWRIGHT__", playwright), encoding="utf-8")
        payload = json.dumps({
            "preview": str(preview),
            "animPage": str(anim_page),
            "chrome": chrome,
            "screenshot": str(screenshot),
        }, ensure_ascii=False)
        try:
            result = subprocess.run(
                ["node", str(script), payload],
                check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180,
            )
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
            print("浏览器脚本执行失败: %s" % exc, file=sys.stderr)
            return 2
        try:
            rows = json.loads(result.stdout.decode("utf-8").strip().splitlines()[-1])
        except (ValueError, IndexError) as exc:
            print("解析浏览器结果失败: %s / %s" % (
                exc, result.stdout.decode("utf-8", "replace")[:500]), file=sys.stderr)
            return 2

    passed = sum(1 for row in rows if row[1])
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    trace = args.out or (root / "docs" / "孙测测" / "traces"
                         / ("桌面精灵v0.7浏览器冒烟-%s.md" % stamp))
    trace = Path(trace)
    trace.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# 桌面精灵 v0.7 浏览器冒烟留痕",
        "",
        "- 执行时间：%s" % datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 脚本：`docs/孙测测/scripts/pet_v07_browser_smoke.py`",
        "- 预览页：`docs/design/pet-v07-preview.html`（同源 CSS + 内联素材）",
        "- 驱动：Playwright + 本机 Chrome，真实时间逐帧采样（不用虚拟时间截图）",
        "- 结论：%s（%d/%d）" % ("通过" if passed == len(rows) else "不通过", passed, len(rows)),
        "",
        "| 检查项 | 结果 | 证据 |",
        "| --- | --- | --- |",
    ]
    lines += ["| %s | %s | %s |" % (name, "通过" if ok else "不通过", detail.replace("|", "\\|"))
              for name, ok, detail in rows]
    lines += ["", "验收截图：`%s`" % screenshot.relative_to(root).as_posix(), ""]
    trace.write_text("\n".join(lines), encoding="utf-8")
    for name, ok, detail in rows:
        print("[%s] %s — %s" % ("通过" if ok else "不通过", name, detail))
    print("汇总：%d/%d 通过，留痕 %s" % (passed, len(rows), trace))
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
