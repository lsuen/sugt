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

const declared = [...generated.matchAll(/^\s{2}'([A-Za-z]+)',$/gm)].map((m) => m[1]);
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
      ? [...rule[1].matchAll(/url\('\.\.\/assets\/pet\/([A-Za-z0-9-]+\.webp)'\)/g)]
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
  // v1.3.2 改为「纯缩放、不裁肢体」后人物在 180×297 画布里占比进一步下降，
  // 再用写死的 >70% / >60% 会把 14 个动作误判为命中框脱节。判据改为：
  // 四边内缩（透明安全边不进命中区）+ 面积不退化；逐动作与素材 50 帧 alpha
  // 并集的精确比对由 SCR-020（Python + Pillow）承担，那里才读得到像素。
  check(`${name} hitBox 合法且四边内缩、面积不退化`,
    [x0, y0, x1, y1].every((v) => Number.isFinite(v) && v >= 0 && v <= 1)
    && x0 > 0.05 && y0 > 0.05 && x1 < 0.95 && y1 < 0.95
    && (x1 - x0) > 0.45 && (y1 - y0) > 0.45,
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
  // v1.3：帧序就是 0..frames-1 的独立位图排列；闭环由 H3 首尾同源关键帧
  // 在像素层面保证，不能用「首尾 index 相邻」来判断（那正是重复帧冒充扩容）。
  check(`${name} 帧序为无重复排列（1000 帧均为独立位图）`,
    order.every((value, i) => value === i)
    && new Set(order).size === pose.frames
    && pose.sourceFrames === pose.frames,
    `${order.length} 帧 / sourceFrames ${pose.sourceFrames}`);
  // 用数值比对而非字符串拼接：生成物按 Python 浮点格式写 0.0 / 1.0，
  // JS `join(', ')` 会写成 0 / 1，字符串断言会把数值一致的常量误判为漂移。
  const hitRow = generated.match(new RegExp(`^\\s*${name}: \\[([^\\]]+)\\]`, 'm'));
  const hitValues = hitRow
    ? hitRow[1].split(',').map((part) => Number(part.trim()))
    : [];
  check(`${name} hitBox 写入前端常量`,
    hitValues.length === 4
    && hitValues.every((value, i) => Number.isFinite(value)
      && Math.abs(value - pose.hitBox[i]) < 1e-6),
    `manifest [${pose.hitBox.join(', ')}] / 生成物 [${hitValues.join(', ')}]`);
}

for (const extra of manifest.extraFrames || []) {
  check(`附加帧 ${extra} 落盘`, existsSync(join(root, 'src/assets/pet', extra)));
}

// v1.3 动作注册表：可扩展、语义唯一、总帧精确 1000。
const registry = manifest.actionRegistry;
check('动作注册表声明帧预算与扩展规则', registry?.version === 1
  && registry.frameBudget.total === 1000
  && registry.frameBudget.uniqueBitmapFrames === true
  && registry.semanticUniqueness === true
  && typeof registry.extension === 'string');
check('20 动作 / 1000 帧 / 语义、素材、优先级、触发词均唯一',
  poses.length === 20
  && manifest.poses.reduce((sum, pose) => sum + pose.frames, 0) === 1000
  && new Set(manifest.poses.map((pose) => pose.semantic)).size === 20
  && new Set(manifest.poses.map((pose) => pose.loop)).size === 20
  && new Set(manifest.poses.map((pose) => pose.poster)).size === 20
  && new Set(manifest.poses.map((pose) => pose.priority)).size === 20);
check('拖拽三段在注册表中各有专属动作，不再复用走路姿',
  ['lifted', 'landing'].every((name) => manifest.poses.some((pose) => pose.name === name
    && pose.group === 'interaction'))
  && manifest.autoActions.drag.pose === 'lifted'
  && manifest.autoActions.drag.releasePose === 'landing'
  && manifest.poses.find((pose) => pose.name === 'walk').semantic.includes('游走'));
check('生成 TS 的 PET_ACTIONS 与 manifest 动作表逐项一致且按 priority 降序',
  generated.includes('export const PET_ACTIONS')
  && manifest.poses.every((pose) => generated.includes(
    `{ name: '${pose.name}', semantic: "${pose.semantic}"`))
  && manifest.posePriority.join(',') === [...manifest.poses]
    .sort((a, b) => b.priority - a.priority)
    .map((pose) => pose.name).join(','));

if (failed) {
  console.error(`pet-manifest smoke: ${failed} failed`);
  process.exit(1);
}
console.log('pet-manifest smoke: all passed');
