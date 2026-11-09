#!/usr/bin/env node
/**
 * 精灵素材契约跨平台冒烟：manifest v2 ↔ pet.css ↔ petManifest.generated.ts。
 * 纯 Node 标准库，不依赖 Python / ffmpeg / Pillow，因此可挂进 npm test 在 Windows CI 跑。
 * 运行：npm run test:pet-manifest
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.cwd();
const read = (rel) => readFileSync(join(root, rel), 'utf8');

let failed = 0;
function check(name, condition, detail) {
  if (condition) {
    console.log(`[ok] ${name}`);
  } else {
    console.error(`[fail] ${name}${detail ? ` — ${detail}` : ''}`);
    failed += 1;
  }
}

const manifest = JSON.parse(read('src/assets/pet/manifest.json'));
const css = read('src/pet/pet.css');
const generated = read('src/pet/petManifest.generated.ts');
const poses = manifest.poses.map((pose) => pose.name);

check('formatVersion 为 2', manifest.formatVersion === 2);
check('姿势名无重复', new Set(poses).size === poses.length);
check('cropBox 与 baseSize 自洽（透明边已裁掉）',
  Array.isArray(manifest.cropBox) && manifest.cropBox.length === 4
  && manifest.cropBox[2] - manifest.cropBox[0] === manifest.baseSize.w
  && manifest.cropBox[3] - manifest.cropBox[1] === manifest.baseSize.h
  && manifest.baseSize.w < manifest.sourceCanvas.w
  && manifest.baseSize.h < manifest.sourceCanvas.h,
  JSON.stringify(manifest.cropBox));
check('编码策略为调色板 + lossless',
  manifest.encoding?.mode === 'palette-lossless'
  && Number.isInteger(manifest.encoding?.colors) && manifest.encoding.colors >= 32);
check('自动交互节奏与动作优先级来自 manifest',
  Number.isFinite(manifest.autoActions?.blink?.minMs)
  && Array.isArray(manifest.autoActions?.idleVariant?.poses)
  && manifest.autoActions.idleVariant.poses.length > 0
  && Array.isArray(manifest.posePriority) && manifest.posePriority.length >= 10);
const live = manifest.autoActions?.liveliness;
const gcd = (a, b) => (b === 0 ? a : gcd(b, a % b));
check('活感层三层周期按 0.1s 为单位两两互质（读不出循环点）', (() => {
  const c = live?.cycles;
  if (!c) return false;
  const values = [c.breathMs, c.secondaryMs, c.weightMs];
  if (!values.every((v) => Number.isInteger(v) && v > 0 && v % 100 === 0)) return false;
  const [a, b, d] = values.map((v) => v / 100);
  return gcd(a, b) === 1 && gcd(a, d) === 1 && gcd(b, d) === 1;
})(), JSON.stringify(live?.cycles));
check('眼→头→身 lerp 严格递减且落在 (0,1)', (() => {
  const g = live?.gaze;
  if (!g) return false;
  const values = [g.eyeLerp, g.headLerp, g.bodyLerp];
  return values.every((v) => Number.isFinite(v) && v > 0 && v < 1)
    && g.eyeLerp > g.headLerp && g.headLerp > g.bodyLerp;
})(), JSON.stringify(live?.gaze));
check('活感层常量已进生成 TS 与 CSS',
  generated.includes('"liveliness"')
  && css.includes('animation: pet-breathe var(--pet-breath-ms)')
  && css.includes('pet-weight-sway') && css.includes('pet-secondary-sway')
  && css.includes('translate: calc(var(--body-x)'));
check('CSS 含生成段标记',
  css.includes('@generated:pet-assets begin') && css.includes('@generated:pet-assets end'));
check('前端常量含裁剪后的 PET_IMAGE_SIZE',
  generated.includes(`w: ${manifest.baseSize.w}, h: ${manifest.baseSize.h}`));
check('生成 TS 逐帧时长为数组且带 motion / 自动动作常量',
  generated.includes('readonly number[]')
  && generated.includes('export const PET_MOTION')
  && generated.includes('export const PET_AUTO_ACTIONS')
  && generated.includes('export const PET_POSE_PRIORITY'));

const declared = [...generated.matchAll(/^\s{2}'([a-z]+)',$/gm)].map((m) => m[1]);
check('生成 TS 姿势清单与 manifest 一致',
  declared.length === poses.length && declared.every((name, i) => name === poses[i]),
  `TS ${declared.join(',')} / manifest ${poses.join(',')}`);

for (const pose of manifest.poses) {
  const name = pose.name;
  for (const [selector, kind, expected] of [
    [`\\.pet-loop\\[data-loop="${name}"\\]`, '动画层', pose.loop],
    [`\\.pet-poster\\[data-frame="${name}"\\]`, '静态帧', pose.poster],
  ]) {
    const rule = css.match(new RegExp(`${selector}\\s*\\{([^}]*)\\}`));
    const urls = rule
      ? [...rule[1].matchAll(/url\('\.\.\/assets\/pet\/([a-z0-9-]+\.webp)'\)/g)]
        .map((m) => m[1])
      : [];
    check(`${name} ${kind} 恰好一张且文件名匹配`,
      urls.length === 1 && urls[0] === expected,
      `${JSON.stringify(urls)} / 期望 ${expected}`);
  }
  check(`${name} 素材文件落盘`,
    existsSync(join(root, 'src/assets/pet', pose.poster))
    && existsSync(join(root, 'src/assets/pet', pose.loop)));
  const [x0, y0, x1, y1] = pose.hitBox;
  check(`${name} hitBox 合法且人物占比 > 70%（透明点击墙已消除）`,
    [x0, y0, x1, y1].every((v) => Number.isFinite(v) && v >= 0 && v <= 1)
    && x1 > x0 && y1 > y0 && (x1 - x0) > 0.7 && (y1 - y0) > 0.7,
    JSON.stringify(pose.hitBox));
  const ms = pose.frameMs;
  const order = pose.frameOrder;
  const totalMs = Array.isArray(ms) ? ms.reduce((a, b) => a + b, 0) : 0;
  check(`${name} 逐帧时长/帧序/播放模式自洽`,
    Array.isArray(ms) && ms.length === pose.frames
    && ms.every((v) => Number.isInteger(v) && v > 0)
    && Array.isArray(order) && order.length === pose.frames
    && order.every((i) => Number.isInteger(i) && i >= 0 && i < pose.sourceFrames)
    && ['loop', 'pingpong', 'once', 'blink'].includes(pose.playback)
    && totalMs >= 800,
    `${pose.frames} 帧 / 总 ${totalMs}ms`);
  check(`${name} 循环首尾衔接（无 6 帧硬跳）`,
    Math.abs(order[order.length - 1] - order[0]) <= 1,
    JSON.stringify(order));
  check(`${name} hitBox 写入前端常量`,
    generated.includes(`${name}: [${pose.hitBox.join(', ')}]`));
}

for (const extra of manifest.extraFrames || []) {
  check(`附加帧 ${extra} 落盘`, existsSync(join(root, 'src/assets/pet', extra)));
}

if (failed) {
  console.error(`pet-manifest smoke: ${failed} failed`);
  process.exit(1);
}
console.log('pet-manifest smoke: all passed');
